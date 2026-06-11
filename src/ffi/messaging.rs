use js_sys::JSON;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// Raw FFI binding for `chrome.runtime.onMessage.addListener`.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "runtime", "onMessage"], js_name = addListener)]
    fn add_on_message_listener(callback: &JsValue);
}

/// Commands the popup can send to the background service worker.
///
/// Each variant is dispatched via `serde_wasm_bindgen` using the `"type"` tag
/// (e.g. `{"type": "runGrouping"}` or `{"type": "getState"}`).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PopupCommand {
    /// Run the full grouping pipeline and persist state.
    RunGrouping,
    /// Retrieve the persisted group state.
    GetState,
    /// Update properties of a persisted group (name, display_name, color, theme).
    UpdateGroup {
        /// Group identifier — domain name, e.g. "github.com"
        name: String,
        /// Optional new display name. None = unchanged.
        #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
        display_name: Option<String>,
        /// Optional new colour (one of the 8 Chrome colours). None = unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        /// Optional new theme description. None = unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        theme: Option<String>,
    },
}

/// Response sent back to the popup after handling a command.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagingResponse {
    /// Whether the command was handled successfully.
    pub success: bool,
    /// Optional payload (e.g. JSON-serialised state, error message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// Handle an `UpdateGroup` command from the popup.
///
/// Loads persisted state, applies field mutations, updates the real Chrome tab group
/// (if one exists and colour or title changed), and persists the updated state.
///
/// All errors are returned as `Err(String)` — the caller serialises them into a
/// `MessagingResponse`.
async fn handle_update_group(
    name: String,
    display_name: Option<String>,
    color: Option<String>,
    theme: Option<String>,
) -> Result<(), String> {
    // 1. Load persisted state
    let mut state = crate::storage::load_state().await;

    // 2. Find the StoredGroup by name
    let chrome_group_id: Option<i32>;
    let chrome_color: Option<String>;
    let chrome_title: Option<String>;
    // Decide whether to push Chrome updates based on original parameters
    // (before they may be partially moved into the group)
    let has_new_display_name = display_name.is_some();
    let has_new_color = color.is_some();

    {
        let group = state
            .groups
            .iter_mut()
            .find(|g| g.name == name)
            .ok_or_else(|| format!("Groupe '{}' introuvable", name))?;

        // 3. Apply field updates
        if let Some(dn) = display_name {
            group.display_name = Some(dn);
        }
        if let Some(c) = color {
            group.color = Some(c);
        }
        if let Some(t) = theme {
            group.theme = t;
        }

        // Capture values for Chrome update (after mutable borrow ends)
        chrome_group_id = group.group_id;
        chrome_color = group.color.clone();
        chrome_title = group.display_name.clone();
    }

    // 4. If the group has a Chrome group_id AND we changed colour or display_name,
    //    update the real Chrome group via FFI
    if let Some(gid) = chrome_group_id {
        if has_new_display_name || has_new_color {
            let props = crate::ffi::tab_groups::UpdateProperties {
                color: chrome_color,
                title: chrome_title,
            };
            crate::ffi::tab_groups::update_tab_group(gid, &props)
                .await
                .map_err(|e| format!("Echec de mise a jour Chrome : {:?}", e))?;
        }
    }

    // 5. Persist updated state
    crate::storage::save_state(&state).await;

    Ok(())
}

/// Register the `chrome.runtime.onMessage` listener for popup ↔ background communication.
///
/// Must be called once during background startup (from `start()`).
///
/// # Behaviour
/// - Deserialises `PopupCommand` from the incoming message
/// - Spawns an async task for the command
/// - Calls `sendResponse` with a serialised `MessagingResponse`
/// - Returns `true` (`JsValue::from_bool(true)`) to keep the MV3 channel open
///
/// # Safety
/// - No global `Mutex<Option<js_sys::Function>>` — `send_fn` is cloned locally before each
///   `spawn_local`, avoiding race conditions when two messages arrive in quick succession.
/// - No `unwrap()` inside the closure — all `serde_wasm_bindgen` calls are handled as `Result`;
///   errors are logged via `oxichrome::log!` and never panic the service worker.
/// - `closure.forget()` prevents garbage collection of the listener closure.
pub fn register_message_listener() {
    let closure = Closure::wrap(Box::new(move |message: JsValue, _sender: JsValue, send_response: JsValue| {
        // Convert send_response to a callable function.
        // Cloned before each spawn_local so no shared mutable state is needed.
        let send_fn: js_sys::Function = send_response.into();

        match serde_wasm_bindgen::from_value::<PopupCommand>(message) {
            Ok(PopupCommand::RunGrouping) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let _ = crate::run_grouping().await;

                    match serde_wasm_bindgen::to_value(&MessagingResponse {
                        success: true,
                        data: None,
                    }) {
                        Ok(val) => {
                            let _ = send_fn.call1(&JsValue::NULL, &val);
                        }
                        Err(e) => {
                            oxichrome::log!(
                                "[messaging] RunGrouping response serialisation error: {:?}",
                                e
                            );
                        }
                    }
                });
            }

            Ok(PopupCommand::GetState) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let state = crate::storage::load_state().await;

                    // Serialise GroupState to a JSON string via js_sys::JSON::stringify
                    // to avoid pulling serde_json into the main dependency graph.
                    let state_json = serde_wasm_bindgen::to_value(&state)
                        .ok()
                        .and_then(|val| JSON::stringify(&val).ok())
                        .and_then(|s| s.as_string());

                    match serde_wasm_bindgen::to_value(&MessagingResponse {
                        success: true,
                        data: state_json,
                    }) {
                        Ok(val) => {
                            let _ = send_fn.call1(&JsValue::NULL, &val);
                        }
                        Err(e) => {
                            oxichrome::log!(
                                "[messaging] GetState response serialisation error: {:?}",
                                e
                            );
                        }
                    }
                });
            }

            Ok(PopupCommand::UpdateGroup {
                name,
                display_name,
                color,
                theme,
            }) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match handle_update_group(name, display_name, color, theme).await {
                        Ok(()) => {
                            match serde_wasm_bindgen::to_value(&MessagingResponse {
                                success: true,
                                data: None,
                            }) {
                                Ok(val) => {
                                    let _ = send_fn.call1(&JsValue::NULL, &val);
                                }
                                Err(e) => {
                                    oxichrome::log!(
                                        "[messaging] UpdateGroup response serialisation error: {:?}",
                                        e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            match serde_wasm_bindgen::to_value(&MessagingResponse {
                                success: false,
                                data: Some(e),
                            }) {
                                Ok(val) => {
                                    let _ = send_fn.call1(&JsValue::NULL, &val);
                                }
                                Err(e2) => {
                                    oxichrome::log!(
                                        "[messaging] UpdateGroup error response serialisation error: {:?}",
                                        e2
                                    );
                                }
                            }
                        }
                    }
                });
            }

            Err(e) => {
                oxichrome::log!(
                    "[messaging] Failed to parse PopupCommand: {:?}",
                    e
                );

                match serde_wasm_bindgen::to_value(&MessagingResponse {
                    success: false,
                    data: Some(format!("Parse error: {:?}", e)),
                }) {
                    Ok(val) => {
                        let _ = send_fn.call1(&JsValue::NULL, &val);
                    }
                    Err(e2) => {
                        oxichrome::log!(
                            "[messaging] Failed to serialise error response: {:?}",
                            e2
                        );
                    }
                }
            }
        }

        // MV3: return true to signal that sendResponse will be called asynchronously.
        JsValue::from_bool(true)
    }) as Box<dyn FnMut(JsValue, JsValue, JsValue) -> JsValue>);

    add_on_message_listener(closure.as_ref());
    closure.forget();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_popup_command_run_grouping_serialization() {
        let cmd = PopupCommand::RunGrouping;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"runGrouping"}"#);
    }

    #[test]
    fn test_popup_command_get_state_serialization() {
        let cmd = PopupCommand::GetState;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"getState"}"#);
    }

    #[test]
    fn test_popup_command_update_group_all_fields() {
        let cmd = PopupCommand::UpdateGroup {
            name: "github.com".into(),
            display_name: Some("GitHub".into()),
            color: Some("blue".into()),
            theme: Some("coding".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let expected = r#"{"type":"updateGroup","name":"github.com","displayName":"GitHub","color":"blue","theme":"coding"}"#;
        assert_eq!(json, expected);
    }

    #[test]
    fn test_popup_command_update_group_none_fields_omitted() {
        let cmd = PopupCommand::UpdateGroup {
            name: "youtube.com".into(),
            display_name: None,
            color: None,
            theme: Some("videos".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        // displayName et color absents
        assert!(json.contains("\"type\":\"updateGroup\""));
        assert!(json.contains("\"name\":\"youtube.com\""));
        assert!(json.contains("\"theme\":\"videos\""));
        assert!(!json.contains("displayName"));
        assert!(!json.contains("\"color\""));
    }

    #[test]
    fn test_popup_command_roundtrip_update_group() {
        let cmd = PopupCommand::UpdateGroup {
            name: "docs.rs".into(),
            display_name: Some("Docs".into()),
            color: None,
            theme: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: PopupCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            PopupCommand::UpdateGroup {
                name,
                display_name,
                color,
                theme,
            } => {
                assert_eq!(name, "docs.rs");
                assert_eq!(display_name, Some("Docs".into()));
                assert_eq!(color, None);
                assert_eq!(theme, None);
            }
            _ => panic!("Expected UpdateGroup"),
        }
    }

    #[test]
    fn test_popup_command_deserialize_unit_variants() {
        let cmd: PopupCommand = serde_json::from_str(r#"{"type":"runGrouping"}"#).unwrap();
        assert!(matches!(cmd, PopupCommand::RunGrouping));
        let cmd: PopupCommand = serde_json::from_str(r#"{"type":"getState"}"#).unwrap();
        assert!(matches!(cmd, PopupCommand::GetState));
    }
}
