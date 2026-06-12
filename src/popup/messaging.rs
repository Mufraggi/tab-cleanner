use oxichrome::runtime;
use serde_wasm_bindgen;
use wasm_bindgen::JsValue;

use crate::ffi::messaging::{MessagingResponse, PopupCommand};

// ── Retry helper for service-worker wake-up ──

/// Maximum number of send attempts (1 initial + 4 retries).
const MAX_RETRY_ATTEMPTS: u32 = 5;

/// Returns `true` when the error is the MV3 "service worker sleeping" error.
fn is_connection_error(e: &oxichrome::OxichromeError) -> bool {
    let s = format!("{:?}", e);
    s.contains("Receiving end does not exist")
        || s.contains("Could not establish connection")
}

/// Async sleep for `ms` milliseconds using `setTimeout`.
async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                &resolve, ms,
            );
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Send a message to the background service worker with automatic retry
/// when the worker is sleeping (MV3).
///
/// On errors matching "Receiving end does not exist" or "Could not establish
/// connection", the function waits with exponential backoff and retries up to
/// `MAX_RETRY_ATTEMPTS` times.  Other errors are returned immediately.
///
/// Backoff delays: 100 ms, 200 ms, 400 ms, 800 ms, 1600 ms.
pub(super) async fn send_message_with_retry(cmd: &PopupCommand) -> Result<JsValue, oxichrome::OxichromeError> {
    let mut attempt: u32 = 0;
    loop {
        match runtime::send_message(cmd).await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if is_connection_error(&e) && attempt < MAX_RETRY_ATTEMPTS {
                    attempt += 1;
                    // Exponential backoff: 100, 200, 400, 800, 1600 ms
                    let delay_ms = 100_i32 * 2_i32.pow(attempt - 1);
                    sleep_ms(delay_ms).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

/// Send RunSemanticGrouping command to the background service worker.
pub async fn trigger_semantic_grouping() -> Result<(), String> {
    let resp_js = send_message_with_retry(&PopupCommand::RunSemanticGrouping)
        .await
        .map_err(|e| format!("Echec d'envoi : {:?}", e))?;

    let resp: MessagingResponse = serde_wasm_bindgen::from_value(resp_js)
        .map_err(|e| format!("Reponse invalide : {:?}", e))?;

    if !resp.success {
        let msg = resp.data.unwrap_or_else(|| "Echec du tri semantique".to_string());
        return Err(msg);
    }
    Ok(())
}

/// Send an UpdateGroup command to the background service worker.
pub async fn send_update_group(
    name: String,
    display_name: Option<String>,
    color: Option<String>,
    theme: Option<String>,
) -> Result<(), String> {
    let cmd = PopupCommand::UpdateGroup {
        name,
        display_name,
        color,
        theme,
    };
    let resp_js = send_message_with_retry(&cmd)
        .await
        .map_err(|e| format!("Echec d'envoi : {:?}", e))?;

    let resp: MessagingResponse = serde_wasm_bindgen::from_value(resp_js)
        .map_err(|e| format!("Reponse invalide : {:?}", e))?;

    if !resp.success {
        let msg = resp.data.unwrap_or_else(|| "Echec de la mise a jour".to_string());
        return Err(msg);
    }
    Ok(())
}

/// Best-effort variant of `send_update_group`.
///
/// Sends an `UpdateGroup` command to the background worker **only** to apply
/// Chrome-native group changes (colour, title). If the worker is sleeping
/// (MV3), the send fails silently — the data is already persisted in storage
/// by the caller. Errors are logged via `oxichrome::log!` but never surfaced
/// to the user.
pub async fn send_update_group_best_effort(
    name: String,
    display_name: Option<String>,
    color: Option<String>,
    theme: Option<String>,
) {
    if let Err(e) = send_update_group(name, display_name, color, theme).await {
        oxichrome::log!(
            "[popup] UpdateGroup best-effort failed (worker sleeping?): {}",
            e
        );
    }
}

/// Best-effort variant to send a `DissolveGroup` command to the background worker.
///
/// If the worker is sleeping (MV3), the send fails silently.
/// Errors are logged via `oxichrome::log!` but never surfaced to the user.
pub async fn send_dissolve_group_best_effort(name: String) {
    let cmd = PopupCommand::DissolveGroup { name };
    match send_message_with_retry(&cmd).await {
        Ok(resp_js) => {
            let _resp: MessagingResponse = serde_wasm_bindgen::from_value(resp_js).unwrap_or_else(|e| {
                oxichrome::log!(
                    "[popup] DissolveGroup best-effort response parse error: {:?}",
                    e
                );
                MessagingResponse {
                    success: false,
                    data: Some(format!("Parse error: {:?}", e)),
                }
            });
        }
        Err(e) => {
            oxichrome::log!(
                "[popup] DissolveGroup best-effort failed (worker sleeping?): {:?}",
                e
            );
        }
    }
}
