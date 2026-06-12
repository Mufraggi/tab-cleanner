//! Semantic grouping pipeline: SML-only, no heuristic fallback.
//!
//! `run_semantic_grouping()` is the main entry point, wired from
//! `PopupCommand::RunSemanticGrouping`. It:
//! 1. Loads the sentence-transformers model + tokenizer from the Cache API
//! 2. Embeds all tabs + group themes in one batch
//! 3. Computes group anchors (theme or centroid)
//! 4. Assigns tabs to existing groups via cosine similarity (threshold 0.25, ajustable dans grouping.rs)
//! 5. Unassigned tabs go to "Other"
//! 6. Reconciles, applies Chrome groups, and persists state
//!
//! # Safety
//! Every `?` maps errors to `Err(String)`. No `unwrap()`, no panics.

use std::collections::HashMap;
use url::Url;

use crate::sml::grouping::{
    assign_tabs_semantic, group_anchor, TabId, SIMILARITY_THRESHOLD,
};
use crate::sml::model_cache::load_weights_from_cache;
use crate::sml::model_loader::{embed_cached, load_model_from_bytes};
use crate::types::{GroupAssignment, QueryAllTabs, TabInfo};

/// Embedding dimension for all-MiniLM-L6-v2.
const EMBEDDING_DIM: usize = 384;

/// Extract a clean domain from a URL (e.g., "https://www.youtube.com/watch?v=..." → "youtube.com").
/// Strips the protocol, www. prefix, and any path/query/fragment.
/// Returns an empty string if the URL is empty, invalid, or has no recognizable host.
///
/// This adds a strong domain signal to the embedding text without reintroducing
/// heuristic grouping — the grouping remains purely semantic.
fn extract_domain(url_text: &str) -> String {
    if url_text.is_empty() {
        return String::new();
    }
    match Url::parse(url_text) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => {
                if let Some(rest) = host.strip_prefix("www.") {
                    rest.to_string()
                } else {
                    host.to_string()
                }
            }
            None => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// Pure logic: build `GroupAssignment`s from pre-computed embeddings.
///
/// Takes tab embeddings, stored groups, theme embeddings, tab info and the tab→embedding
/// lookup map as input, and produces a list of group assignments WITHOUT any I/O.
///
/// Steps 9-11 of the semantic pipeline:
/// 9. Compute group anchors (theme embedding or centroid of tabs in the group)
/// 10. Assign tabs semantically via `assign_tabs_semantic`
/// 11. Map results to `GroupAssignment` — unassigned tabs get `group_name = "Other"`
///
/// This function is pure and testable with synthetic embeddings.
fn build_semantic_assignments(
    tab_embeddings: &[(TabId, Vec<f32>)],
    stored_groups: &[crate::types::StoredGroup],
    theme_embedding_map: &HashMap<String, Vec<f32>>,
    tabs: &[TabInfo],
    tab_emb_map: &HashMap<TabId, Vec<f32>>,
) -> Vec<GroupAssignment> {
    // ── 9. Compute group anchors ─────────────────────────────────────────
    let mut anchors: Vec<(String, Vec<f32>)> = Vec::new();
    for group in stored_groups {
        let theme_emb = theme_embedding_map.get(&group.name);

        let group_tab_embs: Vec<Vec<f32>> = tabs
            .iter()
            .filter(|tab| {
                group.group_id.map_or(false, |gid| tab.group_id == gid)
            })
            .filter_map(|tab| tab_emb_map.get(&tab.id).cloned())
            .collect();

        let anchor = group_anchor(group, &group_tab_embs, theme_emb);
        if let Some(a) = anchor {
            anchors.push((group.name.clone(), a));
        }
    }

    // ── 10. Assign tabs semantically ─────────────────────────────────────
    let semantic_assignments = assign_tabs_semantic(tab_embeddings, &anchors, SIMILARITY_THRESHOLD);

    // ── 11. Build GroupAssignments: SML-assigned + unassigned → "Other" ──
    let mut all_assignments: Vec<GroupAssignment> = Vec::with_capacity(tab_embeddings.len());

    for sa in &semantic_assignments {
        let group_name = sa
            .assigned_group
            .clone()
            .unwrap_or_else(|| "Other".to_string());
        all_assignments.push(GroupAssignment {
            tab_id: sa.tab_id,
            group_name,
            keywords: vec![],
        });
    }

    all_assignments
}

/// Run the full semantic grouping pipeline.
///
/// # Returns
/// `Ok(())` on success. `Err(String)` with a user-readable error message on failure.
///
/// # Errors
/// Returns `Err(...)` if:
/// - Model or tokenizer is not cached (user must click "Télécharger le modèle" first)
/// - Tokenizer JSON is not valid UTF-8
/// - Model fails to load from cached bytes
/// - Inference fails (e.g. on empty input batch)
/// - Chrome API (tabs.query, tabGroups) fails
pub async fn run_semantic_grouping() -> Result<(), String> {
    // ── 1. Load model from cache ─────────────────────────────────────────
    let model_bytes = load_weights_from_cache(crate::types::MODEL_URL)
        .await
        .map_err(|e| format!(
            "Modele non telecharge. Cliquez 'Telecharger le modele' d'abord.\nDetail: {}",
            e
        ))?;
    load_model_from_bytes(model_bytes)
        .map_err(|e| format!("Echec du chargement du modele : {}", e))?;

    // ── 2. Load tokenizer from cache ─────────────────────────────────────
    let tokenizer_bytes = load_weights_from_cache(crate::types::TOKENIZER_URL)
        .await
        .map_err(|e| format!(
            "Tokenizer non telecharge. Cliquez 'Telecharger le modele' d'abord.\nDetail: {}",
            e
        ))?;
    let tokenizer_json = String::from_utf8(tokenizer_bytes)
        .map_err(|e| format!("Tokenizer JSON invalide (pas UTF-8) : {}", e))?;

    // ── 3. Query current tabs ────────────────────────────────────────────
    let tabs: Vec<TabInfo> = oxichrome::tabs::query(&QueryAllTabs {
        current_window: Some(true),
    })
    .await
    .map_err(|e| format!("Erreur de lecture des onglets : {:?}", e))?;

    // ── 4. Load stored state ─────────────────────────────────────────────
    let stored = crate::storage::load_state().await;

    // ── 5. Build embedding texts ─────────────────────────────────────────
    // For each tab: concatenate title + domain (extracted from URL) rather than
    // the full URL. The domain adds a strong signal (e.g., "youtube.com" pulls
    // tabs toward a "Vidéos" anchor) without relying on heuristic domain rules.
    // The grouping remains purely semantic.
    let tab_texts: Vec<(TabId, String)> = tabs
        .iter()
        .map(|tab| {
            let title = tab.title.as_deref().unwrap_or("");
            let url = tab.url.as_deref().unwrap_or("");
            let domain = extract_domain(url);
            let text = format!("{} {}", title, domain);
            (tab.id, text)
        })
        .collect();

    // Collect theme texts from stored groups with non-empty themes
    // On embedde "NOM. THÈME" ensemble plutôt que le thème seul pour que le
    // nom du groupe (signal court et fort) recentre l'ancre sémantique et
    // évite la dilution du vecteur par des descriptions trop longues.
    let mut theme_texts: Vec<(String, String)> = Vec::new();
    for group in &stored.groups {
        if !group.theme.trim().is_empty() {
            theme_texts.push((group.name.clone(), group.theme.clone()));
        }
    }

    // Build ordered list: all tab texts first, then all theme texts
    let mut all_texts: Vec<String> = tab_texts.iter().map(|(_, t)| t.clone()).collect();
    // Embed "NAME. THEME" as anchor text — the group name re-centers the vector
    let theme_text_only: Vec<String> = theme_texts
        .iter()
        .map(|(name, theme)| format!("{}. {}", name, theme))
        .collect();
    let num_tabs = tab_texts.len();
    let num_themes = theme_text_only.len();
    all_texts.extend_from_slice(&theme_text_only);

    // ── 6. Embed all at once ─────────────────────────────────────────────
    let texts_json = serde_json::to_string(&all_texts)
        .map_err(|e| format!("Echec de serialisation des textes : {}", e))?;

    let flat_embeddings = embed_cached(&tokenizer_json, &texts_json)
        .map_err(|e| format!("Echec de l'inference : {}", e))?;

    // ── 7. Slice embeddings ──────────────────────────────────────────────
    let total_expected = flat_embeddings.len() / EMBEDDING_DIM;
    if total_expected < num_tabs + num_themes {
        return Err(format!(
            "Nombre d'embeddings recu ({}) inferieur au nombre attendu ({} tabs + {} themes = {})",
            total_expected,
            num_tabs,
            num_themes,
            num_tabs + num_themes
        ));
    }

    // Build tab_id → embedding map
    let mut tab_embeddings: Vec<(TabId, Vec<f32>)> = Vec::with_capacity(num_tabs);
    for i in 0..num_tabs {
        let start = i * EMBEDDING_DIM;
        let end = start + EMBEDDING_DIM;
        let emb: Vec<f32> = flat_embeddings[start..end].to_vec();
        tab_embeddings.push((tab_texts[i].0, emb));
    }

    // Build theme_name → embedding map
    let mut theme_embedding_map: HashMap<String, Vec<f32>> = HashMap::with_capacity(num_themes);
    for i in 0..num_themes {
        let start = (num_tabs + i) * EMBEDDING_DIM;
        let end = start + EMBEDDING_DIM;
        let emb: Vec<f32> = flat_embeddings[start..end].to_vec();
        theme_embedding_map.insert(theme_texts[i].0.clone(), emb);
    }

    // ── 8. Build tab_id → embedding lookup for centroid computation ──────
    let tab_emb_map: HashMap<TabId, Vec<f32>> = tab_embeddings
        .iter()
        .map(|(id, emb)| (*id, emb.clone()))
        .collect();

    // ── 9-11. Pure: compute anchors, assign tabs, build GroupAssignments ─
    let all_assignments = build_semantic_assignments(
        &tab_embeddings,
        &stored.groups,
        &theme_embedding_map,
        &tabs,
        &tab_emb_map,
    );

    // ── 12. Reconcile, apply, persist ────────────────────────────────────
    let now_ms = js_sys::Date::now();
    let updated = crate::grouping::reconcile(&all_assignments, &stored, now_ms);
    let final_state = crate::grouping::apply::apply_groups(&all_assignments, &updated).await;
    crate::storage::save_state(&final_state).await;

    // ── 13. Log summary ──────────────────────────────────────────────────
    use std::collections::HashSet;
    let group_set: HashSet<&str> = all_assignments
        .iter()
        .map(|a| a.group_name.as_str())
        .collect();
    let sml_count = all_assignments
        .iter()
        .filter(|a| a.group_name != "Other")
        .count();
    oxichrome::log!(
        "Semantic grouping complete: {} tabs → {} groups ({} SML, {} unassigned, {} persisted total)",
        all_assignments.len(),
        group_set.len(),
        sml_count,
        all_assignments.len() - sml_count,
        final_state.groups.len()
    );

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sml::grouping::{
        cosine_similarity, compute_centroid, group_anchor, assign_tab, assign_tabs_semantic,
        SIMILARITY_THRESHOLD,
    };
    use crate::types::StoredGroup;

    // ═══════════════════════════════════════════════════════════════════════
    // SML grouping tests
    // ═══════════════════════════════════════════════════════════════════════

    fn make_stored_group(
        name: &str,
        theme: &str,
        group_id: Option<i32>,
        manual: bool,
    ) -> StoredGroup {
        StoredGroup {
            name: name.to_string(),
            keywords: vec![],
            created_at_ms: 0.0,
            updated_at_ms: 0.0,
            group_id,
            display_name: None,
            theme: theme.to_string(),
            color: None,
            manual,
        }
    }

    // ── Synthetic embeddings (3-D, L2-normalised) ────────────────────────
    fn v1() -> Vec<f32> { vec![1.0, 0.0, 0.0] }
    fn v3() -> Vec<f32> { vec![0.0, 1.0, 0.0] }
    fn v4() -> Vec<f32> { vec![-1.0, 0.0, 0.0] }

    /// Test: group_anchor returns theme embedding when theme is present.
    #[test]
    fn test_anchor_theme_wins_over_centroid() {
        let group = make_stored_group("work", "research", Some(1), false);
        let tab_embs = vec![v1()]; // centroid of [v1] = v1
        let theme_emb = v3();
        let anchor = group_anchor(&group, &tab_embs, Some(&theme_emb));
        assert!(anchor.is_some());
        let a = anchor.unwrap();
        // Should be theme (v3), not centroid (v1)
        assert!((a[1] - 1.0).abs() < 1e-6, "theme anchor should be v3 (0,1,0)");
    }

    /// Test: group_anchor falls back to centroid when theme is empty.
    #[test]
    fn test_anchor_centroid_when_theme_empty() {
        let group = make_stored_group("work", "", Some(1), false);
        let tab_embs = vec![v1(), v1()]; // centroid = v1
        let anchor = group_anchor(&group, &tab_embs, None);
        assert!(anchor.is_some());
        let a = anchor.unwrap();
        assert!((a[0] - 1.0).abs() < 1e-6, "centroid should be v1 (1,0,0)");
    }

    /// Test: group_anchor returns None for empty group with empty theme.
    #[test]
    fn test_anchor_none_when_empty() {
        let group = make_stored_group("work", "", None, false);
        let tab_embs: Vec<Vec<f32>> = vec![];
        let anchor = group_anchor(&group, &tab_embs, None);
        assert!(anchor.is_none());
    }

    /// Test: group_anchor returns None when theme is present but no embedding
    /// was computed (e.g. theme text was empty string → no embedding).
    #[test]
    fn test_anchor_none_when_theme_but_no_embedding() {
        let group = make_stored_group("work", "research", None, false);
        let tab_embs: Vec<Vec<f32>> = vec![];
        // No theme embedding provided and no group tabs → None
        let anchor = group_anchor(&group, &tab_embs, None);
        assert!(anchor.is_none());
    }

    /// Test: empty anchors → all tabs unassigned.
    #[test]
    fn test_empty_anchors_all_none() {
        let tab_embs = vec![
            (1i32, v1()),
            (2i32, v3()),
        ];
        let assignments = assign_tabs_semantic(&tab_embs, &[], 0.4);
        assert_eq!(assignments.len(), 2);
        for a in &assignments {
            assert_eq!(a.assigned_group, None);
        }
    }

    /// Test: tabs assigned only when similarity > threshold.
    #[test]
    fn test_threshold_strictly_greater() {
        // v1 has cos=1.0 with itself → above threshold
        // v4 has cos=-1.0 with v1 → below threshold
        let tab_embs = vec![
            (1i32, v1()),
            (2i32, v4()),
        ];
        let anchors = vec![("group_x".to_string(), v1())];
        let assignments = assign_tabs_semantic(&tab_embs, &anchors, 0.4);
        assert_eq!(assignments[0].assigned_group, Some("group_x".to_string()));
        assert_eq!(assignments[1].assigned_group, None);
    }

    /// Test: centroid computation with multiple vectors.
    #[test]
    fn test_centroid_two_vectors_smoke() {
        let embeddings = vec![v1(), vec![0.6, 0.8, 0.0]];
        let c = compute_centroid(&embeddings);
        assert_eq!(c.len(), 3);
        // Verify unit norm
        let norm: f32 = c.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "centroid must have unit norm");
    }

    /// Test: centroid of a single vector returns the vector itself.
    #[test]
    fn test_centroid_single_vector() {
        let embeddings = vec![v1()];
        let c = compute_centroid(&embeddings);
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert!((c[1] - 0.0).abs() < 1e-6);
        assert!((c[2] - 0.0).abs() < 1e-6);
    }

    /// Test: empty centroid returns empty vec.
    #[test]
    fn test_centroid_empty() {
        let c = compute_centroid(&[]);
        assert!(c.is_empty());
    }

    /// Test: cosine similarity with identical vectors.
    #[test]
    fn test_cosine_identical_returns_one() {
        let sim = cosine_similarity(&v1(), &v1());
        assert!((sim - 1.0).abs() < 1e-6);
    }

    /// Test: assign_tab returns None when anchors are empty.
    #[test]
    fn test_assign_tab_empty_anchors() {
        let result = assign_tab(&v1(), &[], 0.4);
        assert_eq!(result, None);
    }

    /// Test: cosine similarity between orthogonal vectors is 0.
    #[test]
    fn test_cosine_orthogonal() {
        let sim = cosine_similarity(&v1(), &v3());
        assert!((sim - 0.0).abs() < 1e-6);
    }

    /// Test: the SIMILARITY_THRESHOLD constant is 0.25.
    #[test]
    fn test_similarity_threshold_value() {
        assert!((SIMILARITY_THRESHOLD - 0.25).abs() < 1e-6);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // build_semantic_assignments (pure function) tests
    // ═══════════════════════════════════════════════════════════════════════

    /// Test: a tab with embedding close to a group's theme → assigned to that group,
    /// a tab far from all → "Other".
    #[test]
    fn test_build_assignments_theme_anchor_assigns_close_tab() {
        let group = make_stored_group("dev", "coding", None, false);
        let tab_embeddings = vec![
            (1i32, v1()), // close to dev theme (v1)
            (2i32, v4()), // opposite → far from dev
        ];
        let mut theme_emb_map = HashMap::new();
        theme_emb_map.insert("dev".to_string(), v1());
        let tabs: Vec<TabInfo> = vec![]; // no tabs currently in Chrome groups
        let tab_emb_map: HashMap<TabId, Vec<f32>> = tab_embeddings
            .iter()
            .map(|(id, emb)| (*id, emb.clone()))
            .collect();

        let assignments = build_semantic_assignments(
            &tab_embeddings,
            &[group],
            &theme_emb_map,
            &tabs,
            &tab_emb_map,
        );
        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].tab_id, 1);
        assert_eq!(assignments[0].group_name, "dev");
        assert_eq!(assignments[1].tab_id, 2);
        assert_eq!(assignments[1].group_name, "Other");
    }

    /// Test: no stored groups → all tabs assigned to "Other".
    #[test]
    fn test_build_assignments_no_groups_all_other() {
        let tab_embeddings = vec![
            (10i32, v1()),
            (20i32, v3()),
        ];
        let theme_emb_map: HashMap<String, Vec<f32>> = HashMap::new();
        let tabs: Vec<TabInfo> = vec![];
        let tab_emb_map: HashMap<TabId, Vec<f32>> = tab_embeddings
            .iter()
            .map(|(id, emb)| (*id, emb.clone()))
            .collect();

        let assignments = build_semantic_assignments(
            &tab_embeddings,
            &[],
            &theme_emb_map,
            &tabs,
            &tab_emb_map,
        );
        assert_eq!(assignments.len(), 2);
        for a in &assignments {
            assert_eq!(a.group_name, "Other");
        }
    }

    /// Test: empty input → empty output.
    #[test]
    fn test_build_assignments_empty_yields_empty() {
        let assignments = build_semantic_assignments(
            &[],
            &[],
            &HashMap::new(),
            &[],
            &HashMap::new(),
        );
        assert!(assignments.is_empty());
    }

    /// Test: group with tabs in a Chrome group but no theme → centroid fallback used.
    #[test]
    fn test_build_assignments_centroid_fallback() {
        let group = make_stored_group("g1", "", Some(1), false);
        let tab_embeddings = vec![
            (100i32, v1()), // centroid of [v1, v6] ≈ [0.894, 0.447, 0] → cos 0.894 with v1
        ];
        // Two tabs already in the Chrome group (group_id=1)
        let tabs = vec![
            TabInfo { id: 101, url: None, title: None, group_id: 1 },
            TabInfo { id: 102, url: None, title: None, group_id: 1 },
        ];
        // Their embeddings → centroid fallback
        let mut tab_emb_map = HashMap::new();
        tab_emb_map.insert(101i32, v4()); // v4 = [-1, 0, 0]
        tab_emb_map.insert(102i32, v3()); // v3 = [0, 1, 0]
        // centroid of [v4, v3] = normalize([-1, 1, 0]) = [-0.707, 0.707, 0]
        // cos(v1, centroid) = 1*-0.707 + 0*0.707 + 0*0 = -0.707 < threshold

        let theme_emb_map: HashMap<String, Vec<f32>> = HashMap::new();

        let assignments = build_semantic_assignments(
            &tab_embeddings,
            &[group],
            &theme_emb_map,
            &tabs,
            &tab_emb_map,
        );
        assert_eq!(assignments.len(), 1);
        // Tab 100 (v1) should be "Other" — its cos with centroid (-0.707) is below threshold
        assert_eq!(assignments[0].group_name, "Other");
    }

    /// Test: assignment keys must be empty.
    #[test]
    fn test_build_assignments_no_keywords() {
        let group = make_stored_group("docs", "documentation", None, false);
        let tab_embeddings = vec![(5i32, v1())];
        let mut theme_emb_map = HashMap::new();
        theme_emb_map.insert("docs".to_string(), v1());
        let tabs: Vec<TabInfo> = vec![];
        let tab_emb_map: HashMap<TabId, Vec<f32>> = HashMap::new();

        let assignments = build_semantic_assignments(
            &tab_embeddings,
            &[group],
            &theme_emb_map,
            &tabs,
            &tab_emb_map,
        );
        assert_eq!(assignments.len(), 1);
        assert!(assignments[0].keywords.is_empty(), "keywords must be empty (semantic grouping does not extract keywords)");
    }
}
