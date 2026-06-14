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
    /// Create a new empty manual group.
    CreateGroup {
        name: String,
        /// Theme description for SML-based grouping.
        #[serde(skip_serializing_if = "Option::is_none")]
        theme: Option<String>,
    },
    /// Dissolve an existing group (ungroup its tabs, keep the group entry).
    DissolveGroup {
        name: String,
    },
    /// Run semantic grouping: SML first, then heuristic fallback.
    RunSemanticGrouping,
    /// Download model weights + tokenizer from HuggingFace CDN into Cache API.
    DownloadModel,
    /// Check whether model + tokenizer are already cached.
    CheckModelCached,
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
            .ok_or_else(|| format!("Group '{}' not found", name))?;

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
                .map_err(|e| format!("Chrome update failed: {:?}", e))?;
        }
    }

    // 5. Persist updated state
    crate::storage::save_state(&state).await;

    Ok(())
}

/// Pure logic: add a new manual group to the state.
///
/// Returns `true` if the group was added, `false` if it already existed (idempotent).
pub fn apply_create_group(state: &mut crate::types::GroupState, name: &str, theme: Option<&str>, now_ms: f64) -> bool {
    if state.groups.iter().any(|g| g.name == name) {
        return false; // already exists — idempotent
    }
    state.groups.push(crate::types::StoredGroup::new_manual(
        name.to_string(),
        theme.unwrap_or("").to_string(),
        now_ms,
    ));
    true
}

/// Async handler: create a new manual group.
async fn handle_create_group(name: String, theme: Option<String>) -> Result<(), String> {
    let mut state = crate::storage::load_state().await;
    let _ = apply_create_group(&mut state, &name, theme.as_deref(), js_sys::Date::now());
    crate::storage::save_state(&state).await;
    Ok(())
}

/// Pure logic: dissolve a group by clearing its `group_id`.
///
/// Returns `Ok(())` if the group was found (even if `group_id` was already `None`),
/// or `Err(...)` if the group doesn't exist.
pub fn apply_dissolve_group(state: &mut crate::types::GroupState, name: &str) -> Result<(), String> {
    let group = state
        .groups
        .iter_mut()
        .find(|g| g.name == name)
        .ok_or_else(|| format!("Groupe '{}' introuvable", name))?;
    group.group_id = None;
    Ok(())
}

/// Async handler: dissolve a group (ungroup its Chrome tabs, clear its group_id).
async fn handle_dissolve_group(name: String) -> Result<(), String> {
    // 1. Load state and find group
    let mut state = crate::storage::load_state().await;
    let chrome_group_id: Option<i32>;

    {
        let group = state
            .groups
            .iter()
            .find(|g| g.name == name)
            .ok_or_else(|| format!("Group '{}' not found", name))?;
        chrome_group_id = group.group_id;
    }

    // 2. If the group has a Chrome group id, ungroup its tabs
    if let Some(gid) = chrome_group_id {
        let tabs: Vec<crate::types::TabInfo> = oxichrome::tabs::query(
            &crate::types::QueryByGroupId { group_id: gid },
        )
        .await
        .map_err(|e| format!("Error reading tabs: {:?}", e))?;

        let tab_ids: Vec<i32> = tabs.iter().map(|t| t.id).collect();
        if !tab_ids.is_empty() {
            crate::ffi::tabs_ext::ungroup_tabs(&tab_ids)
                .await
                .map_err(|e| format!("Tab ungrouping failed: {:?}", e))?;
        }
    }

    // 3. Clear group_id in state
    apply_dissolve_group(&mut state, &name)?;

    // 4. Save
    crate::storage::save_state(&state).await;

    Ok(())
}

/// Download the model.safetensors and tokenizer.json from HuggingFace CDN into the Cache API.
///
/// Returns `Ok("downloaded")` when both files are cached (either from cache or network).
async fn handle_download_model() -> Result<String, String> {
    let src1 = crate::sml::ensure_model_cached(crate::types::MODEL_URL).await
        .map_err(|e| format!("Model download failed: {}", e))?;
    let src2 = crate::sml::ensure_model_cached(crate::types::TOKENIZER_URL).await
        .map_err(|e| format!("Tokenizer download failed: {}", e))?;
    oxichrome::log!(
        "[messaging] Model cache: {} (model), {} (tokenizer)",
        src1,
        src2
    );
    Ok("downloaded".to_string())
}

/// Check if both model.safetensors and tokenizer.json are cached.
///
/// This is a lightweight operation — it only performs `cache.match()`, not actual loading.
async fn is_model_cached() -> bool {
    let model_ok = crate::sml::model_cache::is_url_cached(crate::types::MODEL_URL).await;
    let tokenizer_ok = crate::sml::model_cache::is_url_cached(crate::types::TOKENIZER_URL).await;
    model_ok && tokenizer_ok
}

/// Helper: serialise a handler result into a `MessagingResponse` and send it back
/// to the popup via `send_fn.call1`.
///
/// - `Ok(data)` → `MessagingResponse { success: true, data }`
/// - `Err(e)`  → `MessagingResponse { success: false, data: Some(e) }`
///
/// Serialisation errors are logged but never panic the service worker.
fn reply(send_fn: &js_sys::Function, result: Result<Option<String>, String>) {
    let (success, data) = match result {
        Ok(data) => (true, data),
        Err(e) => (false, Some(e)),
    };
    match serde_wasm_bindgen::to_value(&MessagingResponse { success, data }) {
        Ok(val) => {
            let _ = send_fn.call1(&JsValue::NULL, &val);
        }
        Err(e) => {
            oxichrome::log!(
                "[messaging] Response serialisation error: {:?}",
                e
            );
        }
    }
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

                    reply(&send_fn, Ok(state_json));
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
                    let result = handle_update_group(name, display_name, color, theme)
                        .await
                        .map(|()| None);
                    reply(&send_fn, result);
                });
            }

            Ok(PopupCommand::CreateGroup { name, theme }) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = handle_create_group(name, theme)
                        .await
                        .map(|()| None);
                    reply(&send_fn, result);
                });
            }

            Ok(PopupCommand::DissolveGroup { name }) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = handle_dissolve_group(name)
                        .await
                        .map(|()| None);
                    reply(&send_fn, result);
                });
            }

            Ok(PopupCommand::RunSemanticGrouping) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = crate::semantic::run_semantic_grouping()
                        .await
                        .map(|_| None);
                    reply(&send_fn, result);
                });
            }

            Ok(PopupCommand::DownloadModel) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = handle_download_model().await.map(Some);
                    reply(&send_fn, result);
                });
            }

            Ok(PopupCommand::CheckModelCached) => {
                let send_fn = send_fn.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let cached = is_model_cached().await;
                    // Backward-compatible: encode bool as "true"/"false" string.
                    // The popup side (check_model_cached) expects these exact strings.
                    let data = Some(if cached { "true" } else { "false" }.to_string());
                    reply(&send_fn, Ok(data));
                });
            }

            Err(e) => {
                oxichrome::log!(
                    "[messaging] Failed to parse PopupCommand: {:?}",
                    e
                );
                reply(&send_fn, Err(format!("Parse error: {:?}", e)));
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
    fn test_popup_command_create_group_serialization() {
        let cmd = PopupCommand::CreateGroup {
            name: "My Group".into(),
            theme: Some("coding projects".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"createGroup","name":"My Group","theme":"coding projects"}"#);
    }

    #[test]
    fn test_popup_command_dissolve_group_serialization() {
        let cmd = PopupCommand::DissolveGroup {
            name: "github.com".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"dissolveGroup","name":"github.com"}"#);
    }

    #[test]
    fn test_popup_command_create_group_roundtrip() {
        let cmd = PopupCommand::CreateGroup {
            name: "My Group".into(),
            theme: Some("coding projects".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: PopupCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            PopupCommand::CreateGroup { name, theme } => {
                assert_eq!(name, "My Group");
                assert_eq!(theme, Some("coding projects".to_string()));
            }
            _ => panic!("Expected CreateGroup"),
        }
    }

    #[test]
    fn test_popup_command_dissolve_group_roundtrip() {
        let cmd = PopupCommand::DissolveGroup {
            name: "github.com".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: PopupCommand = serde_json::from_str(&json).unwrap();
        match parsed {
            PopupCommand::DissolveGroup { name } => {
                assert_eq!(name, "github.com");
            }
            _ => panic!("Expected DissolveGroup"),
        }
    }

    #[test]
    fn test_popup_command_deserialize_unit_variants() {
        let cmd: PopupCommand = serde_json::from_str(r#"{"type":"getState"}"#).unwrap();
        assert!(matches!(cmd, PopupCommand::GetState));
    }

    #[test]
    fn test_popup_command_run_semantic_grouping_serialization() {
        let cmd = PopupCommand::RunSemanticGrouping;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"runSemanticGrouping"}"#);
    }

    #[test]
    fn test_popup_command_download_model_serialization() {
        let cmd = PopupCommand::DownloadModel;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"downloadModel"}"#);
    }

    #[test]
    fn test_popup_command_check_model_cached_serialization() {
        let cmd = PopupCommand::CheckModelCached;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"type":"checkModelCached"}"#);
    }

    #[test]
    fn test_popup_command_deserialize_new_unit_variants() {
        let cmd: PopupCommand = serde_json::from_str(r#"{"type":"runSemanticGrouping"}"#).unwrap();
        assert!(matches!(cmd, PopupCommand::RunSemanticGrouping));
        let cmd: PopupCommand = serde_json::from_str(r#"{"type":"downloadModel"}"#).unwrap();
        assert!(matches!(cmd, PopupCommand::DownloadModel));
        let cmd: PopupCommand = serde_json::from_str(r#"{"type":"checkModelCached"}"#).unwrap();
        assert!(matches!(cmd, PopupCommand::CheckModelCached));
    }

    #[test]
    fn test_popup_command_roundtrip_run_semantic_grouping() {
        let cmd = PopupCommand::RunSemanticGrouping;
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: PopupCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, PopupCommand::RunSemanticGrouping));
    }

    // ── apply_create_group tests ──

    #[test]
    fn test_apply_create_group_new() {
        let mut state = crate::types::GroupState {
            version: 1,
            groups: vec![],
        };
        let added = apply_create_group(&mut state, "github.com", Some("coding"), 1000.0);
        assert!(added, "new group must return true");
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].name, "github.com");
        assert_eq!(state.groups[0].theme, "coding");
        assert_eq!(state.groups[0].manual, true);
        assert_eq!(state.groups[0].group_id, None);
        assert_eq!(
            state.groups[0].display_name,
            Some("github.com".to_string())
        );
        assert_eq!(state.groups[0].created_at_ms, 1000.0);
        assert_eq!(state.groups[0].updated_at_ms, 1000.0);
    }

    #[test]
    fn test_apply_create_group_idempotent() {
        let mut state = crate::types::GroupState {
            version: 1,
            groups: vec![crate::types::StoredGroup {
                name: "github.com".into(),
                keywords: vec![],
                created_at_ms: 500.0,
                updated_at_ms: 500.0,
                group_id: Some(42),
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let added = apply_create_group(&mut state, "github.com", None, 2000.0);
        assert!(!added, "duplicate group must return false");
        assert_eq!(state.groups.len(), 1, "no duplicate added");
        // Existing group unchanged
        assert_eq!(state.groups[0].group_id, Some(42));
        assert_eq!(state.groups[0].created_at_ms, 500.0);
    }

    #[test]
    fn test_apply_create_group_with_existing_other_groups() {
        let mut state = crate::types::GroupState {
            version: 1,
            groups: vec![crate::types::StoredGroup {
                name: "docs.rs".into(),
                keywords: vec![],
                created_at_ms: 500.0,
                updated_at_ms: 500.0,
                group_id: Some(10),
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let added = apply_create_group(&mut state, "github.com", None, 2000.0);
        assert!(added);
        assert_eq!(state.groups.len(), 2);
        assert_eq!(state.groups[1].name, "github.com");
        assert_eq!(state.groups[1].manual, true);
    }

    // ── apply_dissolve_group tests ──

    #[test]
    fn test_apply_dissolve_group_clears_group_id() {
        let mut state = crate::types::GroupState {
            version: 1,
            groups: vec![crate::types::StoredGroup {
                name: "github.com".into(),
                keywords: vec![],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: Some(42),
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let result = apply_dissolve_group(&mut state, "github.com");
        assert!(result.is_ok());
        assert_eq!(state.groups[0].group_id, None);
    }

    #[test]
    fn test_apply_dissolve_group_already_none() {
        let mut state = crate::types::GroupState {
            version: 1,
            groups: vec![crate::types::StoredGroup {
                name: "github.com".into(),
                keywords: vec![],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: None,
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let result = apply_dissolve_group(&mut state, "github.com");
        assert!(result.is_ok(), "dissolving a group with group_id None must succeed");
        assert_eq!(state.groups[0].group_id, None);
    }

    #[test]
    fn test_apply_dissolve_group_not_found() {
        let mut state = crate::types::GroupState {
            version: 1,
            groups: vec![],
        };
        let result = apply_dissolve_group(&mut state, "nonexistent");
        assert!(result.is_err(), "dissolving a non-existent group must return error");
    }
}
