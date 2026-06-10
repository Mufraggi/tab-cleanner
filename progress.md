# Phase 4c+4d — Chrome Native Tab Group Orchestration

## Status ✅ Complete

### New file
- `src/grouping/apply.rs` — `apply_groups()` + `pick_color()` + tests

### Modified files
- `src/types.rs` — added `current_window: Option<bool>` to `QueryAllTabs`
- `src/grouping/mod.rs` — added `pub mod apply;`
- `src/lib.rs` — added `tabGroups` permission; calls `apply_groups()` between reconcile and save_state; passes `current_window: Some(true)` to tabs query

### What was implemented
1. **`pick_color(name)`** — deterministic DJB2 hash → one of 8 Chrome colours (skipping grey). 5 tests covering determinism, no-grey, and reachability of all colours.
2. **`apply_groups(assignments, state)`** — async, catch-and-continue, no unwraps:
   - Builds group_name→tab_ids map (skips "Other")
   - For groups with stored `group_id`: tries to add tabs to existing group; on failure (invalid id), logs, clears id, recreates as new group
   - For groups without `group_id`: creates new Chrome group, captures id, applies title+colour
   - Safety net: handles groups not in state (shouldn't happen after reconcile)
   - Ungroups all "Other" tabs (safe no-op for ungrouped tabs)
   - Returns updated `GroupState` with populated `group_id`s
3. **lib.rs integration**: `run_grouping()` calls `apply_groups` between `reconcile` and `save_state`

### Test count
53 original + 5 new = **58 passed, 0 failed**

### Verification
- `cargo build` — success
- `cargo test` — 58/58 pass
- Zero `unwrap()` calls in Chrome code paths
- All errors caught, logged, and continued
