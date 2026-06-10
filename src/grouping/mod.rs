pub mod apply;
mod domain;
mod keywords;

use crate::types::{GroupAssignment, GroupState, StoredGroup, TabInfo};
use std::collections::{HashMap, HashSet};

/// Main grouping function.
///
/// Algorithm (deterministic, two-pass):
///   1. First pass — group tabs by domain. Tabs sharing the same domain
///      get a group named after that domain (e.g. "github.com").
///   2. Second pass — for tabs without a domain (None), attempt to match
///      their keywords against existing domain-based groups. If a tab's
///      keywords overlap significantly with a domain group, assign it
///      to that group.
///   3. Remaining unmatched tabs → group "Other".
///
/// Edge cases handled:
///   - Tabs with no URL → domain = None → second pass or "Other"
///   - chrome:// pages → domain = None → "Other" (unless keyword match)
///   - Multiple tabs from same domain → all go to same group
///   - Single-tab domain → still gets its own group (no collapsing)
pub fn group_tabs(tabs: Vec<TabInfo>) -> Vec<GroupAssignment> {
    // Step 1 — Extract domain and keywords for every tab
    let enriched: Vec<(TabInfo, Option<String>, Vec<String>)> = tabs
        .iter()
        .map(|tab| {
            let dom = tab.url.as_deref().and_then(domain::extract_domain);
            let kw = tab
                .title
                .as_deref()
                .map(keywords::extract_keywords)
                .unwrap_or_default();
            (tab.clone(), dom, kw)
        })
        .collect();

    // Step 2 — Build domain → group_name mapping
    let mut domain_groups: HashMap<String, String> = HashMap::new();
    for (_, ref dom, _) in &enriched {
        if let Some(domain) = dom {
            domain_groups
                .entry(domain.clone())
                .or_insert_with(|| domain.clone());
        }
    }

    // Step 3 — Assign groups
    let all_tabs: Vec<TabInfo> = tabs.clone();

    enriched
        .into_iter()
        .map(|(tab, domain, keywords)| {
            let group_name = if let Some(ref d) = domain {
                // Tab has a valid domain → use it as the group name
                domain_groups
                    .get(d)
                    .cloned()
                    .unwrap_or_else(|| "Other".to_string())
            } else {
                // No domain → try keyword matching
                let best = find_best_keyword_match(&keywords, &domain_groups, &all_tabs);
                best.unwrap_or_else(|| "Other".to_string())
            };

            GroupAssignment {
                tab_id: tab.id,
                group_name,
                domain,
                keywords,
            }
        })
        .collect()
}

/// Match a tab's keywords against domain groups.
///
/// For each domain group, collects all keywords from all tabs in that group.
/// Computes Jaccard similarity (intersection size / union size) between the
/// tab's keywords and the group's collective keywords.
///
/// Returns the domain (group name) whose tab-collective keywords best overlap,
/// provided similarity > 0.2. Returns `None` if no match exceeds the threshold.
fn find_best_keyword_match(
    tab_keywords: &[String],
    domain_groups: &HashMap<String, String>,
    all_tabs: &[TabInfo],
) -> Option<String> {
    if tab_keywords.is_empty() || domain_groups.is_empty() {
        return None;
    }

    let tab_keyword_set: HashSet<&str> = tab_keywords.iter().map(|s| s.as_str()).collect();

    // Pre-compute which domain each tab maps to
    let tab_domains: Vec<(i32, Option<String>)> = all_tabs
        .iter()
        .map(|tab| {
            let dom = tab
                .url
                .as_deref()
                .and_then(domain::extract_domain);
            (tab.id, dom)
        })
        .collect();

    let mut best_domain: Option<String> = None;
    let mut best_similarity: f64 = 0.0;
    let threshold: f64 = 0.2;

    for domain in domain_groups.keys() {
        // Collect all keywords from all tabs in this domain group
        let mut group_keywords: HashSet<String> = HashSet::new();
        for (tab_id, tab_dom) in &tab_domains {
            if tab_dom.as_deref() == Some(domain.as_str()) {
                // Find the tab in all_tabs by id
                if let Some(tab) = all_tabs.iter().find(|t| t.id == *tab_id) {
                    let kw = tab
                        .title
                        .as_deref()
                        .map(keywords::extract_keywords)
                        .unwrap_or_default();
                    for k in kw {
                        group_keywords.insert(k);
                    }
                }
            }
        }

        if group_keywords.is_empty() {
            continue;
        }

        // Compute Jaccard similarity
        let intersection_size = tab_keyword_set
            .iter()
            .filter(|kw| group_keywords.contains(**kw))
            .count();

        let union_size = tab_keywords.len() + group_keywords.len() - intersection_size;

        if union_size == 0 {
            continue;
        }

        let similarity = intersection_size as f64 / union_size as f64;

        if similarity > threshold && similarity > best_similarity {
            best_similarity = similarity;
            best_domain = Some(domain.clone());
        }
    }

    best_domain
}

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
    use crate::types::TabInfo;

    fn make_tab(id: i32, url: Option<&str>, title: Option<&str>) -> TabInfo {
        TabInfo {
            id,
            url: url.map(|s| s.to_string()),
            title: title.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_group_tabs_all_same_domain() {
        let tabs = vec![
            make_tab(1, Some("https://github.com/user/repo1"), Some("Repo 1")),
            make_tab(2, Some("https://github.com/user/repo2"), Some("Repo 2")),
            make_tab(3, Some("https://github.com/user/repo3"), Some("Repo 3")),
        ];
        let result = group_tabs(tabs);
        assert_eq!(result.len(), 3);
        for assignment in &result {
            assert_eq!(assignment.group_name, "github.com");
            assert_eq!(assignment.domain.as_deref(), Some("github.com"));
        }
    }

    #[test]
    fn test_group_tabs_mixed_domains() {
        let tabs = vec![
            make_tab(1, Some("https://github.com/user/repo"), Some("GitHub Repo")),
            make_tab(
                2,
                Some("https://docs.rs/oxichrome-core"),
                Some("oxichrome docs"),
            ),
            make_tab(3, Some("https://github.com/rust-lang/rust"), Some("Rust repo")),
        ];
        let result = group_tabs(tabs);
        assert_eq!(result.len(), 3);

        // Tab 1 and 3 should be github.com
        assert_eq!(result[0].group_name, "github.com");
        assert_eq!(result[2].group_name, "github.com");
        // Tab 2 should be docs.rs
        assert_eq!(result[1].group_name, "docs.rs");
    }

    #[test]
    fn test_group_tabs_no_url() {
        let tabs = vec![
            make_tab(1, Some("https://github.com"), Some("GitHub")),
            make_tab(2, None, Some("New Tab")),
        ];
        let result = group_tabs(tabs);
        assert_eq!(result.len(), 2);
        // Tab 1 → github.com
        assert_eq!(result[0].group_name, "github.com");
        assert_eq!(result[0].domain.as_deref(), Some("github.com"));
        // Tab 2 → no domain → "Other" (no keyword match with github.com since "GitHub" is boilerplate → no keywords)
        assert_eq!(result[1].group_name, "Other");
        assert_eq!(result[1].domain, None);
    }

    #[test]
    fn test_group_tabs_chrome_url() {
        let tabs = vec![
            make_tab(1, Some("https://github.com"), Some("GitHub")),
            make_tab(2, Some("chrome://extensions"), Some("Extensions")),
        ];
        let result = group_tabs(tabs);
        assert_eq!(result.len(), 2);
        // Tab 1 → github.com
        assert_eq!(result[0].group_name, "github.com");
        // Tab 2 → chrome:// → domain None → "Other"
        assert_eq!(result[1].group_name, "Other");
    }

    #[test]
    fn test_group_tabs_keyword_match() {
        // Tab with a domain "example.com" and keywords matching a github tab
        let tabs = vec![
            make_tab(
                1,
                Some("https://github.com/user/repo"),
                Some("Rust compiler improvements"),
            ),
            // Tab with no domain but keywords overlap with github tab
            make_tab(
                2,
                None,
                Some("Improvements to the Rust compiler"),
            ),
        ];
        let result = group_tabs(tabs);
        assert_eq!(result.len(), 2);
        // Tab 1 → github.com
        assert_eq!(result[0].group_name, "github.com");
        // Tab 2 has no domain, but keywords "improvements", "rust", "compiler" overlap
        // with tab 1's keywords "rust", "compiler", "improvements" → match
        assert_eq!(result[1].group_name, "github.com");
    }

    #[test]
    fn test_group_tabs_empty_input() {
        let result = group_tabs(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_group_tabs_single_tab_domain() {
        let tabs = vec![make_tab(
            1,
            Some("https://example.com/page"),
            Some("Example Page"),
        )];
        let result = group_tabs(tabs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].group_name, "example.com");
    }

    #[test]
    fn test_group_tabs_no_url_falls_to_other() {
        let tabs = vec![
            make_tab(1, None, None),
            make_tab(2, Some("about:blank"), None),
            make_tab(3, Some("data:text/plain,hello"), None),
        ];
        let result = group_tabs(tabs);
        assert_eq!(result.len(), 3);
        for assignment in &result {
            assert_eq!(assignment.group_name, "Other");
            assert_eq!(assignment.domain, None);
        }
    }

    #[test]
    fn test_find_best_keyword_match_no_match() {
        let domain_groups: HashMap<String, String> =
            [("github.com".to_string(), "github.com".to_string())]
                .iter()
                .cloned()
                .collect();
        let all_tabs = vec![make_tab(
            1,
            Some("https://github.com/user/repo"),
            Some("Rust project"),
        )];

        let result = find_best_keyword_match(
            &["python".to_string(), "django".to_string()],
            &domain_groups,
            &all_tabs,
        );
        // No overlap between ["python", "django"] and ["rust", "project"]
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_best_keyword_match_empty_tab_keywords() {
        let domain_groups: HashMap<String, String> = HashMap::new();
        let all_tabs = vec![];
        let result = find_best_keyword_match(&[], &domain_groups, &all_tabs);
        assert_eq!(result, None);
    }

    // -----------------------------------------------------------------------
    // reconcile() tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_reconcile_preserves_existing_group_id() {
        // Stored group has group_id = Some(42), fresh includes that group.
        // Result must keep group_id = Some(42).
        let stored = GroupState {
            version: 1,
            groups: vec![StoredGroup {
                name: "github.com".into(),
                keywords: vec!["rust".into()],
                created_at_ms: 1000.0,
                updated_at_ms: 1000.0,
                group_id: Some(42),
            }],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "github.com".into(),
            domain: Some("github.com".into()),
            keywords: vec!["rust".into()],
        }];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].group_id, Some(42));
    }

    #[test]
    fn test_reconcile_first_run_creates_groups() {
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
                keywords: vec!["rust".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "docs.rs".into(),
                domain: Some("docs.rs".into()),
                keywords: vec!["docs".into()],
            },
            GroupAssignment {
                tab_id: 3,
                group_name: "Other".into(),
                domain: None,
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
            }],
        };
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
                keywords: vec!["rust".into(), "compiler".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
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
            }],
        };
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
                keywords: vec![],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "youtube.com".into(),
                domain: Some("youtube.com".into()),
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
                },
                StoredGroup {
                    name: "old-domain.com".into(),
                    keywords: vec![],
                    created_at_ms: 500.0,
                    updated_at_ms: 500.0,
                    group_id: None,
                },
            ],
        };
        let fresh = vec![GroupAssignment {
            tab_id: 1,
            group_name: "github.com".into(),
            domain: Some("github.com".into()),
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
                domain: None,
                keywords: vec![],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "Other".into(),
                domain: None,
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
            }],
        };
        let fresh: Vec<GroupAssignment> = vec![];
        let result = reconcile(&fresh, &stored, 2000.0);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].name, "github.com");
    }

    // -----------------------------------------------------------------------
    // Idempotence tests — verifies that multiple runs produce stable state
    // -----------------------------------------------------------------------

    #[test]
    fn test_group_tabs_deterministic() {
        // Same tabs must always produce the same GroupAssignments
        let tabs = vec![
            make_tab(1, Some("https://github.com/user/repo1"), Some("Rust compiler updates")),
            make_tab(2, Some("https://docs.rs/oxichrome"), Some("Oxichrome documentation")),
            make_tab(3, Some("https://youtube.com/watch?v=abc"), Some("Rust tutorial video")),
            make_tab(4, None, Some("New Tab")),
        ];
        let r1 = group_tabs(tabs.clone());
        let r2 = group_tabs(tabs);
        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.tab_id, b.tab_id);
            assert_eq!(a.group_name, b.group_name);
            assert_eq!(a.domain, b.domain);
            assert_eq!(a.keywords, b.keywords);
        }
    }

    #[test]
    fn test_reconcile_two_runs_same_assignments_idempotent() {
        // Simulate two successive reconcile() calls with the same fresh assignments,
        // as would happen when the user re-opens the same tabs and re-runs grouping.
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
                keywords: vec!["rust".into(), "compiler".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
                keywords: vec!["cli".into(), "tool".into()],
            },
            GroupAssignment {
                tab_id: 3,
                group_name: "docs.rs".into(),
                domain: Some("docs.rs".into()),
                keywords: vec!["oxichrome".into(), "api".into()],
            },
            GroupAssignment {
                tab_id: 4,
                group_name: "Other".into(),
                domain: None,
                keywords: vec![],
            },
        ];

        // Run 1: fresh start with empty stored state
        let stored = GroupState {
            version: 1,
            groups: vec![],
        };
        let state1 = reconcile(&fresh, &stored, 1000.0);

        // Run 2: same fresh assignments, state from run 1 is the "stored" state
        let state2 = reconcile(&fresh, &state1, 2000.0);

        // Both runs should produce the same number of groups (no duplicates)
        assert_eq!(
            state1.groups.len(),
            state2.groups.len(),
            "Group count must not grow between runs"
        );

        // Both runs should have exactly the same set of group names
        let names1: HashSet<&str> = state1.groups.iter().map(|g| g.name.as_str()).collect();
        let names2: HashSet<&str> = state2.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names1, names2, "Group names must match between runs");

        // Verify no duplicate names within each run
        assert_eq!(names1.len(), state1.groups.len(), "Duplicates in state1");
        assert_eq!(names2.len(), state2.groups.len(), "Duplicates in state2");

        // For each group, verify structural properties
        for g1 in &state1.groups {
            let g2 = state2
                .groups
                .iter()
                .find(|g| g.name == g1.name)
                .expect("Group must exist in second run");

            // Same keywords (deterministic from same fresh assignments)
            assert_eq!(
                g1.keywords, g2.keywords,
                "Keywords for '{}' must match between runs",
                g1.name
            );

            // created_at_ms preserved from first run
            assert_eq!(
                g1.created_at_ms, g2.created_at_ms,
                "created_at_ms for '{}' must be preserved",
                g1.name
            );

            // updated_at_ms reflects the latest run
            assert_eq!(
                g2.updated_at_ms, 2000.0,
                "updated_at_ms for '{}' must be updated to second run timestamp",
                g2.name
            );
        }
    }

    #[test]
    fn test_reconcile_three_runs_no_duplicate_groups() {
        // Three successive runs with the same fresh data must never produce
        // additional groups. The group set stabilizes after the first run.
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
                keywords: vec!["rust".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "youtube.com".into(),
                domain: Some("youtube.com".into()),
                keywords: vec!["tutorial".into()],
            },
        ];

        // Run 1, 2, 3
        let state1 = reconcile(&fresh, &GroupState { version: 1, groups: vec![] }, 1000.0);
        let state2 = reconcile(&fresh, &state1, 2000.0);
        let state3 = reconcile(&fresh, &state2, 3000.0);

        // Group count must never grow beyond the number of unique domain names
        assert_eq!(state1.groups.len(), 2);
        assert_eq!(state2.groups.len(), 2, "Duplicated group after 2nd run");
        assert_eq!(state3.groups.len(), 2, "Duplicated group after 3rd run");

        // Group names must be exactly the same across all runs
        let names1: HashSet<&str> = state1.groups.iter().map(|g| g.name.as_str()).collect();
        let names3: HashSet<&str> = state3.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names1, names3);
    }

    #[test]
    fn test_reconcile_same_inputs_identical_output() {
        // Calling reconcile() twice with the exact same inputs must produce
        // byte-for-byte identical GroupState (pure function property).
        // The deterministic sorting ensures this holds regardless of HashMap iteration order.
        let fresh = vec![
            GroupAssignment {
                tab_id: 1,
                group_name: "github.com".into(),
                domain: Some("github.com".into()),
                keywords: vec!["rust".into()],
            },
            GroupAssignment {
                tab_id: 2,
                group_name: "docs.rs".into(),
                domain: Some("docs.rs".into()),
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

    #[test]
    fn test_reconcile_full_pipeline_same_tabs() {
        // Simulate the real extension flow end-to-end (without storage):
        //   1. group_tabs(tabs) → fresh assignments
        //   2. reconcile(fresh, empty, t1) → state1 (first run)
        //   3. reconcile(fresh, state1, t2) → state2 (second run with same tabs)
        //
        // Verifies that the full pipeline is structurally idempotent.
        let tabs = vec![
            make_tab(1, Some("https://github.com/rust-lang/rust"), Some("Rust compiler repository")),
            make_tab(2, Some("https://github.com/rust-lang/cargo"), Some("Cargo build tool")),
            make_tab(3, Some("https://docs.rs/oxichrome/latest"), Some("Oxichrome API docs")),
            make_tab(4, Some("https://youtube.com/watch?v=xyz"), Some("Rust async tutorial")),
            make_tab(5, None, Some("New Tab")),
        ];

        // Step 1 — group_tabs (deterministic pure function)
        let assignments = group_tabs(tabs);

        // Verify expected grouping structure (domain-based)
        let mut by_group: std::collections::HashMap<&str, Vec<&GroupAssignment>> =
            std::collections::HashMap::new();
        for a in &assignments {
            by_group.entry(&a.group_name).or_default().push(a);
        }
        // github.com should have 2 tabs
        assert_eq!(by_group.get("github.com").map(|v| v.len()), Some(2));
        // docs.rs should have 1 tab
        assert_eq!(by_group.get("docs.rs").map(|v| v.len()), Some(1));
        // youtube.com should have 1 tab
        assert_eq!(by_group.get("youtube.com").map(|v| v.len()), Some(1));
        // "Other" should have 1 tab (the None-URL tab)
        assert_eq!(by_group.get("Other").map(|v| v.len()), Some(1));

        // Step 2 — first reconcile
        let state1 = reconcile(&assignments, &GroupState { version: 1, groups: vec![] }, 1000.0);
        // 3 domain groups (github.com, docs.rs, youtube.com), excluding "Other"
        assert_eq!(state1.groups.len(), 3);

        // Step 3 — second reconcile with same assignments (simulating re-run)
        // In a real scenario, state1 would have been loaded from storage.
        let state2 = reconcile(&assignments, &state1, 2000.0);

        // Group count must not grow
        assert_eq!(
            state2.groups.len(),
            3,
            "Group count must stay stable on re-run"
        );

        // Group names must be the same set
        let expected_names: HashSet<&str> = ["github.com", "docs.rs", "youtube.com"].into();
        let got_names1: HashSet<&str> = state1.groups.iter().map(|g| g.name.as_str()).collect();
        let got_names2: HashSet<&str> = state2.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(got_names1, expected_names);
        assert_eq!(got_names2, expected_names);

        // No duplicate names in either state
        assert_eq!(got_names1.len(), state1.groups.len());
        assert_eq!(got_names2.len(), state2.groups.len());

        // Keywords for each group must be identical between runs
        for g1 in &state1.groups {
            let g2 = state2
                .groups
                .iter()
                .find(|g| g.name == g1.name)
                .expect("Group must survive re-run");
            assert_eq!(
                g1.keywords, g2.keywords,
                "Keywords for '{}' changed between runs",
                g1.name
            );
        }
    }
}
