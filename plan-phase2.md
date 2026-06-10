# Implementation Plan — Phase 2: Heuristic Tab Grouping Logic

## Goal

Implement a **pure classification engine** that reads all open tabs, groups them by domain + title keywords, and produces serializable `GroupAssignment` records — with no native Chrome group creation, no ML, and no side effects beyond logging.

---

## Prerequisites

**Oxichrome version bump:** `Cargo.toml` currently pins `oxichrome = "0.1"`. The context research states this should be `"0.2"`. Before writing any code, verify whether `0.2` is published on crates.io. If not, use `"0.1"` (the `tabs::query` and `storage::*` signatures are identical in practice, as confirmed by research).

**Permissions:** The `#[extension]` macro needs `"tabs"` added to the permissions array so `tabs::query` works at runtime. Without it, the call will fail silently in Chrome.

---

## Tasks

### 1. Update `Cargo.toml` — Bump oxichrome version & add `url` crate for domain parsing
   - **File:** `Cargo.toml`
   - **Changes:**
     - Change `oxichrome = { version = "0.1" }` → `oxichrome = { version = "0.2" }` (or keep `"0.1"` if `0.2` not published — verify first)
     - Add `url = "2"` dependency for robust URL parsing (`url::Url` handles edge cases the `tld` crate doesn't)
     - Ensure `serde = { version = "1", features = ["derive"] }` exists (already present)
   - **Acceptance:** `cargo check` passes with the new dependencies

### 2. Add `"tabs"` to permissions in `#[extension]` macro
   - **File:** `src/lib.rs`
   - **Changes:** In the `#[oxichrome::extension(...)]` attribute, change `permissions = ["storage"]` to `permissions = ["storage", "tabs"]`
   - **Acceptance:** Rebuild produces `manifest.json` with both `"storage"` and `"tabs"` in the permissions array

### 3. Define data types in `src/types.rs`
   - **File:** `src/types.rs` (new file)
   - **Changes:** Define the core data structures:

   ```rust
   use serde::{Deserialize, Serialize};

   /// What we query Chrome for — minimal fields needed for grouping.
   /// 'allow(dead_code)' because `title` is only used for keyword extraction
   /// and `url` is only consumed by the domain extractor.
   #[derive(Debug, Clone, Deserialize)]
   #[serde(rename_all = "camelCase")]
   pub struct TabInfo {
       pub id: i32,
       pub url: Option<String>,    // None for new-tab page, chrome://, etc.
       pub title: Option<String>,  // None for tabs without a title
   }

   /// Input to tabs::query. `{}` returns all tabs in the current window.
   #[derive(Debug, Serialize)]
   #[serde(rename_all = "camelCase")]
   pub struct QueryAllTabs {}

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
   ```

   - **Acceptance:** `cargo check` passes; types derive Serialize/Deserialize without errors

### 4. Implement domain extraction in `src/grouping/domain.rs`
   - **File:** `src/grouping/domain.rs` (new file)
   - **Changes:** Write the domain extraction logic:

   ```rust
   /// Extract a cleaned domain from a URL string.
   ///
   /// Returns `None` for:
   ///   - Empty / missing URLs
   ///   - chrome://, chrome-extension://, about:, file:, data:, etc. (non-web schemes)
   ///   - Malformed URLs that url::Url cannot parse
   ///
   /// Returns `Some(domain)` with `www.` stripped for web URLs.
   pub fn extract_domain(raw_url: &str) -> Option<String>;
   ```

   - **Implementation notes:**
     - Use `url::Url::parse(raw_url)` — returns `Err` for malformed URLs → `None`
     - Check `url.scheme()` — only accept `"http"` and `"https"`; all others (`chrome`, `chrome-extension`, `about`, `file`, `data`, `moz-extension`, etc.) → `None`
     - Get `url.host_str()` — the host portion (e.g. `"www.github.com"`, `"docs.rs"`, `"192.168.1.1"`)
     - Strip leading `"www."` prefix: `host.strip_prefix("www.").unwrap_or(host)`
     - Return `Some(domain.to_lowercase())`
   - **Acceptance:** Unit tests covering: `https://www.github.com/repos` → `"github.com"`, `chrome://extensions` → `None`, `about:blank` → `None`, empty string → `None`, `http://192.168.1.1` → `"192.168.1.1"`, URL with no dots like `http://localhost:3000` → `"localhost"`

### 5. Implement keyword extraction in `src/grouping/keywords.rs`
   - **File:** `src/grouping/keywords.rs` (new file)
   - **Changes:** Write keyword extraction logic:

   ```rust
   /// Extract meaningful keywords from a tab title.
   ///
   /// Returns an empty `Vec` for missing / empty titles.
   /// Keywords are lowercase, deduplicated, and filtered to remove
   /// boilerplate separators and very short noise words.
   pub fn extract_keywords(title: &str) -> Vec<String>;
   ```

   - **Algorithm:**
     1. If title is empty/whitespace → return `vec![]`
     2. Split on common title separators: `" - "`, `" – "`, `" | "`, `" · "`, `" — "`, and also `' '` (space)
        - Flatten: for each separator, split the title, then split each part by spaces
        - This handles titles like `"Pull Requests · hugomufraggi/tab-cleanner · GitHub"`
     3. Filter tokens:
        - Remove tokens shorter than 3 characters (noise: "to", "a", "of", "in", etc.)
        - Remove tokens that are purely numeric
        - Remove common boilerplate words: `"github"`, `"youtube"`, `"google"`, `"docs"`, `"wiki"`, `"page"`, `"tab"`, `"new"`, `"chrome"`, `"mozilla"`, `"firefox"`, `"edge"`
        - Lowercase everything
     4. Deduplicate (preserving order)
     5. Return up to 5 keywords (cap to avoid noise dominance)
   - **Acceptance:** Unit tests: `"Build software better, together"` → `["build", "software", "better", "together"]`, `"YouTube"` → `[]` (all tokens too short / boilerplate), `"Pull Requests · hugomufraggi/tab-cleanner · GitHub"` → `["pull", "requests", "hugomufraggi", "cleanner"]`, empty title → `[]`, `"  "` → `[]`

### 6. Implement core grouping function in `src/grouping/mod.rs`
   - **File:** `src/grouping/mod.rs` (new file)
   - **Changes:** Orchestrate domain + keyword grouping:

   ```rust
   mod domain;
   mod keywords;

   use crate::types::{GroupAssignment, TabInfo};
   use std::collections::HashMap;

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
   pub fn group_tabs(tabs: Vec<TabInfo>) -> Vec<GroupAssignment>;
   ```

   - **Detailed algorithm:**
     ```
     INPUT: Vec<TabInfo>

     Step 1 — Extract domain and keywords for every tab:
       For each tab:
         domain = extract_domain(tab.url)
         keywords = extract_keywords(tab.title)

     Step 2 — Build domain → group_name mapping:
       domain_groups: HashMap<String, String>  // domain → group name
       For all tabs where domain.is_some():
         domain_groups.insert(domain, domain.to_string())
         // First tab with this domain determines group name.
         // All subsequent tabs with same domain get same group_name.

     Step 3 — Assign groups:
       assignments: Vec<GroupAssignment>

       For each tab:
         If domain.is_some() AND domain is in domain_groups:
           → GroupAssignment { group_name = domain, domain = Some(domain), ... }

         Else (no domain, e.g. chrome:// or empty URL):
           → Try keyword matching:
               best_match = find_best_keyword_match(tab.keywords, domain_groups)
               If best_match found:
                 → group_name = that domain's group name
               Else:
                 → group_name = "Other"
           → GroupAssignment { group_name, domain = None, ... }

     Step 4 — Return assignments (one per input tab, same order as input)
     ```

   - **Keyword matching helper** (private):
     ```rust
     /// Match a tab's keywords against domain groups.
     /// Returns the domain whose tab-collective keywords best overlap.
     fn find_best_keyword_match(
         tab_keywords: &[String],
         domain_groups: &HashMap<String, String>,
         all_tabs: &[TabInfo],
     ) -> Option<String>;
     ```
     - For each domain group, collect all keywords from all tabs in that group
     - Compute Jaccard similarity (intersection size / union size) between the tab's keywords and the group's collective keywords
     - If similarity > 0.2 (threshold), return that domain; otherwise None
     - This is deliberately simple — no ML, pure set overlap

   - **Acceptance:**
     - Integration test with 4 tabs:
       - Tab A: `https://github.com/hugomufraggi/tab-cleanner` title `"GitHub - hugomufraggi/tab-cleanner"`
       - Tab B: `https://github.com/rust-lang/rust` title `"GitHub - rust-lang/rust"`
       - Tab C: `https://docs.rs/oxichrome-core` title `"oxichrome_core - Rust"`
       - Tab D: `chrome://extensions` title `"Extensions"`
       - Expect: A+B → group `"github.com"`, C → group `"docs.rs"`, D → group `"Other"`

### 7. Wire grouping into `lib.rs` without breaking the existing skeleton
   - **File:** `src/lib.rs`
   - **Changes:** Add a `module` declaration and an async function that can be called from `start()` or an event handler:

   ```rust
   mod types;
   mod grouping;

   // ...

   /// Entry point for the grouping feature. Logs the result for now.
   /// In a later phase this will also persist to storage and create native groups.
   pub async fn run_grouping() -> Vec<GroupAssignment> {
       let tabs: Vec<TabInfo> = tabs::query(&QueryAllTabs {}).await.unwrap_or_default();
       let assignments = grouping::group_tabs(tabs);
       oxichrome::log!("Grouping complete: {} tabs → {} groups",
           assignments.len(),
           assignments.iter().map(|a| &a.group_name).collect::<HashSet<_>>().len()
       );
       assignments
   }
   ```

   - **Also add** a call in `start()`:
     ```rust
     #[oxichrome::background]
     async fn start() {
         oxichrome::log!("Tab Cleanner started!");
         let _ = run_grouping().await; // fire and forget for now
     }
     ```
   - **Existing code preserved:** The `Extension` struct, `start()`, and `handle_install()` all remain. Only the `permissions` array and the body of `start()` change.
   - **Acceptance:** `cargo check` passes; extension builds; background log shows grouping results when loaded in Chrome.

### 8. Write unit tests for all pure functions
   - **Tests for `src/grouping/domain.rs`:**
     - Test cases as listed in Task 4
     - Add `#[cfg(test)] mod tests` block with `#[test]` functions
   - **Tests for `src/grouping/keywords.rs`:**
     - Test cases as listed in Task 5
     - Include edge case: title with only boilerplate words → empty vec
     - Include edge case: very long title truncated to 5 keywords
   - **Tests for `src/grouping/mod.rs`:**
     - `test_group_tabs_all_same_domain()` — 3 tabs on github.com → all group "github.com"
     - `test_group_tabs_mixed_domains()` — 2 github.com + 1 docs.rs → 2 groups
     - `test_group_tabs_no_url()` — tab with `url = None` → group "Other"
     - `test_group_tabs_chrome_url()` — tab with `url = "chrome://extensions"` → "Other"
     - `test_group_tabs_keyword_match()` — tab without domain but keywords matching an existing group
     - `test_group_tabs_empty_input()` — empty Vec → empty Vec
   - **Acceptance:** `cargo test` passes all tests

---

## Files to Modify

| File | What changes |
|---|---|
| `Cargo.toml` | Bump `oxichrome` to `"0.2"` (verify first); add `url = "2"` |
| `src/lib.rs` | Add `"tabs"` to permissions; add `mod types; mod grouping;`; add `run_grouping()` function; call it from `start()` |

## New Files

| File | Purpose |
|---|---|
| `src/types.rs` | `TabInfo`, `QueryAllTabs`, `GroupAssignment` structs |
| `src/grouping/mod.rs` | `group_tabs()` + `find_best_keyword_match()` helper + module declarations |
| `src/grouping/domain.rs` | `extract_domain()` — URL → Option\<String\> |
| `src/grouping/keywords.rs` | `extract_keywords()` — title → Vec\<String\> |

---

## Dependencies

- **Task 3 (types)** must be done before **Task 4, 5, 6** (they all import from `types`)
- **Task 1 (Cargo.toml)** and **Task 2 (lib.rs permissions)** can be done any time before **Task 7** (wiring lib.rs)
- **Tasks 4 and 5** are independent of each other — can be done in parallel
- **Task 6** depends on **Tasks 4 and 5** (imports both modules)
- **Task 7** depends on **Tasks 3 and 6** (imports types and grouping)
- **Task 8** can be done alongside each task or all at the end

---

## Risks

1. **`oxichrome = "0.2"` may not be published on crates.io yet.**
   - Mitigation: Check `cargo search oxichrome` first. If only `0.1.x` exists, keep `"0.1"`. The `tabs::query` API signature is the same. If even `0.1` doesn't work, the URL parsing and grouping logic is still pure Rust and testable without Chrome.

2. **`tabs::query` may fail at runtime even with the `"tabs"` permission.**
   - In MV3, `chrome.tabs.query` does not need host permissions for basic tab info (url, title, id). But `url` will only be populated if the extension has the `<all_urls>` host permission or the `"tabs"` permission. The `"tabs"` permission alone should be sufficient for `id`, `url`, `title` on all tabs.
   - Mitigation: Don't add host permissions now; if URLs come back empty at runtime, the plan should be revisited.

3. **`url::Url::parse` panics on some inputs.**
   - It does not panic — it returns `Result`. We handle `Err` → `None`. No risk.

4. **Keyword extraction is language-dependent.**
   - The boilerplate word list is English-only. Non-English titles will produce more keywords (which is fine — they'll still group). Not a risk for phase 2, but document it.

5. **Deterministic grouping may produce different results if titles change.**
   - This is expected behavior. Persistence of group state (phase 3) will handle reassignment across sessions.

---

## What is NOT in this phase

- **No Chrome group creation:** `chrome.tabs.group()` and `chrome.tabGroups.*` are out of scope. This is pure classification.
- **No storage persistence:** `storage::get`/`set` are not used. Results are logged only.
- **No event listeners:** `tabs::on_updated`, `tabs::on_activated` are not wired. Grouping only runs once at startup.
- **No popup / message passing:** `runtime::on_message` is not implemented.
- **No FFI for tabGroups:** The `src/ffi/` directory is not created. Phase 3+ concern.
