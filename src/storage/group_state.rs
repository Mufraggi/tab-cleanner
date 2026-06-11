use crate::types::{GroupState, GROUP_STATE_KEY};
use oxichrome::storage;

/// Load persisted group state from storage.
/// Returns a fresh empty GroupState on first run (key absent).
pub async fn load_state() -> GroupState {
    storage::get::<GroupState>(GROUP_STATE_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or_else(|| GroupState {
            version: 1,
            groups: vec![],
        })
}

/// Save group state to storage (fire-and-forget on error).
pub async fn save_state(state: &GroupState) {
    let _ = storage::set(GROUP_STATE_KEY, state).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_state_roundtrip() {
        let state = GroupState {
            version: 1,
            groups: vec![
                crate::types::StoredGroup {
                    name: "github.com".into(),
                    keywords: vec!["rust".into(), "compiler".into()],
                    created_at_ms: 1718100000000.0,
                    updated_at_ms: 1718100000000.0,
                    group_id: None,
                    display_name: None,
                    theme: String::new(),
                    color: None,
                    manual: false,
                },
            ],
        };
        let json = serde_json::to_string(&state).unwrap();
        let roundtripped: GroupState = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.version, 1);
        assert_eq!(roundtripped.groups.len(), 1);
        assert_eq!(roundtripped.groups[0].name, "github.com");
        assert_eq!(roundtripped.groups[0].keywords.len(), 2);
    }
}
