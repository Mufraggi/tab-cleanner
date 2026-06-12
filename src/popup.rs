use leptos::prelude::*;
use oxichrome::{runtime, storage, tabs};
use serde_wasm_bindgen;
use wasm_bindgen::JsValue;

use crate::ffi::messaging::{MessagingResponse, PopupCommand};
use crate::grouping::apply::pick_color;
use crate::types::{GroupState, GROUP_STATE_KEY, ONBOARDING_DONE_KEY, QueryAllTabs, TabInfo};

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
async fn send_message_with_retry(cmd: &PopupCommand) -> Result<JsValue, oxichrome::OxichromeError> {
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

// ── Chrome tab group colour palette (hex) ──
pub const HEX: &[(&str, &str)] = &[
    ("blue", "#5b8def"),
    ("red", "#e05a52"),
    ("yellow", "#e3b341"),
    ("green", "#4f9d69"),
    ("pink", "#d96aa8"),
    ("purple", "#9b6dd6"),
    ("cyan", "#46a7b8"),
    ("orange", "#e08a3c"),
];

pub fn lookup_hex(name: &str) -> &'static str {
    HEX.iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| *v)
        .unwrap_or("#8a8f98")
}

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

// ── Palette of colours (excluding grey) ──
pub const PALETTE: &[&str] = &[
    "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange",
];

// ── Async fetch ──

pub async fn fetch_popup_data() -> Result<PopupData, String> {
    // 1. Read group state directly from storage (no message to background worker)
    //    Chrome storage.local is accessible to popup context without waking the SW.
    let state: GroupState = storage::get::<GroupState>(GROUP_STATE_KEY)
        .await
        .map_err(|e| format!("Erreur de lecture du storage : {:?}", e))?
        .unwrap_or_else(|| GroupState {
            version: 1,
            groups: vec![],
        });

    // 2. Query current tabs
    let tabs: Vec<TabInfo> = tabs::query(&QueryAllTabs {
        current_window: Some(true),
    })
    .await
    .map_err(|e| format!("Impossible de lire les onglets : {:?}", e))?;

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

/// Check whether the model is cached by querying the background worker.
pub async fn check_model_cached() -> Result<bool, String> {
    let resp_js = send_message_with_retry(&PopupCommand::CheckModelCached)
        .await
        .map_err(|e| format!("Echec d'envoi : {:?}", e))?;

    let resp: MessagingResponse = serde_wasm_bindgen::from_value(resp_js)
        .map_err(|e| format!("Reponse invalide : {:?}", e))?;

    if !resp.success {
        let msg = resp.data.unwrap_or_else(|| "Echec de la verification".to_string());
        return Err(msg);
    }

    match resp.data.as_deref() {
        Some("true") => Ok(true),
        _ => Ok(false),
    }
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

/// Persist group fields directly to storage (bypasses background worker).
///
/// Reads `GroupState` from storage, finds the `StoredGroup` by `name`,
/// applies the specified field updates, and writes back.
/// Returns an error if the group is not found or storage operations fail.
pub async fn persist_group_fields(
    name: &str,
    display_name: Option<&str>,
    color: Option<&str>,
    theme: Option<&str>,
) -> Result<(), String> {
    let mut state: GroupState = storage::get::<GroupState>(GROUP_STATE_KEY)
        .await
        .map_err(|e| format!("Erreur de lecture du storage : {:?}", e))?
        .unwrap_or_else(|| GroupState {
            version: 1,
            groups: vec![],
        });

    let group = state
        .groups
        .iter_mut()
        .find(|g| g.name == name)
        .ok_or_else(|| format!("Groupe '{}' introuvable", name))?;

    if let Some(dn) = display_name {
        group.display_name = Some(dn.to_string());
    }
    if let Some(c) = color {
        group.color = Some(c.to_string());
    }
    if let Some(t) = theme {
        group.theme = t.to_string();
    }

    storage::set(GROUP_STATE_KEY, &state)
        .await
        .map_err(|e| format!("Erreur d'écriture du storage : {:?}", e))?;

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

/// Create a new manual group directly in storage.
///
/// Reads `GroupState` from storage, checks for duplicates (idempotent),
/// appends a new `StoredGroup` with `manual: true` and the provided `theme`, and writes back.
/// Returns an error if storage operations fail.
pub async fn persist_create_group(name: &str, theme: &str) -> Result<(), String> {
    let mut state: GroupState = storage::get::<GroupState>(GROUP_STATE_KEY)
        .await
        .map_err(|e| format!("Erreur de lecture du storage : {:?}", e))?
        .unwrap_or_else(|| GroupState {
            version: 1,
            groups: vec![],
        });

    // Idempotent: skip if group already exists
    if state.groups.iter().any(|g| g.name == name) {
        return Ok(());
    }

    let now = js_sys::Date::now();
    state.groups.push(crate::types::StoredGroup {
        name: name.to_string(),
        keywords: vec![],
        created_at_ms: now,
        updated_at_ms: now,
        group_id: None,
        display_name: Some(name.to_string()),
        theme: theme.to_string(),
        color: None,
        manual: true,
    });

    storage::set(GROUP_STATE_KEY, &state)
        .await
        .map_err(|e| format!("Erreur d'écriture du storage : {:?}", e))?;

    Ok(())
}

/// Check whether the onboarding has been completed.
/// Returns `false` if the key is absent or deserialisation fails.
pub async fn is_onboarding_done() -> bool {
    storage::get::<bool>(ONBOARDING_DONE_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or(false)
}

/// Persist the onboarding-done marker.
/// Fire-and-forget on error; errors are logged via `oxichrome::log!`.
pub async fn set_onboarding_done() {
    if let Err(e) = storage::set(ONBOARDING_DONE_KEY, &true).await {
        oxichrome::log!("[popup] set_onboarding_done failed: {:?}", e);
    }
}

/// Batch-create manual groups from onboarding theme selections.
///
/// Reads `GroupState` once, appends new groups for each `(name, theme)` pair
/// that does not already exist, and writes `GroupState` back in a single operation.
/// This avoids N storage writes that `persist_create_group` would perform for N groups.
///
/// **Note**: if the `StoredGroup` struct gains new fields, both this function
/// and `persist_create_group` must be updated.
pub async fn persist_create_groups_batch(names_and_themes: &[(String, String)]) -> Result<(), String> {
    let mut state: GroupState = storage::get::<GroupState>(GROUP_STATE_KEY)
        .await
        .map_err(|e| format!("Erreur de lecture du storage : {:?}", e))?
        .unwrap_or_else(|| GroupState {
            version: 1,
            groups: vec![],
        });

    let mut added = 0usize;
    let now = js_sys::Date::now();
    for (name, theme) in names_and_themes {
        // Idempotent: skip if group already exists
        if state.groups.iter().any(|g| g.name == *name) {
            continue;
        }
        state.groups.push(crate::types::StoredGroup {
            name: name.to_string(),
            keywords: vec![],
            created_at_ms: now,
            updated_at_ms: now,
            group_id: None,
            display_name: Some(name.to_string()),
            theme: theme.to_string(),
            color: None,
            manual: true,
        });
        added += 1;
    }

    if added > 0 {
        storage::set(GROUP_STATE_KEY, &state)
            .await
            .map_err(|e| format!("Erreur d'écriture du storage : {:?}", e))?;
    }

    Ok(())
}

/// Pure merge helper: applies `names_and_themes` to a `GroupState` in-memory.
/// Returns the number of groups that were actually added (skipping duplicates).
///
/// This is extracted for testability so that the merge logic can be verified
/// without mocking Chrome storage.
pub fn apply_create_groups_batch(
    state: &mut GroupState,
    names_and_themes: &[(String, String)],
    now_ms: f64,
) -> usize {
    let mut added = 0usize;
    for (name, theme) in names_and_themes {
        if state.groups.iter().any(|g| g.name == *name) {
            continue;
        }
        state.groups.push(crate::types::StoredGroup {
            name: name.to_string(),
            keywords: vec![],
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            group_id: None,
            display_name: Some(name.to_string()),
            theme: theme.to_string(),
            color: None,
            manual: true,
        });
        added += 1;
    }
    added
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

// ── Render groups list ──

pub fn render_content(
    data: RwSignal<Option<PopupData>>,
    toggle: impl Fn(String) + 'static + Clone + Send,
    is_expanded: impl Fn(&str) -> bool + 'static + Clone + Send,
    editing_name: RwSignal<Option<String>>,
    draft_name: RwSignal<String>,
    on_start_rename: impl Fn(String, String) + 'static + Clone + Send,
    on_commit_rename: impl Fn() + 'static + Clone + Send,
    on_cancel_rename: impl Fn() + 'static + Clone + Send,
    on_color_change: impl Fn(String, String) + 'static + Clone + Send,
    on_theme_change: impl Fn(String, String) + 'static + Clone + Send,
    on_dissolve_group: impl Fn(String) + 'static + Clone + Send,
) -> impl IntoView {
    // ── Reactive groups list for <For> ──
    let groups_each = move || {
        data.with(|opt| {
            opt.as_ref()
                .map(|pd| pd.groups.clone())
                .unwrap_or_default()
        })
    };

    view! {
        // ── Groups: keyed reactive list ──
        <For
            each=groups_each
            key=|g: &GroupDisplay| g.name.clone()
            children=move |g: GroupDisplay| {
                let name = g.name.clone();
                let display_name = g.display_name.clone();
                let color_hex = g.color_hex.clone();
                let color_name = g.color_name.clone();
                let theme = g.theme.clone();
                let tabs_data = g.tabs.clone();
                let count = g.tabs.len();

                // Toggle handler — owned by the button on:click
                let toggle_c = toggle.clone();
                let nt = name.clone();
                let on_toggle = move |_| toggle_c(nt.clone());

                // ── Clones for each reactive block below ──
                // Chevron reactive blocks
                let is_exp_chev1 = is_expanded.clone();
                let ne_chev1 = name.clone();
                let is_exp_chev2 = is_expanded.clone();
                let ne_chev2 = name.clone();

                // Name area reactive block
                let on_commit_name1 = on_commit_rename.clone();
                let on_commit_name2 = on_commit_rename.clone();
                let on_cancel_name = on_cancel_rename.clone();
                let on_start_name = on_start_rename.clone();
                let name_for_name_btn = name.clone();
                let display_for_name_btn = display_name.clone();

                // Expanded body reactive block
                let is_exp_body = is_expanded.clone();
                let ne_body = name.clone();
                let color_name_body = color_name.clone();
                let color_hex_body = color_hex.clone();
                let theme_body = theme.clone();
                let tabs_data_body = tabs_data.clone();

                // Colour and theme callbacks
                let oc = on_color_change.clone();
                let ot = on_theme_change.clone();
                let gn_color = name.clone();
                let gn_theme = name.clone();

                // Dissolve callback
                let od = on_dissolve_group.clone();
                let gn_dissolve = name.clone();
                let has_group_id = g.group_id.is_some();


                view! {
                    <section
                        class="tc-group"
                        style=format!("border-left-color:{}", color_hex)
                    >
                        // ── Group head (always visible) ──
                        <div class="tc-group-head">
                            <button
                                class="tc-chev"
                                on:click=on_toggle
                                aria-label={move || if is_exp_chev1(&ne_chev1) { "Replier" } else { "Deplier" }}
                            >
                                {move || if is_exp_chev2(&ne_chev2) { "\u{25BC}" } else { "\u{25B6}" }}
                            </button>

                            // colour dot — always visible
                            <span
                                class="tc-dot"
                                style=format!("background:{}", color_hex)
                            ></span>

                            // ── Name area: reactive toggle between display & rename input ──
                            {move || {
                                let is_editing = editing_name
                                    .with(|n| n.as_deref() == Some(&name));
                                if is_editing {
                                    let d_name = draft_name;
                                    let draft_val = d_name.get();
                                    let on_commit_key = on_commit_name1.clone();
                                    let on_cancel_key = on_cancel_name.clone();
                                    let on_commit_blur = on_commit_name2.clone();
                                    view! {
                                        <input
                                            class="tc-name-input"
                                            prop:value={draft_val}
                                            on:input=move |ev| {
                                                if let Some(target) = ev.target() {
                                                    if let Ok(val) = js_sys::Reflect::get(
                                                        &target,
                                                        &wasm_bindgen::JsValue::from_str("value"),
                                                    ) {
                                                        if let Some(s) = val.as_string() {
                                                            d_name.set(s);
                                                        }
                                                    }
                                                }
                                            }
                                            on:keydown=move |ev| {
                                                if ev.key() == "Enter" {
                                                    on_commit_key();
                                                }
                                                if ev.key() == "Escape" {
                                                    on_cancel_key();
                                                }
                                            }
                                            on:blur=move |_| on_commit_blur()
                                            autofocus=true
                                        />
                                    }
                                    .into_any()
                                } else {
                                    let on_start = on_start_name.clone();
                                    let gn = name_for_name_btn.clone();
                                    let gd = display_for_name_btn.clone();
                                    view! {
                                        <button
                                            class="tc-name-btn"
                                            on:click=move |_| on_start(gn.clone(), gd.clone())
                                            title="Renommer"
                                        >
                                            <span class="tc-name">{gd.clone()}</span>
                                            <span class="tc-pencil">"\u{270F}"</span>
                                        </button>
                                    }
                                    .into_any()
                                }
                            }}

                            // Tab count badge — always visible
                            <span class="tc-count">{count}</span>
                        </div>

                        // ── Expanded body: reactive conditional ──
                        {move || {
                            if is_exp_body(&ne_body) {
                                let palette_buttons: Vec<_> = PALETTE
                                    .iter()
                                    .map(|c| {
                                        let hex = lookup_hex(c).to_string();
                                        let active = *c == color_name_body;
                                        let style_val = if active {
                                            format!(
                                                "background:{};border-color:#fff;transform:scale(1.05)",
                                                hex
                                            )
                                        } else {
                                            format!("background:{}", hex)
                                        };
                                        let check = if active { "\u{2713}" } else { "" };
                                        let oc_btn = oc.clone();
                                        let gn_btn = gn_color.clone();
                                        let c_btn = c.to_string();
                                        view! {
                                            <button
                                                class="tc-swatch"
                                                title=format!("couleur {}", c)
                                                style=style_val
                                                on:click=move |_| oc_btn(gn_btn.clone(), c_btn.clone())
                                            >
                                                {check}
                                            </button>
                                        }
                                    })
                                    .collect();

                                let tab_items: Vec<_> = tabs_data_body
                                    .iter()
                                    .map(|tab| {
                                        let title = tab
                                            .title
                                            .as_deref()
                                            .unwrap_or("(sans titre)")
                                            .to_string();
                                        let url = tab
                                            .url
                                            .as_deref()
                                            .unwrap_or("")
                                            .to_string();
                                        let swatch = color_hex_body.clone();
                                        view! {
                                            <li class="tc-tab">
                                                <span
                                                    class="tc-favi"
                                                    style=format!("background:{}", swatch)
                                                ></span>
                                                <span class="tc-tab-title">{title}</span>
                                                <span class="tc-tab-url">{url}</span>
                                            </li>
                                        }
                                    })
                                    .collect();

                                // Clone ot inside so the on:blur closure doesn't consume the outer copy
                                let ot_inner = ot.clone();
                                let gn_theme_inner = gn_theme.clone();

                                view! {
                                    <div class="tc-group-body">
                                        <div class="tc-row">
                                            <span class="tc-row-label">"Couleur"</span>
                                            <div class="tc-palette">{palette_buttons}</div>
                                        </div>
                                        <div class="tc-row">
                                            <span class="tc-row-label">
                                                "Theme"
                                                <span class="tc-soon">"bientot"</span>
                                            </span>
                                            <input
                                                class="tc-theme-input"
                                                placeholder="Decris ce groupe pour le tri auto..."
                                                prop:value={theme_body.clone()}
                                                on:input=|_| {}
                                                on:blur=move |ev| {
                                                    if let Some(target) = ev.target() {
                                                        if let Ok(val) = js_sys::Reflect::get(
                                                            &target,
                                                            &wasm_bindgen::JsValue::from_str("value"),
                                                        ) {
                                                            if let Some(s) = val.as_string() {
                                                                let trimmed = s.trim().to_string();
                                                                if !trimmed.is_empty() {
                                                                    ot_inner(gn_theme_inner.clone(), trimmed);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            />
                                        </div>
                                        <p class="tc-theme-hint">
                                            "Servira au tri automatique : explique en quelques mots ce que contient ce groupe."
                                        </p>
                                        <ul class="tc-tab-list">{tab_items}</ul>
                                        {if has_group_id {
                                            let od_btn = od.clone();
                                            let gn_d_btn = gn_dissolve.clone();
                                            view! {
                                                <button
                                                    class="tc-dissolve-btn"
                                                    on:click=move |_| od_btn(gn_d_btn.clone())
                                                >
                                                    "Dissoudre"
                                                </button>
                                            }
                                            .into_any()
                                        } else {
                                            view! {}.into_any()
                                        }}
                                    </div>
                                }
                                .into_any()
                            } else {
                                view! {}.into_any()
                            }
                        }}
                    </section>
                }
            }
        />

        // ── Ungrouped section (reactive) ──
        {move || {
            data.with(|opt| {
                let ungrouped = opt
                    .as_ref()
                    .map(|pd| pd.ungrouped.clone())
                    .unwrap_or_default();
                let ungrouped_items: Vec<_> = ungrouped
                    .iter()
                    .map(|tab| {
                        let title = tab
                            .title
                            .as_deref()
                            .unwrap_or("(sans titre)")
                            .to_string();
                        let url = tab
                            .url
                            .as_deref()
                            .unwrap_or("")
                            .to_string();
                        view! {
                            <li class="tc-tab tc-other-tab">
                                <span class="tc-favi tc-favi-other"></span>
                                <span class="tc-tab-title">{title}</span>
                                <span class="tc-tab-url">{url}</span>
                            </li>
                        }
                    })
                    .collect();
                view! {
                    <section class="tc-other">
                        <div class="tc-other-head">
                            <span class="tc-other-icon">"\u{1F4E5}"</span>
                            <span class="tc-other-title">"Non ranges"</span>
                            <span class="tc-count">{ungrouped.len()}</span>
                        </div>
                        <ul class="tc-tab-list">{ungrouped_items}</ul>
                    </section>
                }
            })
        }}
    }
}

// ── Onboarding render ──

pub fn render_onboarding(
    onboarding_selected: RwSignal<Vec<usize>>,
    on_commencer: impl Fn() + 'static + Clone,
    on_passer: impl Fn() + 'static + Clone,
) -> impl IntoView {
    use crate::types::ONBOARDING_THEMES;

    let can_commencer = move || !onboarding_selected.with(|v| v.is_empty());

    view! {
        <div class="tc-onboarding">
            <div class="tc-onboarding-title">
                "Bienvenue ! Choisis les themes qui t'interessent"
            </div>
            <div class="tc-onboarding-sub">
                "Selectionne les themes que tu souhaites suivre. Des groupes seront crees automatiquement."
            </div>
            <div class="tc-onboarding-grid">
                {
                    ONBOARDING_THEMES.iter().enumerate().map(|(i, (name, theme))| {
                        let toggle = {
                            let os = onboarding_selected;
                            move |_| {
                                let mut sel = os.get();
                                if let Some(pos) = sel.iter().position(|x| *x == i) {
                                    sel.remove(pos);
                                } else {
                                    sel.push(i);
                                }
                                os.set(sel);
                            }
                        };
                        let is_sel = move || onboarding_selected.with(|v| v.contains(&i));
                        let preview = if theme.len() > 60 {
                            format!("{}…", &theme[..60])
                        } else {
                            theme.to_string()
                        };
                        view! {
                            <div
                                class="tc-onboarding-card"
                                class:tc-onboarding-card--selected=is_sel
                                on:click=toggle
                            >
                                <span class="tc-onboarding-card-name">{name.to_string()}</span>
                                <span class="tc-onboarding-card-preview">{preview}</span>
                                {move || if is_sel() {
                                    view! { <span class="tc-onboarding-check">"✓"</span> }.into_any()
                                } else {
                                    view! {}.into_any()
                                }}
                            </div>
                        }.into_any()
                    }).collect::<Vec<_>>()
                }
            </div>
            <button
                class="tc-onboarding-commencer"
                disabled=move || !can_commencer()
                on:click=move |_| on_commencer()
            >
                "Commencer"
            </button>
            <button
                class="tc-onboarding-passer"
                on:click=move |_| on_passer()
            >
                "Passer"
            </button>
        </div>
    }
}

// ── Static CSS ──
pub const CSS: &str = r#"
.tc-shell {
    width: 384px;
    max-width: 100%;
    min-height: 480px;
    max-height: 600px;
    display: flex;
    flex-direction: column;
    background: #171a21;
    color: #e7e9ee;
    font-family: 'Inter', ui-sans-serif, system-ui, sans-serif;
    font-size: 13px;
    line-height: 1.45;
    border: 1px solid #323845;
    border-radius: 14px;
    overflow: hidden;
}
.tc-header {
    padding: 14px 14px 12px;
    border-bottom: 1px solid #323845;
    background: linear-gradient(180deg, #21252e 0%, #171a21 100%);
}
.tc-brand-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
}
.tc-logo {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 18px;
    padding: 0 2px;
}
.tc-logo-bar {
    width: 4px;
    height: 18px;
    border-radius: 2px;
    display: block;
}
.tc-logo-purple { background: #9b6dd6; }
.tc-logo-cyan   { background: #46a7b8; height: 14px; }
.tc-logo-orange { background: #e08a3c; height: 9px; }
.tc-brand {
    font-weight: 700;
    font-size: 15px;
    letter-spacing: -0.2px;
}
.tc-sub {
    color: #8a8f98;
    font-size: 11.5px;
}
.tc-run {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 10px 12px;
    border-radius: 10px;
    border: none;
    cursor: pointer;
    background: linear-gradient(180deg, #6d8cff, #4f6ef0);
    color: #fff;
    font-weight: 600;
    font-size: 13.5px;
    letter-spacing: -0.1px;
    box-shadow: 0 1px 0 rgba(255,255,255,.14) inset, 0 4px 14px rgba(79,110,240,.35);
}
.tc-run:hover:not(:disabled) { filter: brightness(1.06); }
.tc-run:active:not(:disabled) { transform: translateY(1px); }
.tc-run:disabled {
    filter: saturate(.8) brightness(.95);
    cursor: default;
}
.tc-spin-icon {
    display: inline-block;
    animation: tc-spin 1s linear infinite;
}
@keyframes tc-spin { to { transform: rotate(360deg); } }
.tc-last-run {
    text-align: center;
    color: #8a8f98;
    font-size: 10.5px;
    margin-top: 8px;
    letter-spacing: .2px;
}
.tc-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.tc-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 20px;
    color: #8a8f98;
    text-align: center;
}
.tc-error { color: #e05a52; }
.tc-error-detail { font-size: 11px; opacity: .7; }
.tc-spinner {
    width: 24px;
    height: 24px;
    border: 3px solid #323845;
    border-top-color: #4f6ef0;
    border-radius: 50%;
    animation: tc-spin .8s linear infinite;
}
.tc-group {
    background: #21252e;
    border: 1px solid #323845;
    border-left: 3px solid #9b6dd6;
    border-radius: 10px;
    overflow: hidden;
    flex-shrink: 0;
}
.tc-group-head {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 9px 10px;
}
.tc-chev {
    background: none;
    border: none;
    color: #8a8f98;
    cursor: pointer;
    padding: 0;
    display: flex;
    font-size: 11px;
}
.tc-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
    display: inline-block;
}
.tc-name {
    font-weight: 600;
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    color: #e7e9ee;
}
.tc-name-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    color: #e7e9ee;
    cursor: pointer;
    padding: 0;
    flex: 1;
    min-width: 0;
    text-align: left;
}
.tc-name-btn:hover .tc-pencil {
    opacity: 1;
}
.tc-pencil {
    color: #8a8f98;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity .15s;
    font-size: 12px;
    line-height: 1;
}
.tc-name-input {
    flex: 1;
    background: #171a21;
    border: 1px solid #4f6ef0;
    border-radius: 6px;
    color: #e7e9ee;
    padding: 4px 7px;
    font-size: 13px;
    font-weight: 600;
    outline: none;
    min-width: 0;
}
.tc-count {
    margin-left: auto;
    background: #272c37;
    color: #8a8f98;
    font-size: 11px;
    font-weight: 600;
    min-width: 20px;
    height: 20px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 6px;
    flex-shrink: 0;
}
.tc-group-body {
    padding: 4px 10px 11px;
    border-top: 1px solid #323845;
}
.tc-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
}
.tc-row-label {
    color: #8a8f98;
    font-size: 11px;
    font-weight: 600;
    width: 56px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.tc-soon {
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: .4px;
    text-transform: uppercase;
    color: #e3b341;
    background: rgba(227,179,65,.12);
    border-radius: 4px;
    padding: 1px 4px;
    width: fit-content;
}
.tc-palette {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
}
.tc-swatch {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    border: 2px solid transparent;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
}
.tc-swatch:hover { transform: scale(1.08); }
.tc-theme-input {
    flex: 1;
    background: #171a21;
    border: 1px solid #323845;
    border-radius: 7px;
    color: #e7e9ee;
    padding: 7px 9px;
    font-size: 12.5px;
    outline: none;
}
.tc-theme-input::placeholder { color: #5a606c; }
.tc-theme-hint {
    color: #8a8f98;
    font-size: 10.5px;
    margin: 6px 0 0;
    padding-left: 66px;
    line-height: 1.4;
}
.tc-tab-list {
    list-style: none;
    margin: 11px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.tc-tab {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
    column-gap: 8px;
    row-gap: 0;
    padding: 6px 8px;
    border-radius: 7px;
    background: #272c37;
}
.tc-favi {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    grid-row: 1 / 3;
    align-self: center;
    flex-shrink: 0;
    display: inline-block;
}
.tc-favi-other { background: #3a3f4a; }
.tc-tab-title {
    font-size: 12.5px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.tc-tab-url {
    font-size: 10.5px;
    color: #8a8f98;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.tc-other {
    background: transparent;
    border: 1px dashed #323845;
    border-radius: 10px;
    padding: 9px 10px;
}
.tc-other-head {
    display: flex;
    align-items: center;
    gap: 7px;
}
.tc-other-icon { font-size: 14px; }
.tc-other-title {
    font-weight: 600;
    font-size: 12.5px;
    color: #8a8f98;
}
.tc-other-tab { opacity: .7; }
::-webkit-scrollbar { width: 9px; }
::-webkit-scrollbar-thumb { background: #323845; border-radius: 6px; border: 2px solid #171a21; }
::-webkit-scrollbar-track { background: transparent; }

/* ── New group creation ── */
.tc-new-group-area {
    margin-top: 8px;
}
.tc-new-group-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px dashed #323845;
    background: transparent;
    color: #8a8f98;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background .15s, color .15s;
}
.tc-new-group-btn:hover {
    background: #21252e;
    color: #e7e9ee;
    border-color: #4f6ef0;
}
.tc-new-group-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.tc-new-group-input {
    background: #171a21;
    border: 1px solid #4f6ef0;
    border-radius: 8px;
    color: #e7e9ee;
    padding: 8px 10px;
    font-size: 12.5px;
    outline: none;
}
.tc-new-group-input::placeholder { color: #5a606c; }
.tc-new-group-theme {
    background: #171a21;
    border: 1px solid #323845;
    border-radius: 8px;
    color: #e7e9ee;
    padding: 8px 10px;
    font-size: 12px;
    outline: none;
    resize: vertical;
    font-family: inherit;
    line-height: 1.4;
}
.tc-new-group-theme:focus { border-color: #4f6ef0; }
.tc-new-group-theme::placeholder { color: #5a606c; }
.tc-create-btn {
    padding: 8px 14px;
    border-radius: 8px;
    border: none;
    background: linear-gradient(180deg, #6d8cff, #4f6ef0);
    color: #fff;
    font-weight: 600;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
}
.tc-create-btn:hover:not(:disabled) { filter: brightness(1.06); }
.tc-create-btn:active:not(:disabled) { transform: translateY(1px); }
.tc-create-btn:disabled {
    opacity: 0.5;
    cursor: default;
}

/* ── Guided empty state ── */
.tc-empty-guided {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 20px;
    color: #8a8f98;
    text-align: center;
}
.tc-empty-guided p {
    margin: 0;
    line-height: 1.5;
}
.tc-create-guide {
    margin-top: 8px;
    padding: 10px 20px;
    font-size: 13px;
}

/* ── Dissolve button ── */
.tc-dissolve-btn {
    display: block;
    width: 100%;
    margin-top: 10px;
    padding: 7px 12px;
    border-radius: 8px;
    border: 1px solid #3a3033;
    background: transparent;
    color: #c78a8a;
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    transition: background .15s, border-color .15s;
}
.tc-dissolve-btn:hover {
    background: rgba(224,90,82,.08);
    border-color: #e05a52;
    color: #e05a52;
}

/* ── Download progress / status ── */
.tc-download-status {
    text-align: center;
    color: #8a8f98;
    font-size: 10.5px;
    margin-top: 4px;
}

/* ── Onboarding screen ── */
.tc-onboarding {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 20px 16px 16px;
    flex: 1;
    overflow-y: auto;
}
.tc-onboarding-title {
    font-weight: 700;
    font-size: 16px;
    color: #e7e9ee;
    text-align: center;
    margin-bottom: 6px;
    letter-spacing: -0.2px;
    line-height: 1.3;
}
.tc-onboarding-sub {
    color: #8a8f98;
    font-size: 12px;
    text-align: center;
    margin-bottom: 16px;
    line-height: 1.4;
}
.tc-onboarding-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    width: 100%;
    margin-bottom: 16px;
}
.tc-onboarding-card {
    background: #21252e;
    border: 1px solid #323845;
    border-radius: 10px;
    padding: 10px 8px;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s, transform 0.1s;
    display: flex;
    flex-direction: column;
    gap: 4px;
    user-select: none;
    position: relative;
    min-height: 62px;
}
.tc-onboarding-card:hover {
    border-color: #4f6ef0;
    background: #272c37;
}
.tc-onboarding-card:active {
    transform: scale(0.97);
}
.tc-onboarding-card--selected {
    border-color: #4f6ef0;
    background: rgba(79, 110, 240, 0.08);
}
.tc-onboarding-card--selected:hover {
    background: rgba(79, 110, 240, 0.12);
}
.tc-onboarding-card-name {
    font-weight: 600;
    font-size: 11.5px;
    color: #e7e9ee;
    line-height: 1.2;
}
.tc-onboarding-card-preview {
    font-size: 10px;
    color: #8a8f98;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
}
.tc-onboarding-check {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #4f6ef0;
    color: #fff;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 10px;
    font-weight: 700;
    line-height: 1;
}
.tc-onboarding-commencer {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 12px 16px;
    border-radius: 10px;
    border: none;
    cursor: pointer;
    background: linear-gradient(180deg, #6d8cff, #4f6ef0);
    color: #fff;
    font-weight: 600;
    font-size: 14px;
    letter-spacing: -0.1px;
    box-shadow: 0 1px 0 rgba(255,255,255,.14) inset, 0 4px 14px rgba(79,110,240,.35);
    margin-bottom: 10px;
}
.tc-onboarding-commencer:hover:not(:disabled) { filter: brightness(1.06); }
.tc-onboarding-commencer:active:not(:disabled) { transform: translateY(1px); }
.tc-onboarding-commencer:disabled {
    opacity: 0.5;
    cursor: default;
}
.tc-onboarding-passer {
    background: none;
    border: none;
    color: #5a606c;
    font-size: 11.5px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    transition: color 0.15s;
}
.tc-onboarding-passer:hover {
    color: #8a8f98;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GroupState, StoredGroup};

    #[test]
    fn test_apply_create_groups_batch_empty() {
        let mut state = GroupState {
            version: 1,
            groups: vec![],
        };
        let input: Vec<(String, String)> = vec![];
        let added = apply_create_groups_batch(&mut state, &input, 1000.0);
        assert_eq!(added, 0, "empty input must add zero groups");
        assert!(state.groups.is_empty());
    }

    #[test]
    fn test_apply_create_groups_batch_creates_multiple() {
        let mut state = GroupState {
            version: 1,
            groups: vec![],
        };
        let input: Vec<(String, String)> = vec![
            ("Dev".to_string(), "dev theme".to_string()),
            ("Videos".to_string(), "video theme".to_string()),
            ("Shopping".to_string(), "shop theme".to_string()),
        ];
        let added = apply_create_groups_batch(&mut state, &input, 1000.0);
        assert_eq!(added, 3, "must create 3 groups");
        assert_eq!(state.groups.len(), 3);
        assert_eq!(state.groups[0].name, "Dev");
        assert_eq!(state.groups[1].name, "Videos");
        assert_eq!(state.groups[2].name, "Shopping");
    }

    #[test]
    fn test_apply_create_groups_batch_idempotent() {
        let mut state = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "Dev".to_string(),
                keywords: vec![],
                created_at_ms: 500.0,
                updated_at_ms: 500.0,
                group_id: None,
                display_name: None,
                theme: "old theme".to_string(),
                color: None,
                manual: false,
            }],
        };
        let input: Vec<(String, String)> = vec![
            ("Dev".to_string(), "dev theme".to_string()),
            ("Videos".to_string(), "video theme".to_string()),
        ];
        let added = apply_create_groups_batch(&mut state, &input, 1000.0);
        assert_eq!(added, 1, "Dev is duplicate, only Videos must be added");
        assert_eq!(state.groups.len(), 2);
        // Existing group must NOT be mutated
        assert_eq!(state.groups[0].theme, "old theme");
        assert_eq!(state.groups[0].manual, false);
    }

    #[test]
    fn test_apply_create_groups_batch_sets_manual_true() {
        let mut state = GroupState {
            version: 1,
            groups: vec![],
        };
        let input: Vec<(String, String)> = vec![
            ("Finance".to_string(), "money theme".to_string()),
            ("Gaming".to_string(), "game theme".to_string()),
        ];
        let added = apply_create_groups_batch(&mut state, &input, 1000.0);
        assert_eq!(added, 2);
        for g in &state.groups {
            assert!(g.manual, "group '{}' must have manual: true", g.name);
            assert!(!g.theme.is_empty(), "group '{}' must have a theme", g.name);
            assert_eq!(g.display_name.as_deref(), Some(g.name.as_str()));
            assert!(!g.name.is_empty());
        }
    }
}
