use crate::ffi::{tab_groups, tabs_ext};
use crate::types::{GroupAssignment, GroupState, StoredGroup};
use std::collections::HashMap;

/// Deterministically pick a Chrome tab group colour from a group name.
///
/// Avoids `grey`. Uses a DJB2-like hash of the name to select one of the 8
/// other valid Chrome colours (`blue`, `red`, `yellow`, `green`, `pink`,
/// `purple`, `cyan`, `orange`).
///
/// Guaranteed: same `name` always returns the same colour.
pub fn pick_color(name: &str) -> &'static str {
    const COLORS: &[&str] = &[
        "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange",
    ];
    // DJB2-style hash — deterministic, no external randomness
    let hash = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    COLORS[(hash as usize) % COLORS.len()]
}

/// Apply Chrome tab group operations based on group assignments.
///
/// For each domain-based group (skipping `"Other"`):
///   - If a `group_id` is already stored and still valid, tabs are added to
///     the existing Chrome group, then title and colour are refreshed.
///   - If the stored `group_id` is stale (Chrome API returns an error), the
///     id is cleared and a brand-new group is created.
///   - If no `group_id` exists yet, a new Chrome group is created, the new id
///     is captured, title and colour are applied.
///
/// For tabs assigned to `"Other"`, any existing Chrome group membership is
/// removed via `chrome.tabs.ungroup` (no-op for ungrouped tabs).
///
/// All Chrome API errors are caught and logged — the service worker NEVER
/// crashes. Returns the updated `GroupState` with populated `group_id`s.
pub async fn apply_groups(
    assignments: &[GroupAssignment],
    state: &GroupState,
) -> GroupState {
    // ── Step 1: Build group_name → Vec<tab_id> map, skipping "Other" ──
    let mut name_to_tabs: HashMap<String, Vec<i32>> = HashMap::new();
    for a in assignments {
        if a.group_name == "Other" {
            continue;
        }
        name_to_tabs
            .entry(a.group_name.clone())
            .or_default()
            .push(a.tab_id);
    }

    // ── Step 1b: Collect "Other" tab IDs for ungrouping ──
    let other_tab_ids: Vec<i32> = assignments
        .iter()
        .filter(|a| a.group_name == "Other")
        .map(|a| a.tab_id)
        .collect();

    // ── Step 2: Process each stored group ──
    let mut updated_groups: Vec<StoredGroup> = Vec::new();

    for mut group in state.groups.clone() {
        if let Some(tab_ids) = name_to_tabs.remove(&group.name) {
            // This group appears in fresh assignments → action needed
            if let Some(gid) = group.group_id {
                // 2a — Try to add tabs to existing Chrome group
                match tabs_ext::create_tab_group(&tab_ids, Some(gid)).await {
                    Ok(_) => {
                        apply_group_properties(gid, &group.name, group.color.as_deref()).await;
                    }
                    Err(e) => {
                        oxichrome::log!(
                            "apply_groups: group_id {} for '{}' invalid, recreating: {:?}",
                            gid,
                            &group.name,
                            e
                        );
                        group.group_id = None;
                        create_new_group_and_update(&mut group, &tab_ids).await;
                    }
                }
            } else {
                // 2b — No group_id yet → create new Chrome group
                create_new_group_and_update(&mut group, &tab_ids).await;
            }
        }
        // else: group not in fresh assignments → leave untouched
        updated_groups.push(group);
    }

    // ── Step 3: Safety net — groups in name_to_tabs not in state ──
    // After reconcile() this should be empty, but handle edge cases.
    for (name, tab_ids) in name_to_tabs {
        let mut new_group = StoredGroup {
            name: name.clone(),
            keywords: Vec::new(),
            created_at_ms: js_sys::Date::now(),
            updated_at_ms: js_sys::Date::now(),
            group_id: None,
            display_name: None,
            theme: String::new(),
            color: None,
        };
        create_new_group_and_update(&mut new_group, &tab_ids).await;
        updated_groups.push(new_group);
    }

    // ── Step 4: Ungroup "Other" tabs ──
    if !other_tab_ids.is_empty() {
        if let Err(e) = tabs_ext::ungroup_tabs(&other_tab_ids).await {
            oxichrome::log!(
                "apply_groups: failed to ungroup {} tabs: {:?}",
                other_tab_ids.len(),
                e
            );
        }
    }

    GroupState {
        version: state.version,
        groups: updated_groups,
    }
}

/// Create a new Chrome tab group for the given tab IDs, then apply title and
/// colour. Writes the returned group id into `group.group_id`.
///
/// All errors are logged; the function never panics.
async fn create_new_group_and_update(group: &mut StoredGroup, tab_ids: &[i32]) {
    match tabs_ext::create_tab_group(tab_ids, None).await {
        Ok(new_id) => {
            group.group_id = Some(new_id);
            apply_group_properties(new_id, &group.name, group.color.as_deref()).await;
        }
        Err(e) => {
            oxichrome::log!(
                "apply_groups: failed to create group for '{}': {:?}",
                &group.name,
                e
            );
        }
    }
}

/// Update the title and colour of an existing Chrome tab group.
///
/// `color_override` if present, takes precedence over the deterministic `pick_color()`.
/// All errors are logged; the function never panics.
async fn apply_group_properties(group_id: i32, name: &str, color_override: Option<&str>) {
    let color = color_override.unwrap_or_else(|| pick_color(name));
    let props = tab_groups::UpdateProperties {
        color: Some(color.to_string()),
        title: Some(name.to_string()),
    };
    if let Err(e) = tab_groups::update_tab_group(group_id, &props).await {
        oxichrome::log!(
            "apply_groups: failed to update group {} (id={}): {:?}",
            name,
            group_id,
            e
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pick_color_deterministic() {
        // Same name always produces the same colour
        let c1 = pick_color("github.com");
        let c2 = pick_color("github.com");
        assert_eq!(c1, c2);

        // Verify it's one of the 8 valid colours
        let valid: Vec<&str> = vec![
            "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange",
        ];
        assert!(valid.contains(&c1), "unexpected colour: {}", c1);
    }

    #[test]
    fn test_pick_color_not_grey() {
        // Grey must never be returned
        for name in &["test", "hello", "world", "rust", "compiler"] {
            let color = pick_color(name);
            assert_ne!(color, "grey", "pick_color('{}') returned grey", name);
        }
    }

    #[test]
    fn test_pick_color_same_input_same_output() {
        // Deterministic across repeated calls
        let names = ["github.com", "docs.rs", "youtube.com", "a", ""];
        for name in &names {
            let first = pick_color(name);
            for _ in 0..10 {
                assert_eq!(pick_color(name), first);
            }
        }
    }

    #[test]
    fn test_pick_color_all_colors_reachable() {
        // Over 100 distinct names, all 8 colours should appear
        let mut seen = std::collections::HashSet::new();
        for i in 0..100 {
            let name = format!("test-{}", i);
            seen.insert(pick_color(&name));
        }
        assert_eq!(seen.len(), 8, "not all 8 Chrome colours reachable");
        for color in &[
            "blue",
            "red",
            "yellow",
            "green",
            "pink",
            "purple",
            "cyan",
            "orange",
        ] {
            assert!(seen.contains(color), "colour '{}' never returned", color);
        }
    }

    #[test]
    fn test_pick_color_different_names_can_differ() {
        // Not a strict requirement, but at least verify the function doesn't
        // return the same colour for every input (which would indicate a bug).
        let colours: std::collections::HashSet<&str> = (0..20)
            .map(|i| pick_color(&format!("domain{}.com", i)))
            .collect();
        assert!(
            colours.len() > 1,
            "expected multiple colours for different names, got only {:?}",
            colours
        );
    }
}
