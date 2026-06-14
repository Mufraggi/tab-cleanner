use oxichrome::{storage, tabs};

use crate::ffi::messaging::{MessagingResponse};
use crate::grouping::apply::pick_color;
use crate::types::{GroupState, GROUP_STATE_KEY, QueryAllTabs, TabInfo};

use super::lookup_hex;
use super::messaging::send_message_with_retry;

// ── Combined data model ──

#[derive(Clone)]
pub struct GroupDisplay {
    pub name: String,
    pub display_name: String,
    pub color_name: String,
    pub color_hex: String,
    pub theme: String,
    pub tabs: Vec<TabInfo>,
    /// The Chrome tab group id, if the group has been materialised in the browser.
    /// `None` for groups that haven't been created yet or have been dissolved.
    pub group_id: Option<i32>,
}

#[derive(Clone)]
pub struct PopupData {
    pub groups: Vec<GroupDisplay>,
    pub ungrouped: Vec<TabInfo>,
    pub total_groups: usize,
    pub total_tabs: usize,
}

// ── Async fetch ──

pub async fn fetch_popup_data() -> Result<PopupData, String> {
    // 1. Read group state directly from storage (no message to background worker)
    //    Chrome storage.local is accessible to popup context without waking the SW.
    let state: GroupState = storage::get::<GroupState>(GROUP_STATE_KEY)
        .await
        .map_err(|e| format!("Error reading storage: {:?}", e))?
        .unwrap_or_else(|| GroupState {
            version: 1,
            groups: vec![],
        });

    // 2. Query current tabs
    let tabs: Vec<TabInfo> = tabs::query(&QueryAllTabs {
        current_window: Some(true),
    })
    .await
    .map_err(|e| format!("Failed to read tabs: {:?}", e))?;

    // 3. Build group_id -> StoredGroup map
    let id_to_group: std::collections::HashMap<i32, &crate::types::StoredGroup> = state
        .groups
        .iter()
        .filter_map(|g| g.group_id.map(|id| (id, g)))
        .collect();

    // 4. Classify tabs
    let mut group_tabs: std::collections::HashMap<String, Vec<TabInfo>> =
        std::collections::HashMap::new();
    let mut ungrouped: Vec<TabInfo> = Vec::new();

    for tab in tabs {
        if tab.group_id > 0 {
            if let Some(group) = id_to_group.get(&tab.group_id) {
                group_tabs.entry(group.name.clone()).or_default().push(tab);
            } else {
                ungrouped.push(tab);
            }
        } else {
            ungrouped.push(tab);
        }
    }

    // 5. Build display list preserving stored group order,
    //    filtering out groups that have NO open tabs (orphaned groups).
    let mut groups: Vec<GroupDisplay> = Vec::with_capacity(state.groups.len());
    for stored in &state.groups {
        let tabs_in_group = group_tabs.remove(&stored.name).unwrap_or_default();
        if tabs_in_group.is_empty() && !stored.manual {
            continue; // skip orphaned non-manual groups — they stay in storage but are hidden
        }
        let cname = stored
            .color
            .clone()
            .unwrap_or_else(|| pick_color(&stored.name).to_string());
        groups.push(GroupDisplay {
            name: stored.name.clone(),
            display_name: stored
                .display_name
                .clone()
                .unwrap_or_else(|| stored.name.clone()),
            color_name: cname.clone(),
            color_hex: lookup_hex(&cname).to_string(),
            theme: stored.theme.clone(),
            tabs: tabs_in_group,
            group_id: stored.group_id,
        });
    }

    let total_groups = groups.len();
    let total_tabs: usize = groups.iter().map(|g| g.tabs.len()).sum::<usize>() + ungrouped.len();

    Ok(PopupData {
        groups,
        ungrouped,
        total_groups,
        total_tabs,
    })
}

/// Check whether the model is cached by querying the background worker.
pub async fn check_model_cached() -> Result<bool, String> {
    let resp_js = send_message_with_retry(&crate::ffi::messaging::PopupCommand::CheckModelCached)
        .await
        .map_err(|e| format!("Send failed: {:?}", e))?;

    let resp: MessagingResponse = serde_wasm_bindgen::from_value(resp_js)
        .map_err(|e| format!("Invalid response: {:?}", e))?;

    if !resp.success {
        let msg = resp.data.unwrap_or_else(|| "Verification failed".to_string());
        return Err(msg);
    }

    match resp.data.as_deref() {
        Some("true") => Ok(true),
        _ => Ok(false),
    }
}
