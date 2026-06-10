# Implementation Plan — Phase 3: Persistence of Group State

## Goal

Persist group state across extension runs using `storage::get`/`set` so that re-running the grouping logic never duplicates a group and reuses known groups for tabs from unchanged domains.

---

## Design Decisions (explicit, to guide implementation)

### 1. Stable group identity = domain (group_name)
Since grouping is domain-based, a domain maps 1:1 to a group. The group name (e.g. `"github.com"`) IS the stable identifier. No UUID generation needed, no `js_sys` dependency.

### 2. Schema

```rust
/// A known group persisted across runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredGroup {
    /// Domain-derived group name — also the stable identifier. e.g. "github.com".
    pub name: String,
    /// Keywords extracted from tabs currently in this group. Recomputed each run.
    pub keywords: Vec<String>,
    /// Timestamps (ms since epoch, from js_sys::Date::now()).
    pub created_at_ms: f64,
    pub updated_at_ms: f64,
}

/// Top-level persistence payload stored under GROUP_STATE_KEY.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupState {
    /// Schema version for future migrations. v1 for phase 3.
    pub version: u32,
    pub groups: Vec<StoredGroup>,
}

/// Storage key — stable across runs. Value: serialized GroupState.
pub const GROUP_STATE_KEY: &str = "tab_cleanner_group_state";
```

### 3. "Other" is never persisted
The `"Other"` catch-all group is transient per-run. Only domain-based groups (with a real URL) are stored.

### 4. `GroupAssignment` stays unchanged
The existing struct does not gain a `group_id` field. Phase 3 is a storage concern only; consumers can look up stable IDs from `GroupState` if needed later.

### 5. Keywords are recomputed (not merged) each run
For a given stored group, keywords are rebuilt from the fresh assignments belonging to that group. This keeps them deterministic and representative of current tabs. Dedup, cap at 10.

### 6. Groups are never deleted
Stored groups persist even if no current tabs belong to them (tabs may return later). Storage growth is bounded by unique domains ever seen — acceptable for now.

### 7. Idempotence guarantee
`group_tabs()` is deterministic (same tabs → same group names). `reconcile()` matches by group name (domain). Running twice with the same tabs: first run creates stored groups, second run finds existing groups by name → no duplicates.

### 8. Timestamps via js_sys
`created_at_ms` and `updated_at_ms` use `js_sys::Date::now()` (available through oxichrome's dependency tree; add `js-sys` to `Cargo.toml` if not already transitively available).

---

## Tasks

### 1. Add `js-sys` dependency to `Cargo.toml`
   - **File:** `Cargo.toml`
   - **Changes:** Add `js-sys = "0.3"` under `[dependencies]`
   - **Why:** Needed for `js_sys::Date::now()` to generate timestamps in `StoredGroup`
   - **Acceptance:** `cargo check` resolves `js_sys` without errors

### 2. Add `StoredGroup`, `GroupState`, and `GROUP_STATE_KEY` to `src/types.rs`
   - **File:** `src/types.rs`
   - **Changes:** Append the following after the existing `GroupAssignment` struct:

   ```rust
   /// A known group persisted across runs.
   /// The `name` field is the domain (e.g. "github.com") and also serves as the stable identifier.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct StoredGroup {
       pub name: String,
       pub keywords: Vec<String>,
       pub created_at_ms: f64,
       pub updated_at_ms: f64,
   }

   /// Top-level persistence payload stored under GROUP_STATE_KEY.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct GroupState {
       pub version: u32,
       pub groups: Vec<StoredGroup>,
   }

   /// Storage key for the group state in chrome.storage.local.
   pub const GROUP_STATE_KEY: &str = "tab_cleanner_group_state";
   ```

   - **Acceptance:** `cargo check` passes; types are `Serialize`/`Deserialize`-compatible

### 3. Create `src/storage/mod.rs` — module declaration
   - **File:** `src/storage/mod.rs` (new)
   - **Changes:**
     - `pub mod group_state;`
     - Re-export: `pub use group_state::{load_state, save_state};`
   - **Acceptance:** `cargo check` resolves the module

### 4. Create `src/storage/group_state.rs` — persistence functions
   - **File:** `src/storage/group_state.rs` (new)
   - **Changes:** Write two public async functions using `oxichrome::storage::{get, set}`:

   ```rust
   use oxichrome::storage;
   use crate::types::{GroupState, GROUP_STATE_KEY};

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
   ```

   - Note: `save_state` is fire-and-forget (logs on error in a later polish phase if needed). `load_state` flattens both `Result::Err` and `None` into a fresh default state.
   - **Acceptance:** `cargo check` passes; `oxichrome::storage::get`/`set` resolve

### 5. Add `reconcile()` function to `src/grouping/mod.rs`
   - **File:** `src/grouping/mod.rs`
   - **Changes:** Add the following public function after `group_tabs()`:

   ```rust
   use crate::types::{GroupAssignment, GroupState, StoredGroup};
   use std::collections::HashSet;

   /// Reconcile fresh group assignments with persisted state to produce
   /// an updated GroupState ready for storage.
   ///
   /// Rules (idempotent):
   /// - For each unique group_name in fresh assignments (except "Other"),
   ///   ensure a StoredGroup with that name exists.
   /// - If a StoredGroup already exists, reuse it (update keywords + timestamp).
   /// - If not, create a new one.
   /// - Groups not present in fresh assignments are left untouched (never deleted).
   /// - "Other" is never persisted.
   pub fn reconcile(fresh: &[GroupAssignment], stored: &GroupState) -> GroupState {
       let now_ms = js_sys::Date::now();

       // Build name → StoredGroup map from existing state
       let mut existing: std::collections::HashMap<String, StoredGroup> = stored
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
               // Cap at 10
               group.keywords = kw_set.into_iter().take(10).collect();
               group.updated_at_ms = now_ms;
           }
           updated_groups.push(group);
       }

       GroupState {
           version: 1,
           groups: updated_groups,
       }
   }
   ```

   - **Important:** Add `use crate::types::{GroupState, StoredGroup};` to the existing imports at the top of `src/grouping/mod.rs`.
   - **Important:** `reconcile` takes `&GroupState` (not `Option<&GroupState>`) — defaulting belongs in the orchestration layer, not in `reconcile`.
   - **Acceptance:** `cargo check` passes

### 6. Wire persistence into `src/lib.rs`
   - **File:** `src/lib.rs`
   - **Changes:**
     1. Add `mod storage;` at the top (alongside existing `mod grouping; mod types;`)
     2. Rewrite `run_grouping()` to integrate storage:

     ```rust
     /// Run grouping and persist the resulting state to storage.
     /// Idempotent: re-running with the same tabs reuses existing groups.
     pub async fn run_grouping() -> Vec<GroupAssignment> {
         // 1. Query all current tabs
         let tabs: Vec<TabInfo> = tabs::query(&QueryAllTabs {}).await.unwrap_or_default();

         // 2. Classify tabs (pure, deterministic)
         let assignments = grouping::group_tabs(tabs);

         // 3. Load persisted state (empty on first run)
         let stored = crate::storage::load_state().await;

         // 4. Reconcile fresh assignments with stored state
         let updated = grouping::reconcile(&assignments, &stored);

         // 5. Save updated state (fire-and-forget)
         crate::storage::save_state(&updated).await;

         // 6. Log summary
         let group_set: HashSet<&str> = assignments.iter()
             .map(|a| a.group_name.as_str())
             .collect();
         oxichrome::log!(
             "Grouping complete: {} tabs → {} groups ({} persisted total)",
             assignments.len(),
             group_set.len(),
             updated.groups.len()
         );

         assignments
     }
     ```

     Use `crate::storage::load_state()` and `crate::storage::save_state()` to avoid any ambiguity with `oxichrome::storage` from the prelude.

   - **Acceptance:** `cargo check` passes

### 7. Write unit tests for `reconcile()` in `src/grouping/mod.rs`
   - **File:** `src/grouping/mod.rs`
   - **Changes:** Add 6 test functions in the existing `#[cfg(test)] mod tests` block:

   **Test 7a — First run (empty stored state) creates groups:**
   ```rust
   #[test]
   fn test_reconcile_first_run_creates_groups() {
       let fresh = vec![
           GroupAssignment { tab_id: 1, group_name: "github.com".into(), domain: Some("github.com".into()), keywords: vec!["rust".into()] },
           GroupAssignment { tab_id: 2, group_name: "docs.rs".into(), domain: Some("docs.rs".into()), keywords: vec!["docs".into()] },
           GroupAssignment { tab_id: 3, group_name: "Other".into(), domain: None, keywords: vec![] },
       ];
       let stored = GroupState { version: 1, groups: vec![] };
       let result = reconcile(&fresh, &stored);
       assert_eq!(result.groups.len(), 2); // "Other" not persisted
       let names: Vec<&str> = result.groups.iter().map(|g| g.name.as_str()).collect();
       assert!(names.contains(&"github.com"));
       assert!(names.contains(&"docs.rs"));
   }
   ```

   **Test 7b — Second run reuses existing groups (no duplicates):**
   ```rust
   #[test]
   fn test_reconcile_no_duplicate_on_rerun() {
       let stored = GroupState {
           version: 1,
           groups: vec![
               StoredGroup { name: "github.com".into(), keywords: vec!["rust".into()], created_at_ms: 1000.0, updated_at_ms: 1000.0 },
           ],
       };
       let fresh = vec![
           GroupAssignment { tab_id: 1, group_name: "github.com".into(), domain: Some("github.com".into()), keywords: vec!["rust".into(), "compiler".into()] },
           GroupAssignment { tab_id: 2, group_name: "github.com".into(), domain: Some("github.com".into()), keywords: vec!["cli".into()] },
       ];
       let result = reconcile(&fresh, &stored);
       assert_eq!(result.groups.len(), 1); // Still only one github.com
       let g = &result.groups[0];
       assert_eq!(g.name, "github.com");
       assert_eq!(g.created_at_ms, 1000.0); // original creation time preserved
       assert!(g.updated_at_ms > 1000.0);   // updated timestamp
       assert!(g.keywords.contains(&"rust".to_string()));
       assert!(g.keywords.contains(&"compiler".to_string()));
       assert!(g.keywords.contains(&"cli".to_string()));
   }
   ```

   **Test 7c — New domain on second run adds a group:**
   ```rust
   #[test]
   fn test_reconcile_adds_new_domain() {
       let stored = GroupState {
           version: 1,
           groups: vec![
               StoredGroup { name: "github.com".into(), keywords: vec![], created_at_ms: 1000.0, updated_at_ms: 1000.0 },
           ],
       };
       let fresh = vec![
           GroupAssignment { tab_id: 1, group_name: "github.com".into(), domain: Some("github.com".into()), keywords: vec![] },
           GroupAssignment { tab_id: 2, group_name: "youtube.com".into(), domain: Some("youtube.com".into()), keywords: vec!["video".into()] },
       ];
       let result = reconcile(&fresh, &stored);
       assert_eq!(result.groups.len(), 2);
   }
   ```

   **Test 7d — Orphaned groups (no current tabs) are preserved:**
   ```rust
   #[test]
   fn test_reconcile_preserves_orphaned_groups() {
       let stored = GroupState {
           version: 1,
           groups: vec![
               StoredGroup { name: "github.com".into(), keywords: vec!["rust".into()], created_at_ms: 1000.0, updated_at_ms: 1000.0 },
               StoredGroup { name: "old-domain.com".into(), keywords: vec![], created_at_ms: 500.0, updated_at_ms: 500.0 },
           ],
       };
       let fresh = vec![
           GroupAssignment { tab_id: 1, group_name: "github.com".into(), domain: Some("github.com".into()), keywords: vec!["rust".into()] },
       ];
       let result = reconcile(&fresh, &stored);
       assert_eq!(result.groups.len(), 2); // old-domain.com still there
   }
   ```

   **Test 7e — All "Other" means no persisted groups:**
   ```rust
   #[test]
   fn test_reconcile_all_other_creates_no_groups() {
       let fresh = vec![
           GroupAssignment { tab_id: 1, group_name: "Other".into(), domain: None, keywords: vec![] },
           GroupAssignment { tab_id: 2, group_name: "Other".into(), domain: None, keywords: vec![] },
       ];
       let stored = GroupState { version: 1, groups: vec![] };
       let result = reconcile(&fresh, &stored);
       assert_eq!(result.groups.len(), 0);
   }
   ```

   **Test 7f — Empty input preserves existing state:**
   ```rust
   #[test]
   fn test_reconcile_empty_fresh_preserves_existing() {
       let stored = GroupState {
           version: 1,
           groups: vec![
               StoredGroup { name: "github.com".into(), keywords: vec!["rust".into()], created_at_ms: 1000.0, updated_at_ms: 1000.0 },
           ],
       };
       let fresh: Vec<GroupAssignment> = vec![];
       let result = reconcile(&fresh, &stored);
       assert_eq!(result.groups.len(), 1);
       assert_eq!(result.groups[0].name, "github.com");
   }
   ```

   - **Acceptance:** `cargo test` passes all 6 new tests + all 38 existing tests (44 total)

### 8. Add structural test for GroupState serialization in `src/storage/group_state.rs`
   - **File:** `src/storage/group_state.rs`
   - **Changes:** Add a `#[cfg(test)]` block with a roundtrip test:

   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn test_group_state_roundtrip() {
           let state = GroupState {
               version: 1,
               groups: vec![
                   StoredGroup {
                       name: "github.com".into(),
                       keywords: vec!["rust".into(), "compiler".into()],
                       created_at_ms: 1718100000000.0,
                       updated_at_ms: 1718100000000.0,
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
   ```

   - **Risk:** If `serde_json` is not already in the dependency tree, this test won't compile. In that case, add `serde_json = "1"` as `[dev-dependencies]` in `Cargo.toml`, or skip the roundtrip and test construction only (no serde_json needed).
   - **Acceptance:** `cargo test` passes

---

## Files to Modify

| File | Changes |
|---|---|
| `Cargo.toml` | Add `js-sys = "0.3"` dependency; optionally add `serde_json = "1"` under `[dev-dependencies]` |
| `src/types.rs` | Append `StoredGroup`, `GroupState` structs, `GROUP_STATE_KEY` constant (after existing `GroupAssignment`) |
| `src/grouping/mod.rs` | Add `reconcile()` public function + 6 tests; update imports to include `GroupState`, `StoredGroup` |
| `src/lib.rs` | Add `mod storage;`, rewrite `run_grouping()` to load → reconcile → save |

## New Files

| File | Purpose |
|---|---|
| `src/storage/mod.rs` | Module declaration + re-exports `load_state`, `save_state` |
| `src/storage/group_state.rs` | `load_state()`, `save_state()`, structural roundtrip test |

---

## Dependencies

- **Task 1** (`Cargo.toml`) — no dependencies; do first
- **Task 2** (`types.rs`) — no dependencies; can be done in parallel with Task 1
- **Task 3** (`storage/mod.rs`) — depends on Task 2 (imports types)
- **Task 4** (`storage/group_state.rs`) — depends on Tasks 2 and 3 (imports types + module exists)
- **Task 5** (`reconcile()` in grouping/mod.rs) — depends on Task 2 (imports `GroupState`, `StoredGroup`)
- **Task 6** (`lib.rs` wiring) — depends on Tasks 4 and 5 (imports storage and reconcile)
- **Task 7** (tests for reconcile) — depends on Task 5 (function must exist)
- **Task 8** (test for default state) — depends on Task 4 (types + state module must exist)

**Recommended order:** 1 → 2 → 3 → 4 + 5 (parallel) → 6 → 7 + 8 (parallel)

---

## Risks

1. **`js_sys::Date::now()` availability** — `js-sys` may already be a transitive dependency through `oxichrome` / `wasm-bindgen`. Verify with `cargo tree -i js-sys`. If already available, skip Task 1. If `js-sys` is blocked or `js_sys::Date::now()` isn't available, fallback: use `0.0` as timestamp (acceptable for phase 3).

2. **Naming conflict with `oxichrome::storage`** — `src/lib.rs` has `use oxichrome::prelude::*` which brings `oxichrome::storage` into scope. Adding `mod storage;` creates `crate::storage`. These are different paths — use `crate::storage::load_state()` explicitly to avoid any potential ambiguity. If `cargo check` complains, rename the local module to `mod state_persistence;`.

3. **`storage::get` returning `Err` on first install** — `load_state()` defaults to empty `GroupState` on both `Err` and `None`, which is the correct behavior for key-not-found.

4. **`serde_json` not in dependencies** — The roundtrip test in Task 8 uses `serde_json`. If not already available, add `serde_json = "1"` under `[dev-dependencies]` in `Cargo.toml`.

5. **Storage quota** — `chrome.storage.local` has 10 MB limit. At ~200 bytes per `StoredGroup`, ~50,000 domains are needed to hit the limit. Not a concern for phase 3.

6. **Idempotence verification** — The key guarantee is that `reconcile()` never creates duplicate `StoredGroup` entries for the same `name`. This is enforced by `HashMap::entry().or_insert_with()` which only inserts on absent keys. Test 7b explicitly verifies this.

---

## What is NOT in this phase

- **No Chrome native group creation** — `chrome.tabs.group()` and `chrome.tabGroups.*` remain out of scope.
- **No tabGroups FFI** — `src/ffi/` directory is not created.
- **No event-driven re-grouping** — `tabs::on_updated`, `tabs::on_activated` are not wired. Grouping only runs at startup.
- **No popup / message passing** — `runtime::on_message` is not implemented.
- **No group deletion or manual management** — Stored groups are append-only.
- **No keyword matching against stored groups** — `find_best_keyword_match` only matches against domain groups present in the *current* tab set, not against all stored groups. This is intentional: orphaned stored groups should not attract new tab assignments.
