use leptos::prelude::*;

use super::{PALETTE, lookup_hex};
use super::data::{GroupDisplay, PopupData};

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
