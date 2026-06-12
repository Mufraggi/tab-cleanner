pub mod css;
pub mod data;
pub mod handlers;
pub mod messaging;
pub mod persistence;
pub mod render;

// ── Re-exports for crate::popup::… ──
pub use css::CSS;
pub use data::{check_model_cached, fetch_popup_data, PopupData};
pub use messaging::{
    send_dissolve_group_best_effort, send_update_group_best_effort, trigger_semantic_grouping,
};
pub use persistence::{
    is_onboarding_done, persist_create_group, persist_create_groups_batch,
    persist_group_fields, set_onboarding_done,
};
pub use render::{render_content, render_onboarding};

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

// ── Palette of colours (excluding grey) ──
pub const PALETTE: &[&str] = &[
    "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange",
];
