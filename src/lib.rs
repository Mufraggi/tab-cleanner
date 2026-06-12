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
    // Heuristic run_grouping() retired; grouping is manual via the popup "Ranger" button.

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
    {
        let model_cached = model_cached.clone();
        let onboarding_pending_sort = onboarding_pending_sort;
        let data = data.clone();
        let error_msg = error_msg.clone();
        spawn_local(async move {
            loop {
                // Wait 5 seconds
                let promise = js_sys::Promise::new(&mut |resolve, _| {
                    if let Some(window) = web_sys::window() {
                        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                            &resolve, 5000,
                        );
                    }
                });
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;

                if model_cached.get() {
                    break;
                }
                match crate::popup::check_model_cached().await {
                    Ok(true) => {
                        model_cached.set(true);
                        if onboarding_pending_sort.get_untracked() {
                            onboarding_pending_sort.set(false);
                            let data = data.clone();
                            let error_msg = error_msg.clone();
                            spawn_local(async move {
                                if let Err(e) = crate::popup::trigger_semantic_grouping().await {
                                    error_msg.set(Some(e));
                                    return;
                                }
                                match crate::popup::fetch_popup_data().await {
                                    Ok(pd) => data.set(Some(pd)),
                                    Err(e) => error_msg.set(Some(e)),
                                }
                            });
                        }
                        break;
                    }
                    Ok(false) => { /* still not cached, continue polling */ }
                    Err(e) => {
                        oxichrome::log!("[popup] CheckModelCached poll failed: {}", e);
                    }
                }
            }
        });
    }

    // Shared refresh function (unused for now, but kept for future use)
    let _refresh_data = {
        let data = data;
        let error_msg = error_msg;
        move || {
            let data = data.clone();
            let error_msg = error_msg.clone();
            spawn_local(async move {
                match crate::popup::fetch_popup_data().await {
                    Ok(pd) => {
                        data.set(Some(pd));
                        error_msg.set(None);
                    }
                    Err(e) => {
                        error_msg.set(Some(e));
                    }
                }
            });
        }
    };

    // Semantic grouping handler
    let on_semantic = {
        let data = data.clone();
        let error_msg = error_msg.clone();
        move |_| {
            if is_semantic_ranking.get_untracked() {
                return;
            }
            is_semantic_ranking.set(true);
            let data = data.clone();
            let error_msg = error_msg.clone();
            spawn_local(async move {
                match crate::popup::trigger_semantic_grouping().await {
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
                        error_msg.set(Some(e));
                    }
                }
                is_semantic_ranking.set(false);
            });
        }
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
        let editing_name = editing_name;
        let draft_name = draft_name;
        move || {
            if let Some(ref group_name) = editing_name.get_untracked() {
                let new_name = draft_name.get_untracked().trim().to_string();
                if !new_name.is_empty() {
                    let group_name = group_name.clone();
                    let new_name_clone = new_name.clone();
                    let data = data.clone();

                    // Optimistic UI update
                    data.update(|opt| {
                        if let Some(ref mut pd) = opt {
                            if let Some(g) = pd.groups.iter_mut().find(|g| g.name == group_name) {
                                g.display_name = new_name_clone;
                            }
                        }
                    });

                    wasm_bindgen_futures::spawn_local(async move {
                        // 1. Persist directly to storage (no background worker needed)
                        let persist = crate::popup::persist_group_fields(
                            &group_name,
                            Some(&new_name),
                            None,
                            None,
                        ).await;

                        // 2. Refresh UI from storage
                        match crate::popup::fetch_popup_data().await {
                            Ok(pd) => {
                                data.set(Some(pd));
                            }
                            Err(e) => {
                                oxichrome::log!("[popup] Refresh apres renommage echoue: {}", e);
                            }
                        }

                        // 3. Best-effort: notify background to update Chrome native group
                        if persist.is_ok() {
                            crate::popup::send_update_group_best_effort(
                                group_name,
                                Some(new_name),
                                None,
                                None,
                            ).await;
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
        move |group_name: String, color_name: String| {
            let hex = crate::popup::lookup_hex(&color_name).to_string();
            let gn = group_name.clone();
            let cn = color_name.clone();

            // 1. Update local display immediately (optimistic)
            data.update(|opt| {
                if let Some(ref mut pd) = opt {
                    if let Some(g) = pd.groups.iter_mut().find(|g| g.name == gn) {
                        g.color_name = cn;
                        g.color_hex = hex;
                    }
                }
            });

            let data = data.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // 2. Persist directly to storage (no background worker needed)
                let persist = crate::popup::persist_group_fields(
                    &group_name,
                    None,
                    Some(&color_name),
                    None,
                ).await;

                // 3. Refresh UI from storage
                match crate::popup::fetch_popup_data().await {
                    Ok(pd) => {
                        data.set(Some(pd));
                    }
                    Err(e) => {
                        oxichrome::log!("[popup] Refresh apres couleur echoue: {}", e);
                    }
                }

                // 4. Best-effort: notify background to update Chrome native group
                if persist.is_ok() {
                    crate::popup::send_update_group_best_effort(
                        group_name,
                        None,
                        Some(color_name),
                        None,
                    ).await;
                }
            });
        }
    };

    // ── New group creation handler ──
    let on_create_group = {
        let data = data.clone();
        move || {
            let name = new_group_name.get_untracked();
            let theme = new_group_theme.get_untracked();
            let trimmed_name = name.trim().to_string();
            let trimmed_theme = theme.trim().to_string();
            if trimmed_name.is_empty() || trimmed_theme.is_empty() {
                return;
            }
            new_group_name.set(String::new());
            new_group_theme.set(String::new());
            show_new_group_input.set(false);
            let data = data.clone();
            spawn_local(async move {
                let _ = crate::popup::persist_create_group(&trimmed_name, &trimmed_theme).await;
                match crate::popup::fetch_popup_data().await {
                    Ok(pd) => {
                        data.set(Some(pd));
                    }
                    Err(e) => {
                        oxichrome::log!("[popup] Refresh apres creation echoue: {}", e);
                    }
                }
            });
        }
    };

    // ── Dissolve group handler ──
    let on_dissolve_group = {
        let data = data.clone();
        move |name: String| {
            let data = data.clone();
            spawn_local(async move {
                crate::popup::send_dissolve_group_best_effort(name).await;
                match crate::popup::fetch_popup_data().await {
                    Ok(pd) => {
                        data.set(Some(pd));
                    }
                    Err(e) => {
                        oxichrome::log!("[popup] Refresh apres dissolution echoue: {}", e);
                    }
                }
            });
        }
    };

    // ── Theme change handler ──
    let on_theme_change = {
        let data = data;
        move |group_name: String, theme_value: String| {
            let data = data.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // 1. Persist directly to storage ONLY (theme has no Chrome API effect)
                let _ = crate::popup::persist_group_fields(
                    &group_name,
                    None,
                    None,
                    Some(&theme_value),
                ).await;

                // 2. Refresh UI from storage
                match crate::popup::fetch_popup_data().await {
                    Ok(pd) => {
                        data.set(Some(pd));
                    }
                    Err(e) => {
                        oxichrome::log!("[popup] Refresh apres theme echoue: {}", e);
                    }
                }
                // NO background call — theme is storage-only
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
            {move || if show_onboarding.get() {
                crate::popup::render_onboarding(
                    onboarding_selected,
                    {
                        let data = data.clone();
                        let error_msg = error_msg.clone();
                        let onboarding_pending_sort = onboarding_pending_sort;
                        let show_onboarding = show_onboarding;
                        move || {
                            let selected: Vec<(String, String)> = onboarding_selected
                                .get()
                                .iter()
                                .map(|&i| {
                                    let (name, theme) = crate::types::ONBOARDING_THEMES[i];
                                    (name.to_string(), theme.to_string())
                                })
                                .collect();
                            let data = data.clone();
                            let error_msg = error_msg.clone();
                            spawn_local(async move {
                                if let Err(e) = crate::popup::persist_create_groups_batch(&selected).await {
                                    error_msg.set(Some(e));
                                    return;
                                }
                                crate::popup::set_onboarding_done().await;
                                let cached = crate::popup::check_model_cached().await.unwrap_or(false);
                                if cached {
                                    if let Err(e) = crate::popup::trigger_semantic_grouping().await {
                                        error_msg.set(Some(e));
                                    }
                                    match crate::popup::fetch_popup_data().await {
                                        Ok(pd) => data.set(Some(pd)),
                                        Err(e) => error_msg.set(Some(e)),
                                    }
                                } else {
                                    onboarding_pending_sort.set(true);
                                    match crate::popup::fetch_popup_data().await {
                                        Ok(pd) => data.set(Some(pd)),
                                        Err(e) => error_msg.set(Some(e)),
                                    }
                                }
                                show_onboarding.set(false);
                            });
                        }
                    },
                    {
                        let show_onboarding = show_onboarding;
                        move || {
                            spawn_local(async move {
                                crate::popup::set_onboarding_done().await;
                                show_onboarding.set(false);
                            });
                        }
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
                                    format!("{} groupes . {} onglets", d.total_groups, d.total_tabs)
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
                    title="Range les onglets dans les groupes existants par similarite thematique. Les onglets sans correspondance restent non ranges."
                >
                    {move || if is_semantic_ranking.get() {
                        view! { <span class="tc-spin-icon">"\u{27F3}"</span> " Rangement\u{2026}" }.into_any()
                    } else if !model_cached.get() {
                        view! { <span>"\u{1F9E0}"</span> " Ranger (modele requis)" }.into_any()
                    } else {
                        view! { <span>"\u{1F9E0}"</span> " Ranger" }.into_any()
                    }}
                </button>

                // ── Model preparation status ──
                {move || if !model_cached.get() {
                    view! {
                        <div class="tc-download-status">
                            "Preparation du tri semantique\u{2026}"
                        </div>
                    }.into_any()
                } else {
                    view! {}.into_any()
                }}

                <div class="tc-last-run">
                    {move || if is_semantic_ranking.get() { "Rangement en cours\u{2026}" } else { "Rangement manuel via le bouton ci-dessus" }}
                </div>

                // ── New group creation ──
                <div class="tc-new-group-area">
                    {move || if show_new_group_input.get() {
                        let name_val = new_group_name.get();
                        let theme_val = new_group_theme.get();
                        let on_name_input = move |ev: leptos::ev::Event| {
                            if let Some(target) = ev.target() {
                                if let Ok(v) = js_sys::Reflect::get(
                                    &target,
                                    &wasm_bindgen::JsValue::from_str("value"),
                                ) {
                                    if let Some(s) = v.as_string() {
                                        new_group_name.set(s);
                                    }
                                }
                            }
                        };
                        let on_theme_input = move |ev: leptos::ev::Event| {
                            if let Some(target) = ev.target() {
                                if let Ok(v) = js_sys::Reflect::get(
                                    &target,
                                    &wasm_bindgen::JsValue::from_str("value"),
                                ) {
                                    if let Some(s) = v.as_string() {
                                        new_group_theme.set(s);
                                    }
                                }
                            }
                        };
                        let on_key = move |ev: leptos::ev::KeyboardEvent| {
                            if ev.key() == "Escape" {
                                show_new_group_input.set(false);
                                new_group_name.set(String::new());
                                new_group_theme.set(String::new());
                            }
                        };
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
                                    on:keydown=on_key
                                    autofocus=true
                                    placeholder="Nom du groupe..."
                                />
                                <textarea
                                    class="tc-new-group-theme"
                                    prop:value={theme_val}
                                    on:input=on_theme_input
                                    on:keydown=on_key
                                    placeholder="Decris ce que ce groupe doit contenir — le tri rangera les onglets pertinents ici."
                                    rows="2"
                                ></textarea>
                                <button
                                    class="tc-create-btn"
                                    disabled={move || !can_create()}
                                    on:click=move |_| {
                                        on_create_click();
                                    }
                                >
                                    "Creer"
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
                                "+ Nouveau groupe"
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
                        let all_empty = data.with(|opt| {
                            opt.as_ref().map(|pd| pd.groups.iter().all(|g| g.theme.is_empty())).unwrap_or(false)
                        });
                        if all_empty {
                            view! {
                                <div class="tc-state tc-empty-guided">
                                    <p>"Cree un groupe avec un theme pour commencer."</p>
                                    <p>"Le tri rangera automatiquement tes onglets correspondants."</p>
                                    <button
                                        class="tc-create-btn tc-create-guide"
                                        on:click=move |_| show_new_group_input.set(true)
                                    >
                                        "+ Creer mon premier groupe"
                                    </button>
                                </div>
                            }.into_any()
                        } else {
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
// in src/semantic.rs, triggered by the "Ranger" button in the popup.
