use oxichrome::storage;

use crate::types::{GroupState, GROUP_STATE_KEY, ONBOARDING_DONE_KEY};

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
    state.groups.push(crate::types::StoredGroup::new_manual(
        name.to_string(),
        theme.to_string(),
        now,
    ));

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
/// If `StoredGroup` gains new fields, update `StoredGroup::new_manual` — all call sites
/// (including this one) automatically pick up the new defaults.
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
        state.groups.push(crate::types::StoredGroup::new_manual(
            name.to_string(),
            theme.to_string(),
            now,
        ));
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
        state.groups.push(crate::types::StoredGroup::new_manual(
            name.to_string(),
            theme.to_string(),
            now_ms,
        ));
        added += 1;
    }
    added
}

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
