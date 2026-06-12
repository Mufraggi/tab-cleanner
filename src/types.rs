use serde::{Deserialize, Serialize};

/// Helper to default ungrouped Chrome tab groupId to -1.
fn no_group() -> i32 { -1 }

/// What we query Chrome for — minimal fields needed for grouping.
/// `allow(dead_code)` because `title` is only used for keyword extraction
/// and `url` is only consumed by the domain extractor.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    pub id: i32,
    pub url: Option<String>,    // None for new-tab page, chrome://, etc.
    pub title: Option<String>,  // None for tabs without a title
    /// The Chrome tab group this tab belongs to, or -1 if ungrouped.
    #[serde(default = "no_group")]
    pub group_id: i32,
}

/// Input to tabs::query.
/// `current_window: Some(true)` limits results to the current window.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryAllTabs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_window: Option<bool>,
}

/// The output of the grouping algorithm. One per tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupAssignment {
    pub tab_id: i32,
    /// Human-readable group label, e.g. "github.com", "YouTube", "Other"
    pub group_name: String,
    /// Key representative keywords extracted from the title.
    /// Empty if the title was missing or contained no useful words.
    pub keywords: Vec<String>,
}

/// A known group persisted across runs.
/// The `name` field is the domain (e.g. "github.com") and also serves as the stable identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredGroup {
    pub name: String,
    pub keywords: Vec<String>,
    pub created_at_ms: f64,
    pub updated_at_ms: f64,
    /// The Chrome tab group id, if the group has been created in the browser.
    /// `None` for groups that haven't been materialised yet.
    #[serde(default)]
    pub group_id: Option<i32>,

    /// Optional display name shown in the UI instead of the domain `name`.
    /// `None` means the UI should display `name` (the domain).
    #[serde(default)]
    pub display_name: Option<String>,

    /// Theme colour for the group. Empty string means default / unset.
    #[serde(default)]
    pub theme: String,

    /// User colour override. If None, the deterministic pick_color() is used.
    #[serde(default)]
    pub color: Option<String>,

    /// Whether this group was created manually by the user (not via automatic grouping).
    /// Manual groups remain visible in the popup even with zero open tabs.
    #[serde(default)]
    pub manual: bool,
}

impl StoredGroup {
    /// Construct a new manual group with default values.
    ///
    /// Sets `display_name = Some(name)`, `manual = true`, and all other
    /// fields to their natural defaults. Centralises the construction pattern
    /// that was previously duplicated at 4 call sites.
    pub fn new_manual(name: String, theme: String, now_ms: f64) -> Self {
        StoredGroup {
            name: name.clone(),
            keywords: vec![],
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            group_id: None,
            display_name: Some(name),
            theme,
            color: None,
            manual: true,
        }
    }
}

/// Top-level persistence payload stored under GROUP_STATE_KEY.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupState {
    pub version: u32,
    pub groups: Vec<StoredGroup>,
}

/// Query tabs by Chrome group id.
/// Used by `handle_dissolve_group` to find tabs belonging to a group being dissolved.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryByGroupId {
    pub group_id: i32,
}

// ── HuggingFace CDN URLs for the sentence-transformers model ──
/// URL for the all-MiniLM-L6-v2 model weights (safetensors f16).
pub const MODEL_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/model.safetensors";
/// URL for the all-MiniLM-L6-v2 tokenizer vocabulary (tokenizer.json).
pub const TOKENIZER_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

/// Storage key for the group state in chrome.storage.local.
pub const GROUP_STATE_KEY: &str = "tab_cleanner_group_state";

/// Storage key for the onboarding-done marker.
/// Set to `true` after the user completes or skips the onboarding flow.
pub const ONBOARDING_DONE_KEY: &str = "tab_cleanner_onboarding_done";

/// The 12 onboarding themes presented as a grid of cards.
/// Each tuple is (display name, theme description).
///
/// Thèmes courts (3-5 mots-clés forts) pour des ancres sémantiques denses.
/// Les groupes DÉJÀ créés avec les anciens thèmes longs ne sont pas migrés ;
/// il faut recréer ou éditer leurs thèmes pour profiter des nouvelles ancres.
pub const ONBOARDING_THEMES: [(&str, &str); 12] = [
    ("Dev / Tech", "développement code programmation github logiciel"),
    ("Vidéos", "vidéos youtube streaming films séries"),
    ("Réseaux sociaux", "réseaux sociaux linkedin twitter facebook instagram"),
    ("Shopping", "achats shopping boutique e-commerce commande colis"),
    ("Actualités", "actualités news presse journal article"),
    ("Travail / Productivité", "travail productivité documents email projet"),
    ("Apprentissage", "cours apprentissage formation tutoriel éducation"),
    ("Voyage", "voyage vol hôtel réservation destination"),
    ("Finance", "banque finance investissement bourse budget"),
    ("Gaming", "jeux vidéo gaming esport guide"),
    ("Santé / Bien-être", "santé bien-être sport fitness nutrition"),
    ("Cuisine / Recettes", "cuisine recette gastronomie repas restaurant"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stored_group_deserialize_without_group_id() {
        // Old JSON without group_id field — must deserialize to None
        let json = r#"{
            "name": "github.com",
            "keywords": ["rust", "compiler"],
            "created_at_ms": 1000.0,
            "updated_at_ms": 2000.0
        }"#;
        let group: StoredGroup = serde_json::from_str(json).expect("deserialize old JSON");
        assert_eq!(group.name, "github.com");
        assert_eq!(group.keywords, vec!["rust", "compiler"]);
        assert_eq!(group.created_at_ms, 1000.0);
        assert_eq!(group.updated_at_ms, 2000.0);
        assert_eq!(group.group_id, None, "missing group_id must default to None");
    }

    #[test]
    fn test_stored_group_deserialize_with_group_id() {
        // New JSON with group_id present
        let json = r#"{
            "name": "github.com",
            "keywords": ["rust"],
            "created_at_ms": 1000.0,
            "updated_at_ms": 2000.0,
            "group_id": 42
        }"#;
        let group: StoredGroup = serde_json::from_str(json).expect("deserialize with group_id");
        assert_eq!(group.group_id, Some(42));
        // display_name and theme not in JSON → must default
        assert_eq!(group.display_name, None);
        assert_eq!(group.theme, "");
    }

    #[test]
    fn test_stored_group_deserialize_without_display_name_and_theme() {
        // Old JSON without display_name and theme fields
        // Must deserialize to default values: None and ""
        let json = r#"{
            "name": "youtube.com",
            "keywords": ["video"],
            "created_at_ms": 1000.0,
            "updated_at_ms": 2000.0,
            "group_id": 7
        }"#;
        let group: StoredGroup = serde_json::from_str(json).expect("deserialize old JSON");
        assert_eq!(group.name, "youtube.com");
        assert_eq!(group.keywords, vec!["video"]);
        assert_eq!(group.group_id, Some(7));
        assert_eq!(group.display_name, None, "missing display_name must default to None");
        assert_eq!(group.theme, "", "missing theme must default to empty string");
    }

    #[test]
    fn test_stored_group_deserialize_without_manual() {
        // Old JSON without manual field — must deserialize to false
        let json = r#"{
            "name": "github.com",
            "keywords": ["rust"],
            "created_at_ms": 1000.0,
            "updated_at_ms": 2000.0,
            "group_id": 42,
            "display_name": null,
            "theme": ""
        }"#;
        let group: StoredGroup = serde_json::from_str(json).expect("deserialize old JSON without manual");
        assert_eq!(group.manual, false, "missing manual must default to false");
    }

    #[test]
    fn test_stored_group_deserialize_without_color() {
        // Old JSON without color field — must deserialize to None
        let json = r#"{
            "name": "github.com",
            "keywords": ["rust"],
            "created_at_ms": 1000.0,
            "updated_at_ms": 2000.0,
            "group_id": 42,
            "display_name": null,
            "theme": ""
        }"#;
        let group: StoredGroup = serde_json::from_str(json).expect("deserialize");
        assert_eq!(group.color, None);
    }

    // ── Onboarding theme constant tests ──

    #[test]
    fn test_onboarding_themes_count() {
        assert_eq!(ONBOARDING_THEMES.len(), 12);
    }

    #[test]
    fn test_onboarding_themes_unique_names() {
        let mut names: Vec<&str> = ONBOARDING_THEMES.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let orig_len = names.len();
        names.dedup();
        assert_eq!(names.len(), orig_len, "all 12 theme names must be unique");
    }

    #[test]
    fn test_onboarding_themes_non_empty() {
        for (i, (name, theme)) in ONBOARDING_THEMES.iter().enumerate() {
            assert!(!name.is_empty(), "theme {} name must not be empty", i);
            assert!(!theme.is_empty(), "theme {} description must not be empty", i);
        }
    }

    #[test]
    fn test_onboarding_done_key_value() {
        assert_eq!(ONBOARDING_DONE_KEY, "tab_cleanner_onboarding_done");
    }
}
