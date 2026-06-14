use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys;

mod ffi;
mod grouping;
mod popup;
mod semantic;
mod sml;
mod storage;
mod types;

use crate::popup::{CSS, PopupData};

#[oxichrome::extension(
    name = "Tab Cleanner",
    version = "0.1.0",
    permissions = ["storage", "tabs", "tabGroups"]
)]
struct Extension;

/// Fire-and-forget background download of model + tokenizer.
/// Idempotent: does nothing if files are already cached.
/// Errors are logged but never surface to the user.
fn spawn_background_download(log_tag: &'static str) {
    wasm_bindgen_futures::spawn_local(async move {
        match crate::sml::ensure_model_cached(crate::types::MODEL_URL).await {
            Ok(src) => oxichrome::log!("[{}] Model cache: {}", log_tag, src),
            Err(e) => oxichrome::log!("[{}] Model download failed (best-effort): {}", log_tag, e),
        }
        match crate::sml::ensure_model_cached(crate::types::TOKENIZER_URL).await {
            Ok(src) => oxichrome::log!("[{}] Tokenizer cache: {}", log_tag, src),
            Err(e) => oxichrome::log!("[{}] Tokenizer download failed (best-effort): {}", log_tag, e),
        }
    });
}

#[oxichrome::background]
async fn start() {
    oxichrome::log!("Tab Cleanner started!");
    ffi::messaging::register_message_listener();
    // Heuristic run_grouping() retired; grouping is manual via the popup "Sort" button.

    // Safety net: download model in background if not already cached.
    // Idempotent — ensure_model_cached checks cache first.
    spawn_background_download("start");
}

#[oxichrome::on(runtime::on_installed)]
async fn handle_install(details: oxichrome::__private::wasm_bindgen::JsValue) {
    oxichrome::log!("Tab Cleanner installed: {:?}", details);
    // Auto-download model in background on install.
    spawn_background_download("install");
}

// ── Popup component (must be at root level for oxichrome-build detection) ──

#[oxichrome::popup]
fn Popup() -> impl IntoView {
    let expanded: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    let editing_controls: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    let loading = RwSignal::new(true);
    let error_msg: RwSignal<Option<String>> = RwSignal::new(None);
    let data: RwSignal<Option<PopupData>> = RwSignal::new(None);

    // Onboarding state
    let show_onboarding: RwSignal<bool> = RwSignal::new(false);
    let onboarding_selected: RwSignal<Vec<usize>> = RwSignal::new(Vec::new());
    let onboarding_pending_sort: RwSignal<bool> = RwSignal::new(false);

    // Semantic grouping state
    let model_cached: RwSignal<bool> = RwSignal::new(false);
    let is_semantic_ranking: RwSignal<bool> = RwSignal::new(false);

    // New group creation state
    let new_group_name: RwSignal<String> = RwSignal::new(String::new());
    let new_group_theme: RwSignal<String> = RwSignal::new(String::new());
    let show_new_group_input: RwSignal<bool> = RwSignal::new(false);

    // Initial load
    spawn_local(async move {
        // ── Onboarding detection ──
        // Only show onboarding when the marker is absent AND the groups list is empty.
        // If the marker is absent but groups already exist (e.g. restored from backup),
        // skip onboarding and proceed with the normal popup.
        let onboarding_done = crate::popup::is_onboarding_done().await;
        if !onboarding_done {
            // Read GroupState directly to check if any groups already exist
            let state: Option<crate::types::GroupState> =
                oxichrome::storage::get::<crate::types::GroupState>(crate::types::GROUP_STATE_KEY)
                    .await
                    .unwrap_or(None);
            let has_groups = state
                .as_ref()
                .map(|s| !s.groups.is_empty())
                .unwrap_or(false);
            if !has_groups {
                show_onboarding.set(true);
                loading.set(false);
                // Still check model cache in background
                let mc = model_cached.clone();
                spawn_local(async move {
                    match crate::popup::check_model_cached().await {
                        Ok(cached) => mc.set(cached),
                        Err(e) => {
                            oxichrome::log!("[popup] CheckModelCached failed: {}", e);
                        }
                    }
                });
                return;
            }
        }
        // ── Normal popup flow ──
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

        // Check if model is cached
        match crate::popup::check_model_cached().await {
            Ok(cached) => model_cached.set(cached),
            Err(e) => {
                oxichrome::log!("[popup] CheckModelCached failed: {}", e);
            }
        }
    });

    // Periodic re-check: poll every 5s until model is cached
    crate::popup::handlers::spawn_model_polling(
        model_cached,
        onboarding_pending_sort,
        data,
        error_msg,
    );

    // Shared refresh function (unused for now, but kept for future use)
    let _refresh_data = move || {
        crate::popup::handlers::handle_refresh_data(data, error_msg)
    };

    // Semantic grouping handler
    let on_semantic = move |_| {
        crate::popup::handlers::handle_semantic_grouping(data, error_msg, is_semantic_ranking)
    };

    // Toggle expand
    let toggle_expand = move |name: String| {
        crate::popup::handlers::handle_toggle_expand(name, expanded)
    };

    // Check if a group is expanded
    let is_expanded = move |name: &str| -> bool {
        crate::popup::handlers::handle_is_expanded(name, expanded)
    };

    // Rename state
    let editing_name: RwSignal<Option<String>> = RwSignal::new(None);
    let draft_name: RwSignal<String> = RwSignal::new(String::new());

    let start_rename = move |name: String, current_display: String| {
        crate::popup::handlers::handle_start_rename(name, current_display, editing_name, draft_name)
    };

    let commit_rename = move || {
        crate::popup::handlers::handle_commit_rename(data, editing_name, draft_name)
    };

    let cancel_rename = move || {
        crate::popup::handlers::handle_cancel_rename(editing_name)
    };

    // ── Colour change handler ──
    let on_color_change = move |group_name: String, color_name: String| {
        crate::popup::handlers::handle_color_change(group_name, color_name, data)
    };

    // ── New group creation handler ──
    let on_create_group = move || {
        crate::popup::handlers::handle_create_group(
            data,
            new_group_name,
            new_group_theme,
            show_new_group_input,
        )
    };

    // ── Dissolve group handler ──
    let on_dissolve_group = move |name: String| {
        crate::popup::handlers::handle_dissolve_group(name, data)
    };

    // ── Theme change handler ──
    let on_theme_change = move |group_name: String, theme_value: String| {
        crate::popup::handlers::handle_theme_change(group_name, theme_value, data)
    };

    // ── Editing controls toggle ──
    let toggle_edit_controls = move |name: String| {
        crate::popup::handlers::handle_toggle_expand(name, editing_controls)
    };
    let is_editing_controls = move |name: &str| -> bool {
        crate::popup::handlers::handle_is_expanded(name, editing_controls)
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
            {move || if show_onboarding.get() {
                crate::popup::render_onboarding(
                    onboarding_selected,
                    move || {
                        crate::popup::handlers::handle_onboarding_commencer(
                            onboarding_selected,
                            data,
                            error_msg,
                            onboarding_pending_sort,
                            show_onboarding,
                        )
                    },
                    move || {
                        crate::popup::handlers::handle_onboarding_passer(show_onboarding)
                    },
                ).into_any()
            } else {
                view! {
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
                                    format!("{} groups \u{b7} {} tabs", d.total_groups, d.total_tabs)
                                })
                                .unwrap_or_else(|| "\u{2014}".to_string())
                            }}
                        </div>
                    </div>
                </div>

                // ── Primary "Ranger" button (semantic grouping) ──
                <button
                    class="tc-run"
                    disabled={move || !model_cached.get() || is_semantic_ranking.get()}
                    on:click=on_semantic
                    title="Sort tabs into existing groups by thematic similarity. Tabs without a match stay unsorted."
                >
                    {move || if is_semantic_ranking.get() {
                        view! { <span class="tc-spin-icon">"\u{27F3}"</span> " Sorting\u{2026}" }.into_any()
                    } else if !model_cached.get() {
                        view! { <span>"\u{27F3}"</span> " Sort (model required)" }.into_any()
                    } else {
                        view! { <span>"\u{27F3}"</span> " Sort" }.into_any()
                    }}
                </button>

                // ── Model preparation status ──
                {move || if !model_cached.get() {
                    view! {
                        <div class="tc-download-status">
                            "Preparing semantic sorting\u{2026}"
                        </div>
                    }.into_any()
                } else {
                    view! {}.into_any()
                }}

                <div class="tc-last-run">
                    {move || if is_semantic_ranking.get() { "Sorting\u{2026}" } else { "Manual sorting via the button above" }}
                </div>

                // ── New group creation ──
                <div class="tc-new-group-area">
                    {move || if show_new_group_input.get() {
                        let name_val = new_group_name.get();
                        let theme_val = new_group_theme.get();
                        let on_name_input = crate::popup::handlers::make_on_name_input(new_group_name);
                        let on_theme_input = crate::popup::handlers::make_on_theme_input(new_group_theme);
                        let on_key1 = crate::popup::handlers::make_on_new_group_key(
                            new_group_name,
                            new_group_theme,
                            show_new_group_input,
                        );
                        let on_key2 = on_key1.clone();
                        let on_create_click = on_create_group.clone();
                        let name_empty = move || new_group_name.get().trim().is_empty();
                        let theme_empty = move || new_group_theme.get().trim().is_empty();
                        let can_create = move || !name_empty() && !theme_empty();
                        view! {
                            <div class="tc-new-group-form">
                                <input
                                    class="tc-new-group-input"
                                    prop:value={name_val}
                                    on:input=on_name_input
                                    on:keydown=on_key1
                                    autofocus=true
                                    placeholder="Group name..."
                                />
                                <textarea
                                    class="tc-new-group-theme"
                                    prop:value={theme_val}
                                    on:input=on_theme_input
                                    on:keydown=on_key2
                                    placeholder="Describe what this group should contain — sorting will place relevant tabs here."
                                    rows="2"
                                ></textarea>
                                <button
                                    class="tc-create-btn"
                                    disabled={move || !can_create()}
                                    on:click=move |_| {
                                        on_create_click();
                                    }
                                >
                                    "Create"
                                </button>
                            </div>
                        }
                        .into_any()
                    } else {
                        view! {
                            <button
                                class="tc-new-group-btn"
                                on:click=move |_| show_new_group_input.set(true)
                            >
                                "+ New group"
                            </button>
                        }
                        .into_any()
                    }}
                </div>
            </header>

            // ── Content ──
            <div class="tc-scroll">
                {move || {
                    if loading.get() {
                        view! {
                            <div class="tc-state">
                                <div class="tc-spinner"></div>
                                <p>"Loading groups\u{2026}"</p>
                            </div>
                        }.into_any()
                    } else if let Some(ref err) = error_msg.get() {
                        let msg = err.clone();
                        view! {
                            <div class="tc-state tc-error">
                                <p>"Failed to load data."</p>
                                <p class="tc-error-detail">{msg}</p>
                            </div>
                        }.into_any()
                    } else if data.get().is_some() {
                        let all_empty = data.with(|opt| {
                            opt.as_ref().map(|pd| pd.groups.iter().all(|g| g.theme.is_empty())).unwrap_or(false)
                        });
                        if all_empty {
                            view! {
                                <div class="tc-state tc-empty-guided">
                                    <p>"Create a group with a theme to get started."</p>
                                    <p>"Sorting will automatically place your matching tabs."</p>
                                    <button
                                        class="tc-create-btn tc-create-guide"
                                        on:click=move |_| show_new_group_input.set(true)
                                    >
                                        "+ Create my first group"
                                    </button>
                                </div>
                            }.into_any()
                        } else {
                            crate::popup::render_content(
                                data,
                                toggle_expand.clone(),
                                is_expanded.clone(),
                                toggle_edit_controls.clone(),
                                is_editing_controls.clone(),
                                editing_name,
                                draft_name,
                                start_rename.clone(),
                                commit_rename.clone(),
                                cancel_rename.clone(),
                                on_color_change,
                                on_theme_change,
                                on_dissolve_group,
                            ).into_any()
                        }
                    } else {
                        view! {}.into_any()
                    }
                }}
            </div>
                }.into_any()
            }}
        </div>
    }
}

// ── run_grouping() removed — heuristic grouping retired. ──
// Grouping is now performed exclusively via run_semantic_grouping()
// in src/semantic.rs, triggered by the "Sort" button in the popup.
