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
    /// The cleaned domain that was used for grouping, if any.
    /// None for tabs without a valid URL (chrome://, empty, etc.).
    pub domain: Option<String>,
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

/// Storage key for the group state in chrome.storage.local.
pub const GROUP_STATE_KEY: &str = "tab_cleanner_group_state";

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
}
