pub mod apply;

use crate::types::{GroupAssignment, GroupState, StoredGroup};
use std::collections::{HashMap, HashSet};

// ── Heuristic grouping (group_tabs, find_best_keyword_match) removed. ──
// Grouping is now performed exclusively by the SML pipeline (src/semantic.rs).
// The reconcile() function below is kept intact — it remains the bridge between
// fresh GroupAssignment (from SML) and persisted StoredGroup state.

/// Reconcile fresh group assignments with persisted state to produce
/// an updated GroupState ready for storage.
///
/// `now_ms` is the current timestamp (ms since epoch) used for new/updated groups.
/// It is passed as a parameter so this function remains pure and testable outside WASM.
///
/// Rules (idempotent):
/// - For each unique group_name in fresh assignments (except "Other"),
///   ensure a StoredGroup with that name exists.
/// - If a StoredGroup already exists, reuse it (update keywords + timestamp).
/// - If not, create a new one.
/// - Groups not present in fresh assignments are left untouched (never deleted).
/// - "Other" is never persisted.
pub fn reconcile(
    fresh: &[GroupAssignment],
    stored: &GroupState,
    now_ms: f64,
) -> GroupState {
    // Build name → StoredGroup map from existing state
    let mut existing: HashMap<String, StoredGroup> = stored
        .groups
        .iter()
        .map(|g| (g.name.clone(), g.clone()))
        .collect();

    // Collect unique domain-based group names from fresh assignments
    let fresh_names: HashSet<&str> = fresh
        .iter()
        .filter_map(|a| {
            if a.group_name == "Other" {
                None
            } else {
                Some(a.group_name.as_str())
            }
        })
        .collect();

    // Ensure every fresh group name has a StoredGroup entry
    for name in &fresh_names {
        existing.entry(name.to_string()).or_insert_with(|| StoredGroup {
            name: name.to_string(),
            keywords: vec![],
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            group_id: None,
            display_name: None,
            theme: String::new(),
            color: None,
            manual: false,
        });
    }

    // Recompute keywords for each group that appears in fresh assignments
    let mut updated_groups: Vec<StoredGroup> = vec![];
    for (name, mut group) in existing {
        if fresh_names.contains(name.as_str()) {
            // Collect keywords from all fresh assignments in this group
            let mut kw_set: HashSet<String> = HashSet::new();
            for a in fresh.iter().filter(|a| a.group_name == name) {
                for kw in &a.keywords {
                    kw_set.insert(kw.clone());
                }
            }
            // Cap at 10, then sort for deterministic ordering
            let mut keywords: Vec<String> = kw_set.into_iter().take(10).collect();
            keywords.sort();
            group.keywords = keywords;
            group.updated_at_ms = now_ms;
        }
        updated_groups.push(group);
    }

    // Sort groups by name for deterministic output order
    updated_groups.sort_by(|a, b| a.name.cmp(&b.name));

    GroupState {
        version: 1,
        groups: updated_groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // reconcile() tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_reconcile_preserves_color() {
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec!["rust".into()],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: Some(42),
                display_name: None,
                theme: String::new(),
                color: Some("blue".into()),
                manual: false,
            }],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "github.com".into(),
            keywords: vec!["rust".into()],
        }];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups[0].color, Some("blue".into()));
    }

    #[test]
    fn test_reconcile_preserves_existing_group_id() {
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec!["rust".into()],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: Some(42),
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "github.com".into(),
            keywords: vec!["rust".into()],
        }];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].group_id, Some(42));
    }

    #[test]
    fn test_reconcile_preserves_display_name_and_theme() {
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "youtube.com".into(),
                keywords: vec!["video".into()],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: Some(5),
                display_name: Some("YouTube".into()),
                theme: "blue".into(),
                color: None,
                manual: false,
            }],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "youtube.com".into(),
            keywords: vec!["video".into()],
        }];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].display_name, Some("YouTube".into()));
        assert_eq!(result.groups[0].theme, "blue");
        assert_eq!(result.groups[0].group_id, Some(5));
    }

    #[test]
    fn test_reconcile_first_run_creates_groups() {
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                keywords: vec!["rust".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "docs.rs".into(),
                keywords: vec!["docs".into()],
            },
            GroupAssignment {
                tab_id: 3,
                group_name: "Other".into(),
                keywords: vec![],
            },
        ];
        let stored = GroupState {
            version: 1,
            groups: vec![],
        };
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 2); // "Other" not persisted
        let names: Vec<&str> = result.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"github.com"));
        assert!(names.contains(&"docs.rs"));
    }

    #[test]
    fn test_reconcile_no_duplicate_on_rerun() {
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec!["rust".into()],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: None,
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                keywords: vec!["rust".into(), "compiler".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "github.com".into(),
                keywords: vec!["cli".into()],
            },
        ];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1); // Still only one github.com
        let g = &result.groups[0];
        assert_eq!(g.name, "github.com");
        assert_eq!(g.created_at_ms, 1000.0); // original creation time preserved
        assert!(g.updated_at_ms > 1000.0); // updated timestamp
        assert!(g.keywords.contains(&"rust".to_string()));
        assert!(g.keywords.contains(&"compiler".to_string()));
        assert!(g.keywords.contains(&"cli".to_string()));
    }

    #[test]
    fn test_reconcile_adds_new_domain() {
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec![],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: None,
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                keywords: vec![],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "youtube.com".into(),
                keywords: vec!["video".into()],
            },
        ];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 2);
    }

    #[test]
    fn test_reconcile_preserves_orphaned_groups() {
        let stored = GroupState {
            version: 1,
            groups: vec![
                StoredGroup {
                    name: "github.com".into(),
                    keywords: vec!["rust".into()],
                    created_at_ms: 1000.0,
                    updated_at_ms: 1000.0,
                    group_id: None,
                    display_name: None,
                    theme: String::new(),
                    color: None,
                    manual: false,
                },
                StoredGroup {
                    name: "old-domain.com".into(),
                    keywords: vec![],
                    created_at_ms: 500.0,
                    updated_at_ms: 500.0,
                    group_id: None,
                    display_name: None,
                    theme: String::new(),
                    color: None,
                    manual: false,
                },
            ],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "github.com".into(),
            keywords: vec!["rust".into()],
        }];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 2); // old-domain.com still there
    }

    #[test]
    fn test_reconcile_all_other_creates_no_groups() {
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "Other".into(),
                keywords: vec![],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "Other".into(),
                keywords: vec![],
            },
        ];
        let stored = GroupState {
            version: 1,
            groups: vec![],
        };
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 0);
    }

    #[test]
    fn test_reconcile_empty_fresh_preserves_existing() {
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec!["rust".into()],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: None,
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };
        let fresh: Vec<GroupAssignment> = vec![];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].name, "github.com");
    }

    #[test]
    fn test_reconcile_preserves_manual() {
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec!["rust".into()],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: Some(42),
                display_name: None,
                theme: String::new(),
                color: None,
                manual: true,
            }],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "github.com".into(),
            keywords: vec!["rust".into()],
        }];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].manual, true);
    }

    #[test]
    fn test_reconcile_new_group_manual_false() {
        let stored = GroupState {
            version: 1,
            groups: vec![],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "github.com".into(),
            keywords: vec!["rust".into()],
        }];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].manual, false);
    }

    // -----------------------------------------------------------------------
    // Idempotence tests — verifies that multiple runs produce stable state
    // -----------------------------------------------------------------------

    #[test]
    fn test_reconcile_two_runs_same_assignments_idempotent() {
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                keywords: vec!["rust".into(), "compiler".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "github.com".into(),
                keywords: vec!["cli".into(), "tool".into()],
            },
            GroupAssignment {
                tab_id: 3,
                group_name: "docs.rs".into(),
                keywords: vec!["oxichrome".into(), "api".into()],
            },
            GroupAssignment {
                tab_id: 4,
                group_name: "Other".into(),
                keywords: vec![],
            },
        ];

        let stored = GroupState {
            version: 1,
            groups: vec![],
        };
        let state1 = reconcile(&fresh, &stored, 1000.0);
        let state2 = reconcile(&fresh, &state1, 2000.0);

        assert_eq!(
            state1.groups.len(),
            state2.groups.len(),
            "Group count must not grow between runs"
        );

        let names1: HashSet<&str> = state1.groups.iter().map(|g| g.name.as_str()).collect();
        let names2: HashSet<&str> = state2.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names1, names2, "Group names must match between runs");

        assert_eq!(names1.len(), state1.groups.len(), "Duplicates in state1");
        assert_eq!(names2.len(), state2.groups.len(), "Duplicates in state2");

        for g1 in &state1.groups {
            let g2 = state2
                .groups
                .iter()
                .find(|g| g.name == g1.name)
                .expect("Group must exist in second run");

            assert_eq!(
                g1.keywords, g2.keywords,
                "Keywords for '{}' must match between runs",
                g1.name
            );

            assert_eq!(
                g1.created_at_ms, g2.created_at_ms,
                "created_at_ms for '{}' must be preserved",
                g1.name
            );

            assert_eq!(
                g2.updated_at_ms, 2000.0,
                "updated_at_ms for '{}' must be updated to second run timestamp",
                g2.name
            );
        }
    }

    #[test]
    fn test_reconcile_three_runs_no_duplicate_groups() {
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                keywords: vec!["rust".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "youtube.com".into(),
                keywords: vec!["tutorial".into()],
            },
        ];

        let state1 = reconcile(&fresh, &GroupState { version: 1, groups: vec![] }, 1000.0);
        let state2 = reconcile(&fresh, &state1, 2000.0);
        let state3 = reconcile(&fresh, &state2, 3000.0);

        assert_eq!(state1.groups.len(), 2);
        assert_eq!(state2.groups.len(), 2, "Duplicated group after 2nd run");
        assert_eq!(state3.groups.len(), 2, "Duplicated group after 3rd run");

        let names1: HashSet<&str> = state1.groups.iter().map(|g| g.name.as_str()).collect();
        let names3: HashSet<&str> = state3.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names1, names3);
    }

    #[test]
    fn test_reconcile_same_inputs_identical_output() {
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                keywords: vec!["rust".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "docs.rs".into(),
                keywords: vec!["documentation".into()],
            },
        ];
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec!["rust".into()],
                created_at_ms: 500.0,
                updated_at_ms: 500.0,
                group_id: None,
                display_name: None,
                theme: String::new(),
                color: None,
                manual: false,
            }],
        };

        let result1 = reconcile(&fresh, &stored, 1000.0);
        let result2 = reconcile(&fresh, &stored, 1000.0);

        assert_eq!(result1.version, result2.version);
        assert_eq!(result1.groups.len(), result2.groups.len());

        for (g1, g2) in result1.groups.iter().zip(result2.groups.iter()) {
            assert_eq!(g1.name, g2.name);
            assert_eq!(g1.keywords, g2.keywords);
            assert_eq!(g1.created_at_ms, g2.created_at_ms);
            assert_eq!(g1.updated_at_ms, g2.updated_at_ms);
        }
    }
}
