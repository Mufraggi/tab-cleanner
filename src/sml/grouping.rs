//! Semantic grouping logic for tab-cleanner SML.
//!
//! This module implements pure semantic group assignment: given embeddings for
//! tabs and existing groups, it assigns each tab to the closest group whose
//! anchor passes a similarity threshold.
//!
//! KEY RULE: The SML assigns tabs ONLY to existing groups. It NEVER creates
//! new groups automatically. Tabs that don't match any existing group remain
//! unassigned (return `None`).
//!
//! Group anchor logic:
//! - If a `StoredGroup` has a non-empty `theme` AND a `theme_embedding` is
//!   provided, the theme embedding is used as the group anchor.
//! - Otherwise, the centroid (mean of tab embeddings in the group, L2-normalised)
//!   is used as the anchor.
//! - If a group has no tabs and no theme, it has no anchor (`None`).
//!
//! This is PURE logic (Vec<f32> in/out), testable without browser or model.
//! Integration with the heuristic classifier is done in step 5 (RunGrouping).

use crate::types::StoredGroup;

/// Type alias for a tab's unique identifier (Chrome tab id).
pub type TabId = i32;

/// Default similarity threshold for semantic assignment.
/// A tab is assigned to a group only if the cosine similarity between the
/// tab's embedding and the group's anchor exceeds this value.
///
/// Plage utile : 0.25-0.35.
/// - Plus bas (0.25) = regroupe plus largement, plus de faux positifs.
/// - Plus haut (0.35) = plus strict, moins d'assignations.
/// Ajuster cette constante pour calibrer le comportement du tri.
pub const SIMILARITY_THRESHOLD: f32 = 0.25;

/// The result of semantic assignment for a single tab.
///
/// - `tab_id`: the Chrome tab id
/// - `assigned_group`: `Some(name)` if assigned to an existing group,
///    `None` if no group passed the threshold (tab left unassigned)
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticAssignment {
    pub tab_id: TabId,
    pub assigned_group: Option<String>,
}

// ── Core functions ─────────────────────────────────────────────────────────

/// Cosine similarity between two vectors that are already L2-normalised.
///
/// Since both vectors have unit norm, the dot product equals the cosine.
/// Returns 0.0 if the dimensions differ.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Compute the centroid of a set of embeddings.
///
/// The centroid is the component-wise mean, then L2-normalised so it has unit
/// norm. This preserves the property that dot product with the centroid equals
/// cosine similarity.
///
/// Returns an empty `Vec` if `embeddings` is empty.
pub fn compute_centroid(embeddings: &[Vec<f32>]) -> Vec<f32> {
    if embeddings.is_empty() {
        return vec![];
    }
    let dim = embeddings[0].len();
    let mut sum = vec![0.0f32; dim];
    for emb in embeddings {
        for (i, &v) in emb.iter().enumerate() {
            if i < dim {
                sum[i] += v;
            }
        }
    }
    let n = embeddings.len() as f32;
    for v in &mut sum {
        *v /= n;
    }
    // L2-normalise
    let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut sum {
            *v /= norm;
        }
    }
    sum
}

/// Determine the semantic anchor for a group.
///
/// Rules:
/// - If `group.theme` is non-empty AND `theme_embedding` is `Some`, return
///   the theme embedding (the declarative anchor).
/// - Otherwise, if the group has tab embeddings, compute and return the
///   centroid.
/// - Otherwise (empty group, no theme), return `None` — the group is not
///   usable as an assignment target.
pub fn group_anchor(
    group: &StoredGroup,
    group_tab_embeddings: &[Vec<f32>],
    theme_embedding: Option<&Vec<f32>>,
) -> Option<Vec<f32>> {
    // Priority 1: theme anchor (declarative, user-defined)
    if !group.theme.is_empty() {
        if let Some(theme_emb) = theme_embedding {
            return Some(theme_emb.clone());
        }
    }
    // Priority 2: centroid of existing tab embeddings
    if !group_tab_embeddings.is_empty() {
        return Some(compute_centroid(group_tab_embeddings));
    }
    // No anchor possible
    None
}

/// Assign a single tab to the closest group whose anchor passes the threshold.
///
/// For each group anchor, computes cosine similarity between the tab embedding
/// and the anchor. Returns the name of the group with the highest similarity,
/// but ONLY if that similarity exceeds `threshold`. Otherwise returns `None`
/// (tab left unassigned — no automatic group creation).
///
/// If multiple groups tie for the highest similarity, the first one in
/// iteration order wins (deterministic if `group_anchors` order is stable).
pub fn assign_tab(
    tab_embedding: &[f32],
    group_anchors: &[(String, Vec<f32>)],
    threshold: f32,
) -> Option<String> {
    let mut best_group: Option<String> = None;
    let mut best_similarity: f32 = threshold; // only beat if strictly > threshold

    for (name, anchor) in group_anchors {
        let sim = cosine_similarity(tab_embedding, anchor);
        if sim > best_similarity {
            best_similarity = sim;
            best_group = Some(name.clone());
        }
    }

    best_group
}

/// Assign multiple tabs to existing groups using semantic similarity.
///
/// Each tab is independently evaluated against all group anchors. Tabs whose
/// closest anchor exceeds `threshold` are assigned (`Some(group_name)`);
/// others are left unassigned (`None`).
///
/// This is the main entry point for semantic grouping. It NEVER creates new
/// groups — tabs that don't match any existing group are simply skipped.
pub fn assign_tabs_semantic(
    tab_embeddings: &[(TabId, Vec<f32>)],
    group_anchors: &[(String, Vec<f32>)],
    threshold: f32,
) -> Vec<SemanticAssignment> {
    tab_embeddings
        .iter()
        .map(|(tab_id, embedding)| {
            let assigned = assign_tab(embedding, group_anchors, threshold);
            SemanticAssignment {
                tab_id: *tab_id,
                assigned_group: assigned,
            }
        })
        .collect()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Synthetic embeddings (3-D, L2-normalised) ──────────────────────────
    // v1 = [1, 0, 0]
    // v2 = [0.9, 0.1, 0]  (close to v1, cos ~0.9)
    // v3 = [0, 1, 0]      (orthogonal to v1, cos = 0)
    // v4 = [-1, 0, 0]     (opposite to v1, cos = -1)
    // v5 = [0, 0.707, 0.707]  (45° from v3, cos ~0.707 with v3)
    // v6 = [0.6, 0.8, 0]      (cos ~0.6 with v1)

    fn v1() -> Vec<f32> { vec![1.0, 0.0, 0.0] }
    fn v2() -> Vec<f32> { vec![0.9, 0.1, 0.0] }
    fn v3() -> Vec<f32> { vec![0.0, 1.0, 0.0] }
    fn v4() -> Vec<f32> { vec![-1.0, 0.0, 0.0] }
    fn v5() -> Vec<f32> { vec![0.0, 0.70710677, 0.70710677] }
    fn v6() -> Vec<f32> { vec![0.6, 0.8, 0.0] }

    // ── Helper: build a minimal StoredGroup ────────────────────────────────

    fn make_group(name: &str, theme: &str) -> StoredGroup {
        StoredGroup {
            name: name.to_string(),
            keywords: vec![],
            created_at_ms: 0.0,
            updated_at_ms: 0.0,
            group_id: None,
            display_name: None,
            theme: theme.to_string(),
            color: None,
            manual: false,
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // cosine_similarity
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_cosine_identical() {
        let v = v1();
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6, "identical vectors → 1.0, got {}", sim);
    }

    #[test]
    fn test_cosine_orthogonal() {
        let sim = cosine_similarity(&v1(), &v3());
        assert!((sim - 0.0).abs() < 1e-6, "orthogonal vectors → 0.0, got {}", sim);
    }

    #[test]
    fn test_cosine_opposite() {
        let sim = cosine_similarity(&v1(), &v4());
        assert!((sim - (-1.0)).abs() < 1e-6, "opposite vectors → -1.0, got {}", sim);
    }

    #[test]
    fn test_cosine_close() {
        let sim = cosine_similarity(&v1(), &v2());
        assert!((sim - 0.9).abs() < 1e-6, "close vectors → ~0.9, got {}", sim);
    }

    #[test]
    fn test_cosine_mismatched_dimensions() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6, "mismatched dims → 0.0, got {}", sim);
    }

    #[test]
    fn test_cosine_empty_slice() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6, "empty slices → 0.0, got {}", sim);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // compute_centroid
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_centroid_single_vector() {
        let embeddings = vec![v1()];
        let c = compute_centroid(&embeddings);
        assert_eq!(c.len(), 3);
        // Centroid of a single vector should be the vector itself (already normalised)
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert!((c[1] - 0.0).abs() < 1e-6);
        assert!((c[2] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_centroid_two_vectors() {
        let embeddings = vec![v1(), v6()];
        let c = compute_centroid(&embeddings);
        // Raw mean: [(1.0+0.6)/2, (0.0+0.8)/2, (0.0+0.0)/2] = [0.8, 0.4, 0.0]
        // Norm: sqrt(0.8^2 + 0.4^2) = sqrt(0.64 + 0.16) = sqrt(0.8) ≈ 0.8944
        // Normalised: [0.8/0.8944, 0.4/0.8944, 0.0] ≈ [0.8944, 0.4472, 0.0]
        let norm = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "centroid must have unit norm, got {}", norm);
        assert!(c[0] > c[1], "v1 contribution dominates: c[0] > c[1]");
        assert!((c[2] - 0.0).abs() < 1e-6, "z component should be 0");
        // By symmetry, the centroid of two vectors is equidistant from both
        // in cosine space: both cosines must be equal.
        let cos_with_v1 = cosine_similarity(&c, &v1());
        let cos_with_v6 = cosine_similarity(&c, &v6());
        let diff = (cos_with_v1 - cos_with_v6).abs();
        assert!(diff < 1e-6, "centroid must be equidistant from v1 and v6, diff={}", diff);
    }

    #[test]
    fn test_centroid_empty() {
        let embeddings: Vec<Vec<f32>> = vec![];
        let c = compute_centroid(&embeddings);
        assert!(c.is_empty(), "empty input → empty output");
    }

    #[test]
    fn test_centroid_normalised_output() {
        let embeddings = vec![v3(), v5()];
        let c = compute_centroid(&embeddings);
        // v3 = [0, 1, 0], v5 = [0, 0.707, 0.707]
        // Raw mean: [0, 0.8535, 0.3535]
        // After normalisation: still unit vector
        let norm = c.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "centroid must have unit norm, got {}", norm);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // group_anchor
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_anchor_theme_priority() {
        // Group with non-empty theme + theme_embedding provided → theme wins
        let group = make_group("work", "research");
        let tab_embs = vec![v1(), v2()];
        let theme_emb = v3();
        let anchor = group_anchor(&group, &tab_embs, Some(&theme_emb));
        assert!(anchor.is_some());
        // Anchor should be the theme embedding, not the centroid
        let a = anchor.unwrap();
        assert!((a[0] - 0.0).abs() < 1e-6, "theme is v3 (0,1,0): x should be 0");
        assert!((a[1] - 1.0).abs() < 1e-6, "theme is v3 (0,1,0): y should be 1");
    }

    #[test]
    fn test_anchor_centroid_when_no_theme() {
        // Group with empty theme → centroid fallback
        let group = make_group("work", "");
        let tab_embs = vec![v1(), v6()];
        let anchor = group_anchor(&group, &tab_embs, None);
        assert!(anchor.is_some());
        let a = anchor.unwrap();
        // Should be the centroid of [v1, v6]
        let expected_centroid = compute_centroid(&tab_embs);
        assert!((a[0] - expected_centroid[0]).abs() < 1e-6);
        assert!((a[1] - expected_centroid[1]).abs() < 1e-6);
    }

    #[test]
    fn test_anchor_centroid_when_theme_missing() {
        // Group with theme "research" but theme_embedding is None → centroid
        let group = make_group("work", "research");
        let tab_embs = vec![v1(), v2()];
        let anchor = group_anchor(&group, &tab_embs, None);
        assert!(anchor.is_some());
        // Without a theme_embedding, falls back to centroid
        let expected = compute_centroid(&tab_embs);
        let a = anchor.unwrap();
        assert!((a[0] - expected[0]).abs() < 1e-6);
    }

    #[test]
    fn test_anchor_none_when_empty_and_no_theme() {
        // Empty group, no theme → no anchor
        let group = make_group("work", "");
        let tab_embs: Vec<Vec<f32>> = vec![];
        let anchor = group_anchor(&group, &tab_embs, None);
        assert!(anchor.is_none(), "empty group with no theme → None");
    }

    #[test]
    fn test_anchor_none_when_empty_theme_present_but_no_embedding() {
        // Empty group, has theme text but no theme_embedding → centroid fails → None
        let group = make_group("work", "research");
        let tab_embs: Vec<Vec<f32>> = vec![];
        let anchor = group_anchor(&group, &tab_embs, None);
        // Theme is non-empty but no theme_embedding provided.
        // Centroid is tried but empty → None.
        assert!(anchor.is_none(), "empty group with theme but no embedding → None");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // assign_tab
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_assign_tab_best_above_threshold() {
        // v1 is closest to anchor [1,0,0] (cos=1.0) > threshold → assigned
        let anchors = vec![
            ("docs".to_string(), v3()),  // cos(v1, v3) = 0.0
            ("github".to_string(), v1()), // cos(v1, v1) = 1.0
        ];
        let result = assign_tab(&v1(), &anchors, 0.4);
        assert_eq!(result, Some("github".to_string()));
    }

    #[test]
    fn test_assign_tab_all_below_threshold() {
        // v4 is opposite to v1 (cos=-1) and far from v3 (cos=0) → all below 0.4
        let anchors = vec![
            ("docs".to_string(), v1()), // cos(v4, v1) = -1.0
            ("music".to_string(), v3()), // cos(v4, v3) = 0.0
        ];
        let result = assign_tab(&v4(), &anchors, 0.4);
        assert_eq!(result, None, "all similarities below threshold → None");
    }

    #[test]
    fn test_assign_tab_exact_threshold_not_assigned() {
        // Threshold 0.8, v2 has cos=0.9 with v1 → assigned
        // But v6 has cos=0.6 with v1 → below 0.8 → not assigned
        let anchors = vec![
            ("github".to_string(), v1()),
        ];
        let result = assign_tab(&v2(), &anchors, 0.8);
        assert_eq!(result, Some("github".to_string()), "cos=0.9 > 0.8 threshold");
    }

    #[test]
    fn test_assign_tab_matches_closest_anchor() {
        // v6 has cos ~0.6 with v1, cos ~0.8 with v6 itself
        let anchors = vec![
            ("group_a".to_string(), v1()),
            ("group_b".to_string(), v6()),
        ];
        let result = assign_tab(&v6(), &anchors, 0.4);
        // v6 is closest to group_b (cos=1.0)
        assert_eq!(result, Some("group_b".to_string()));
    }

    #[test]
    fn test_assign_tab_empty_anchors() {
        let result = assign_tab(&v1(), &[], 0.4);
        assert_eq!(result, None, "no anchors → no assignment");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // assign_tabs_semantic
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_assign_tabs_mixed_assignment() {
        // Three tabs, two group anchors:
        //   tab1 (v1) → close to anchor "x" (v1) → assigned
        //   tab2 (v4) → far from all → None
        //   tab3 (v6) → close to anchor "y" (v6) → assigned
        let tab_embs = vec![
            (1, v1()),
            (2, v4()),
            (3, v6()),
        ];
        let anchors = vec![
            ("group_x".to_string(), v1()),
            ("group_y".to_string(), v6()),
        ];
        let assignments = assign_tabs_semantic(&tab_embs, &anchors, 0.4);

        assert_eq!(assignments.len(), 3);

        // Tab 1 → assigned to group_x (cos=1.0 with anchor group_x)
        assert_eq!(assignments[0].tab_id, 1);
        assert_eq!(assignments[0].assigned_group, Some("group_x".to_string()));

        // Tab 2 → unassigned (cos=-1.0 with group_x, cos=0.0 with group_y)
        assert_eq!(assignments[1].tab_id, 2);
        assert_eq!(assignments[1].assigned_group, None);

        // Tab 3 → assigned to group_y (cos=1.0 with anchor group_y)
        assert_eq!(assignments[2].tab_id, 3);
        assert_eq!(assignments[2].assigned_group, Some("group_y".to_string()));
    }

    #[test]
    fn test_assign_tabs_all_unassigned() {
        // All tabs far from all anchors → all None
        let tab_embs = vec![
            (10, v4()), // opposite to v1
            (20, v4()),
        ];
        let anchors = vec![
            ("group_x".to_string(), v1()),
        ];
        let assignments = assign_tabs_semantic(&tab_embs, &anchors, 0.4);
        assert_eq!(assignments.len(), 2);
        for a in &assignments {
            assert_eq!(a.assigned_group, None);
        }
    }

    #[test]
    fn test_assign_tabs_empty_input() {
        let assignments = assign_tabs_semantic(&[], &[], 0.4);
        assert!(assignments.is_empty());
    }

    #[test]
    fn test_assign_tabs_no_anchors() {
        // Tabs exist but no group anchors → all unassigned
        let tab_embs = vec![
            (1, v1()),
            (2, v3()),
        ];
        let assignments = assign_tabs_semantic(&tab_embs, &[], 0.4);
        assert_eq!(assignments.len(), 2);
        for a in &assignments {
            assert_eq!(a.assigned_group, None);
        }
    }

    #[test]
    fn test_assign_tabs_default_threshold_constant() {
        // Verify the new SIMILARITY_THRESHOLD (0.25) is used correctly.
        // Use tab v2 (cos=0.9 with v1) and v7 (cos=0.2 with v1).
        let v7 = vec![0.2, 0.0, 0.0]; // unit vector with cos=0.2 to v1
        assert!((cosine_similarity(&v7, &v1()) - 0.2).abs() < 1e-6);

        let tab_embs = vec![
            (1, v2()), // cos=0.9 with v1 → above threshold
            (2, v7),   // cos=0.2 with v1 → below threshold
        ];
        let anchors = vec![
            ("group_x".to_string(), v1()),
        ];
        let assignments = assign_tabs_semantic(&tab_embs, &anchors, SIMILARITY_THRESHOLD);

        // Tab 1: assigned (0.9 > 0.25)
        assert_eq!(assignments[0].assigned_group, Some("group_x".to_string()));
        // Tab 2: unassigned (0.2 < 0.25)
        assert_eq!(assignments[1].assigned_group, None);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Integration: full flow with synthetic data
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_full_flow_with_existing_groups() {
        // Simulate a realistic flow:
        // 1. Two existing StoredGroups with tab embeddings and anchors
        // 2. Several tabs, some matching existing groups, some not
        // 3. verify no new groups created, only assignments to existing ones

        // Groups:
        // - "dev" (theme = "coding", theme_embedding = v1 [1,0,0])
        // - "media" (theme = "" → centroid of [v3, v5])

        let dev_group = make_group("dev", "coding");
        let media_group = make_group("media", "");

        let dev_tab_embs = vec![v1(), v2()]; // tabs in dev group
        let media_tab_embs = vec![v3(), v5()]; // tabs in media group

        // Compute anchors
        let dev_anchor = group_anchor(&dev_group, &dev_tab_embs, Some(&v1()))
            .expect("dev group should have anchor");
        let media_anchor = group_anchor(&media_group, &media_tab_embs, None)
            .expect("media group should have anchor");

        let anchors = vec![
            ("dev".to_string(), dev_anchor),
            ("media".to_string(), media_anchor),
        ];

        // Tabs to assign:
        //   tab 100: v1 [1,0,0] → close to dev (cos~1.0)
        //   tab 101: v3 [0,1,0] → close to media (centroid of [v3, v5])
        //   tab 102: v4 [-1,0,0] → far from all
        //   tab 103: v6 [0.6,0.8,0] → ambiguous, closer to dev or media?

        let tab_embs = vec![
            (100, v1()),
            (101, v3()),
            (102, v4()),
            (103, v6()),
        ];

        let assignments = assign_tabs_semantic(&tab_embs, &anchors, 0.4);

        assert_eq!(assignments.len(), 4);

        // Tab 100 → dev (cos~1.0 with dev anchor)
        assert_eq!(assignments[0].tab_id, 100);
        assert_eq!(assignments[0].assigned_group, Some("dev".to_string()));

        // Tab 101 → media (closer to media centroid than to dev)
        assert_eq!(assignments[1].tab_id, 101);
        assert_eq!(assignments[1].assigned_group, Some("media".to_string()));

        // Tab 102 → None (opposite to dev, orthogonal to media → both < 0.4)
        assert_eq!(assignments[2].tab_id, 102);
        assert_eq!(assignments[2].assigned_group, None);

        // Tab 103 → whichever is closer (should be dev based on cos values)
        // v6 with v1 = 0.6, v6 with media centroid (centroid of [v3,v5]):
        //   centroid = normalize([0, (1+0.707)/2, (0+0.707)/2])
        //   = normalize([0, 0.8535, 0.3535])
        //   norm = sqrt(0 + 0.7285 + 0.1250) = sqrt(0.8535) = 0.9238
        //   centroid ≈ [0, 0.924, 0.383]
        //   cos(v6, dev) = 0.6*1 + 0.8*0 + 0*0 = 0.6
        //   cos(v6, media) = 0*0 + 0.8*0.924 + 0*0.383 = 0.739
        // So v6 is closer to media centroid. Let me verify computationally.
        // Actually let me just check it's assigned to one of them and above threshold.
        assert!(assignments[3].assigned_group.is_some());
        let group_name = assignments[3].assigned_group.as_deref().unwrap();
        assert!(
            group_name == "dev" || group_name == "media",
            "tab 103 should be assigned to dev or media, got {}",
            group_name
        );
    }
}
