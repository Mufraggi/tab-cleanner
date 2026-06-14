use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::popup::PopupData;

// ── Expand / collapse ──

pub fn handle_toggle_expand(name: String, expanded: RwSignal<Vec<String>>) {
    let mut v = expanded.get();
    if let Some(pos) = v.iter().position(|x| x == &name) {
        v.remove(pos);
    } else {
        v.push(name);
    }
    expanded.set(v);
}

pub fn handle_is_expanded(name: &str, expanded: RwSignal<Vec<String>>) -> bool {
    expanded.with(|v| v.iter().any(|x| x == name))
}

// ── Rename ──

pub fn handle_start_rename(
    name: String,
    current_display: String,
    editing_name: RwSignal<Option<String>>,
    draft_name: RwSignal<String>,
) {
    editing_name.set(Some(name));
    draft_name.set(current_display);
}

pub fn handle_commit_rename(
    data: RwSignal<Option<PopupData>>,
    editing_name: RwSignal<Option<String>>,
    draft_name: RwSignal<String>,
) {
    if let Some(ref group_name) = editing_name.get_untracked() {
        let new_name = draft_name.get_untracked().trim().to_string();
        if !new_name.is_empty() {
            let group_name = group_name.clone();
            let new_name_clone = new_name.clone();
            let data = data;

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
                )
                .await;

                // 2. Refresh UI from storage
                match crate::popup::fetch_popup_data().await {
                    Ok(pd) => {
                        data.set(Some(pd));
                    }
                    Err(e) => {
                        oxichrome::log!("[popup] Refresh after rename failed: {}", e);
                    }
                }

                // 3. Best-effort: notify background to update Chrome native group
                if persist.is_ok() {
                    crate::popup::send_update_group_best_effort(
                        group_name,
                        Some(new_name),
                        None,
                        None,
                    )
                    .await;
                }
            });
        }
    }
    editing_name.set(None);
}

pub fn handle_cancel_rename(editing_name: RwSignal<Option<String>>) {
    editing_name.set(None);
}

// ── Colour change ──

pub fn handle_color_change(
    group_name: String,
    color_name: String,
    data: RwSignal<Option<PopupData>>,
) {
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

    let data = data;
    wasm_bindgen_futures::spawn_local(async move {
        // 2. Persist directly to storage (no background worker needed)
        let persist = crate::popup::persist_group_fields(
            &group_name,
            None,
            Some(&color_name),
            None,
        )
        .await;

        // 3. Refresh UI from storage
        match crate::popup::fetch_popup_data().await {
            Ok(pd) => {
                data.set(Some(pd));
            }
            Err(e) => {
                oxichrome::log!("[popup] Refresh after color failed: {}", e);
            }
        }

        // 4. Best-effort: notify background to update Chrome native group
        if persist.is_ok() {
            crate::popup::send_update_group_best_effort(
                group_name,
                None,
                Some(color_name),
                None,
            )
            .await;
        }
    });
}

// ── Create group ──

pub fn handle_create_group(
    data: RwSignal<Option<PopupData>>,
    new_group_name: RwSignal<String>,
    new_group_theme: RwSignal<String>,
    show_new_group_input: RwSignal<bool>,
) {
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
    let data = data;
    spawn_local(async move {
        let _ = crate::popup::persist_create_group(&trimmed_name, &trimmed_theme).await;
        match crate::popup::fetch_popup_data().await {
            Ok(pd) => {
                data.set(Some(pd));
            }
            Err(e) => {
                oxichrome::log!("[popup] Refresh after creation failed: {}", e);
            }
        }
    });
}

// ── Dissolve group ──

pub fn handle_dissolve_group(name: String, data: RwSignal<Option<PopupData>>) {
    let data = data;
    spawn_local(async move {
        crate::popup::send_dissolve_group_best_effort(name).await;
        match crate::popup::fetch_popup_data().await {
            Ok(pd) => {
                data.set(Some(pd));
            }
            Err(e) => {
                oxichrome::log!("[popup] Refresh after dissolve failed: {}", e);
            }
        }
    });
}

// ── Theme change ──

pub fn handle_theme_change(
    group_name: String,
    theme_value: String,
    data: RwSignal<Option<PopupData>>,
) {
    let data = data;
    wasm_bindgen_futures::spawn_local(async move {
        // 1. Persist directly to storage ONLY (theme has no Chrome API effect)
        let _ = crate::popup::persist_group_fields(&group_name, None, None, Some(&theme_value))
            .await;

        // 2. Refresh UI from storage
        match crate::popup::fetch_popup_data().await {
            Ok(pd) => {
                data.set(Some(pd));
            }
            Err(e) => {
                oxichrome::log!("[popup] Refresh after theme failed: {}", e);
            }
        }
        // NO background call — theme is storage-only
    });
}

// ── Semantic grouping (Ranger) ──

pub fn handle_semantic_grouping(
    data: RwSignal<Option<PopupData>>,
    error_msg: RwSignal<Option<String>>,
    is_semantic_ranking: RwSignal<bool>,
) {
    if is_semantic_ranking.get_untracked() {
        return;
    }
    is_semantic_ranking.set(true);
    let data = data;
    let error_msg = error_msg;
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

// ── Refresh data (currently unused) ──

pub fn handle_refresh_data(
    data: RwSignal<Option<PopupData>>,
    error_msg: RwSignal<Option<String>>,
) {
    let data = data;
    let error_msg = error_msg;
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

// ── Model polling loop ──

pub fn spawn_model_polling(
    model_cached: RwSignal<bool>,
    onboarding_pending_sort: RwSignal<bool>,
    data: RwSignal<Option<PopupData>>,
    error_msg: RwSignal<Option<String>>,
) {
    spawn_local(async move {
        loop {
            // Wait 5 seconds
            let promise = js_sys::Promise::new(&mut |resolve, _| {
                if let Some(window) = web_sys::window() {
                    let _ = window
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5000);
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
                        let data = data;
                        let error_msg = error_msg;
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

// ── Onboarding handlers ──

pub fn handle_onboarding_commencer(
    onboarding_selected: RwSignal<Vec<usize>>,
    data: RwSignal<Option<PopupData>>,
    error_msg: RwSignal<Option<String>>,
    onboarding_pending_sort: RwSignal<bool>,
    show_onboarding: RwSignal<bool>,
) {
    let selected: Vec<(String, String)> = onboarding_selected
        .get()
        .iter()
        .map(|&i| {
            let (name, theme) = crate::types::ONBOARDING_THEMES[i];
            (name.to_string(), theme.to_string())
        })
        .collect();
    let data = data;
    let error_msg = error_msg;
    spawn_local(async move {
        if let Err(e) = crate::popup::persist_create_groups_batch(&selected).await {
            error_msg.set(Some(e));
            return;
        }
        crate::popup::set_onboarding_done().await;
        let cached = crate::popup::check_model_cached()
            .await
            .unwrap_or(false);
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

pub fn handle_onboarding_passer(show_onboarding: RwSignal<bool>) {
    spawn_local(async move {
        crate::popup::set_onboarding_done().await;
        show_onboarding.set(false);
    });
}

// ── New group input factories (thin wrappers returning closures) ──

pub fn make_on_name_input(
    new_group_name: RwSignal<String>,
) -> impl Fn(leptos::ev::Event) + 'static + Clone {
    move |ev: leptos::ev::Event| {
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
    }
}

pub fn make_on_theme_input(
    new_group_theme: RwSignal<String>,
) -> impl Fn(leptos::ev::Event) + 'static + Clone {
    move |ev: leptos::ev::Event| {
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
    }
}

pub fn make_on_new_group_key(
    new_group_name: RwSignal<String>,
    new_group_theme: RwSignal<String>,
    show_new_group_input: RwSignal<bool>,
) -> impl Fn(leptos::ev::KeyboardEvent) + 'static + Clone {
    move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Escape" {
            show_new_group_input.set(false);
            new_group_name.set(String::new());
            new_group_theme.set(String::new());
        }
    }
}
