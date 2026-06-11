use leptos::prelude::*;
use oxichrome::prelude::*;
use std::collections::HashSet;
use wasm_bindgen_futures::spawn_local;
use web_sys;

mod ffi;
mod grouping;
mod popup;
mod storage;
mod types;

use crate::popup::{CSS, PopupData};
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
    ffi::messaging::register_message_listener();
    let _ = run_grouping().await;
}

#[oxichrome::on(runtime::on_installed)]
async fn handle_install(details: oxichrome::__private::wasm_bindgen::JsValue) {
    oxichrome::log!("Tab Cleanner installed: {:?}", details);
}

// ── Popup component (must be at root level for oxichrome-build detection) ──

#[oxichrome::popup]
fn Popup() -> impl IntoView {
    let expanded: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    let is_ranking = RwSignal::new(false);
    let loading = RwSignal::new(true);
    let error_msg: RwSignal<Option<String>> = RwSignal::new(None);
    let data: RwSignal<Option<PopupData>> = RwSignal::new(None);

    // Initial load
    spawn_local(async move {
        match crate::popup::fetch_popup_data().await {
            Ok(pd) => {
                data.set(Some(pd));
                loading.set(false);
            }
            Err(e) => {
                error_msg.set(Some(e));
                loading.set(false);
            }
        }
    });

    // Run grouping handler
    let on_run = move |_| {
        if is_ranking.get_untracked() {
            return;
        }
        is_ranking.set(true);
        spawn_local(async move {
            let _ = crate::popup::trigger_run_grouping().await;
            // Refetch data after grouping
            match crate::popup::fetch_popup_data().await {
                Ok(pd) => {
                    data.set(Some(pd));
                    error_msg.set(None);
                }
                Err(e) => {
                    error_msg.set(Some(e));
                }
            }
            is_ranking.set(false);
        });
    };

    // Toggle expand
    let toggle_expand = move |name: String| {
        let mut v = expanded.get();
        if let Some(pos) = v.iter().position(|x| x == &name) {
            v.remove(pos);
        } else {
            v.push(name);
        }
        expanded.set(v);
    };

    // Check if a group is expanded
    let is_expanded = move |name: &str| -> bool {
        expanded.with(|v| v.iter().any(|x| x == name))
    };

    // Rename state
    let editing_name: RwSignal<Option<String>> = RwSignal::new(None);
    let draft_name: RwSignal<String> = RwSignal::new(String::new());

    let start_rename = move |name: String, current_display: String| {
        editing_name.set(Some(name));
        draft_name.set(current_display);
    };

    let commit_rename = {
        let data = data;
        let error_msg = error_msg;
        let editing_name = editing_name;
        let draft_name = draft_name;
        move || {
            if let Some(ref group_name) = editing_name.get_untracked() {
                let new_name = draft_name.get_untracked().trim().to_string();
                if !new_name.is_empty() {
                    let group_name = group_name.clone();
                    let new_name_clone = new_name.clone();
                    let data = data.clone();
                    let error_msg = error_msg.clone();

                    // Optimistic UI update
                    data.update(|opt| {
                        if let Some(ref mut pd) = opt {
                            if let Some(g) = pd.groups.iter_mut().find(|g| g.name == group_name) {
                                g.display_name = new_name_clone;
                            }
                        }
                    });

                    // Send to background
                    wasm_bindgen_futures::spawn_local(async move {
                        match crate::popup::send_update_group(
                            group_name,
                            Some(new_name),
                            None,
                            None,
                        ).await {
                            Ok(()) => {
                                // Full refresh to pick up persisted value
                                match crate::popup::fetch_popup_data().await {
                                    Ok(pd) => {
                                        data.set(Some(pd));
                                        error_msg.set(None);
                                    }
                                    Err(e) => {
                                        error_msg.set(Some(e));
                                    }
                                }
                            }
                            Err(e) => {
                                error_msg.set(Some(format!("Echec renommage : {}", e)));
                                // On failure, refresh to revert optimistic update
                                match crate::popup::fetch_popup_data().await {
                                    Ok(pd) => {
                                        data.set(Some(pd));
                                    }
                                    Err(_) => {}
                                }
                            }
                        }
                    });
                }
            }
            editing_name.set(None);
        }
    };

    let cancel_rename = move || {
        editing_name.set(None);
    };

    // ── Colour change handler ──
    let on_color_change = {
        let data = data;
        let error_msg = error_msg;
        move |group_name: String, color_name: String| {
            let data = data.clone();
            let error_msg = error_msg.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match crate::popup::send_update_group(
                    group_name,
                    None,           // display_name unchanged
                    Some(color_name),
                    None,           // theme unchanged
                ).await {
                    Ok(()) => {
                        // Refresh popup data
                        match crate::popup::fetch_popup_data().await {
                            Ok(pd) => {
                                data.set(Some(pd));
                                error_msg.set(None);
                            }
                            Err(e) => {
                                error_msg.set(Some(e));
                            }
                        }
                    }
                    Err(e) => {
                        error_msg.set(Some(format!("Echec couleur : {}", e)));
                    }
                }
            });
        }
    };

    // ── Theme change handler ──
    let on_theme_change = {
        let data = data;
        let error_msg = error_msg;
        move |group_name: String, theme_value: String| {
            let data = data.clone();
            let error_msg = error_msg.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match crate::popup::send_update_group(
                    group_name,
                    None,           // display_name unchanged
                    None,           // color unchanged
                    Some(theme_value),
                ).await {
                    Ok(()) => {
                        match crate::popup::fetch_popup_data().await {
                            Ok(pd) => {
                                data.set(Some(pd));
                                error_msg.set(None);
                            }
                            Err(e) => {
                                error_msg.set(Some(e));
                            }
                        }
                    }
                    Err(e) => {
                        error_msg.set(Some(format!("Echec theme : {}", e)));
                    }
                }
            });
        }
    };

    // ── Render ──
    // Inject CSS into <head> on mount (avoids Leptos view! <style> text escaping)
    Effect::new(move |_| {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if doc.get_element_by_id("tc-styles").is_none() {
                if let Ok(style_el) = doc.create_element("style") {
                    let _ = style_el.set_attribute("id", "tc-styles");
                    style_el.set_text_content(Some(CSS));
                    if let Some(head) = doc.head() {
                        let _ = head.append_child(&style_el);
                    }
                }
            }
        }
    });

    view! {
        <div class="tc-shell">
            // ── Header ──
            <header class="tc-header">
                <div class="tc-brand-row">
                    <div class="tc-logo" aria-hidden="true">
                        <span class="tc-logo-bar tc-logo-purple"></span>
                        <span class="tc-logo-bar tc-logo-cyan"></span>
                        <span class="tc-logo-bar tc-logo-orange"></span>
                    </div>
                    <div>
                        <div class="tc-brand">"Tab Cleanner"</div>
                        <div class="tc-sub">
                            {move || {
                                data.get().as_ref().map(|d| {
                                    format!("{} groupes . {} onglets", d.total_groups, d.total_tabs)
                                })
                                .unwrap_or_else(|| "\u{2014}".to_string())
                            }}
                        </div>
                    </div>
                </div>

                <button
                    class="tc-run"
                    disabled={move || is_ranking.get()}
                    on:click=on_run
                >
                    {move || if is_ranking.get() {
                        view! { <span class="tc-spin-icon">"\u{27F3}"</span> " Rangement\u{2026}" }.into_any()
                    } else {
                        view! { <span>"\u{25B6}"</span> " Ranger maintenant" }.into_any()
                    }}
                </button>

                <div class="tc-last-run">
                    {move || if is_ranking.get() { "Rangement en cours\u{2026}" } else { "Dernier rangement a l'ouverture" }}
                </div>
            </header>

            // ── Content ──
            <div class="tc-scroll">
                {move || {
                    if loading.get() {
                        view! {
                            <div class="tc-state">
                                <div class="tc-spinner"></div>
                                <p>"Chargement des groupes\u{2026}"</p>
                            </div>
                        }.into_any()
                    } else if let Some(ref err) = error_msg.get() {
                        let msg = err.clone();
                        view! {
                            <div class="tc-state tc-error">
                                <p>"Impossible de charger les donnees."</p>
                                <p class="tc-error-detail">{msg}</p>
                            </div>
                        }.into_any()
                    } else if data.get().is_some() {
                        crate::popup::render_content(
                            data,
                            toggle_expand.clone(),
                            is_expanded.clone(),
                            editing_name,
                            draft_name,
                            start_rename.clone(),
                            commit_rename.clone(),
                            cancel_rename.clone(),
                            on_color_change,
                            on_theme_change,
                        ).into_any()
                    } else {
                        view! {}.into_any()
                    }
                }}
            </div>
        </div>
    }
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
