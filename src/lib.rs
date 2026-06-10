use oxichrome::prelude::*;
use std::collections::HashSet;

mod ffi;
mod grouping;
mod storage;
mod types;

use crate::types::{GroupAssignment, QueryAllTabs, TabInfo};

#[oxichrome::extension(
    name = "Tab Cleanner",
    version = "0.1.0",
    permissions = ["storage", "tabs", "tabGroups"]
)]
struct Extension;

#[oxichrome::background]
async fn start() {
    oxichrome::log!("Tab Cleanner started!");
    let _ = run_grouping().await;
}

#[oxichrome::on(runtime::on_installed)]
async fn handle_install(details: oxichrome::__private::wasm_bindgen::JsValue) {
    oxichrome::log!("Tab Cleanner installed: {:?}", details);
}

/// Run grouping and persist the resulting state to storage.
/// Idempotent: re-running with the same tabs reuses existing groups.
pub async fn run_grouping() -> Vec<GroupAssignment> {
    // 1. Query all current tabs (current window only)
    let tabs: Vec<TabInfo> = tabs::query(&QueryAllTabs {
            current_window: Some(true),
        })
        .await
        .unwrap_or_default();

    // 2. Classify tabs (pure, deterministic)
    let assignments = grouping::group_tabs(tabs);

    // 3. Load persisted state (empty on first run)
    let stored = crate::storage::load_state().await;

    // 4. Reconcile fresh assignments with stored state
    let now_ms = js_sys::Date::now();
    let updated = grouping::reconcile(&assignments, &stored, now_ms);

    // 5. Apply Chrome tab groups (create/update/ungroup)
    let final_state = grouping::apply::apply_groups(&assignments, &updated).await;

    // 6. Save updated state (fire-and-forget)
    crate::storage::save_state(&final_state).await;

    // 7. Log summary
    let group_set: HashSet<&str> = assignments
        .iter()
        .map(|a| a.group_name.as_str())
        .collect();
    oxichrome::log!(
        "Grouping complete: {} tabs → {} groups ({} persisted total)",
        assignments.len(),
        group_set.len(),
        final_state.groups.len()
    );

    assignments
}
