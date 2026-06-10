# Research: Oxichrome v0.2 API (Signatures, Macros, FFI, tabGroups)

## Summary

Oxichrome v0.2 wraps Chrome Extension APIs in type-safe async Rust through five proc macros and three API modules (`tabs`, `storage`, `runtime`). The `#[extension]` macro's `permissions` array passes strings verbatim into `manifest.json` — so `"tabGroups"` **can** be added there and will generate correctly. However, Oxichrome v0.2 provides **no Rust bindings** for `chrome.tabGroups`; you must write custom `#[wasm_bindgen] extern "C"` FFI block(s) for each `tabGroups` method you need.

---

## Findings

### 1. `tabs::query` — Generic, serde-driven query returning `Vec<T>`

**Exact signature** (from `oxichrome-core/src/tabs.rs`):

```rust
pub async fn query<Q: Serialize, T: DeserializeOwned>(
    query_info: &Q,
) -> Result<Vec<T>>
```

- `Q` is any struct that derives `Serialize`. Fields correspond to Chrome's `queryInfo` object (`active`, `currentWindow`, `url`, etc.).
- `T` is any struct that derives `Deserialize`. The returned `Vec<T>` contains the matched tabs with the fields you care about.
- Internally serializes `query_info` via `serde_wasm_bindgen::to_value()`, calls `chrome.tabs.query()`, and deserializes the result array.

**Usage from docs:**
```rust
#[derive(Serialize)]
struct Query { active: bool }

#[derive(Deserialize)]
struct Tab { id: i32, url: String }

let tabs: Vec<Tab> = oxichrome::tabs::query(&Query { active: true }).await?;
```

**Source:** [oxichrome-core/src/tabs.rs (lines 9-15)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-core/src/tabs.rs), [Docs — tabs::query on docs.rs](https://docs.rs/oxichrome-core/latest/oxichrome_core/tabs/fn.query.html)

---

### 2. `storage::get` — Returns `Option<T>`, `None` on absent key

**Exact signature** (from `oxichrome-core/src/storage.rs`):

```rust
pub async fn get<T: DeserializeOwned>(key: &str) -> Result<Option<T>>
```

- Takes a single string key (not a JS array).
- Uses `js_sys::Reflect::get` on the result object to extract the value for that key.
- Returns `Ok(None)` when the value is `undefined` or `null` — this is the key-absent case.
- Deserializes via `serde_wasm_bindgen::from_value()`.

**Source:** [oxichrome-core/src/storage.rs (lines 9-23)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-core/src/storage.rs), [Docs — storage::get on docs.rs](https://docs.rs/oxichrome-core/latest/oxichrome_core/storage/fn.get.html)

---

### 3. `storage::set` — Wraps a single key-value pair

**Exact signature** (from `oxichrome-core/src/storage.rs`):

```rust
pub async fn set<T: Serialize>(key: &str, value: &T) -> Result<()>
```

- Creates a JS object `{ [key]: value }` via `js_sys::Object::new()` + `Reflect::set`.
- Calls `chrome.storage.local.set()` with that object.
- Value must implement `Serialize`; any serde-compatible type works (primitives, structs, `Vec`, `HashMap`, etc.).

**Source:** [oxichrome-core/src/storage.rs (lines 25-34)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-core/src/storage.rs), [Docs — storage::set on docs.rs](https://docs.rs/oxichrome-core/latest/oxichrome_core/storage/fn.set.html)

---

### 4. `#[wasm_bindgen] extern "C"` — The FFI pattern for Chrome JS APIs

The definitive pattern lives in `oxichrome-core/src/js_bridge.rs`. It uses `#[wasm_bindgen] extern "C"` blocks with `js_namespace` and `js_name` attributes to declare JS function signatures:

**Async API call pattern:**
```rust
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = query)]
    pub fn chrome_tabs_query(query_info: &JsValue) -> js_sys::Promise;
}
```

- `js_namespace` maps the JS property chain (`chrome.tabs` → `["chrome", "tabs"]`).
- `js_name` overrides the Rust function name to match the JS method name.
- Returns `js_sys::Promise`, which is resolved with `JsFuture::from(promise).await?` in the higher-level wrapper.
- All parameters are `&JsValue` to go through `serde_wasm_bindgen` serialization at the call site.

**Event listener pattern:**
```rust
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabs", "onUpdated"], js_name = addListener)]
    pub fn chrome_tabs_on_updated_add_listener(
        callback: &Closure<dyn FnMut(JsValue, JsValue, JsValue)>,
    );
}
```

- Takes `&Closure<dyn FnMut(...)>` as the callback.
- The `#[on]` macro calls `closure.forget()` to prevent the closure from being garbage-collected (essential for service worker lifetime).

**Custom FFI pattern (user-side) — opaque JS type:**
```rust
#[wasm_bindgen]
extern "C" {
    type EyeDropper;                          // opaque JS type
    #[wasm_bindgen(constructor)]              // calls `new EyeDropper()`
    fn new() -> EyeDropper;
    #[wasm_bindgen(method)]                   // calls `.open()` on instance
    fn open(this: &EyeDropper) -> js_sys::Promise;
}
```

This pattern can be copied for any unwrapped Chrome API (including `tabGroups`).

**Source:** [oxichrome-core/src/js_bridge.rs](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-core/src/js_bridge.rs), [Color Picker example](https://github.com/0xsouravm/oxichrome/blob/main/examples/color-picker/src/lib.rs)

---

### 5. `#[extension]` macro — Generates `manifest.json`; permissions pass through verbatim

The macro is applied to a struct:

```rust
#[oxichrome::extension(
    name = "My Extension",       // required
    version = "1.0.0",           // required
    description = "...",          // optional
    permissions = ["storage", "tabs", "tabGroups"]  // optional, string array
)]
struct MyExtension;
```

**What happens at build time:**
1. The proc macro (`oxichrome-macros/src/codegen/extension.rs`) parses the args and generates a `__oxichrome_meta` module with `const PERMISSIONS: &[&str]`.
2. The build tool (`oxichrome-build`) runs `syn`-based source parsing (`source_parser.rs`) that walks the AST, extracts `permissions` strings.
3. `manifest.rs` places them directly into `manifest.json` under `"permissions": [...]`.

**Supported args** (from `parse.rs`): `name`, `version`, `description`, `permissions`. No `host_permissions`, `optional_permissions`, or `optional_host_permissions` in v0.2.

**Source:** [extension.rs codegen](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-macros/src/codegen/extension.rs), [parse.rs — ExtensionArgs](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-macros/src/parse.rs), [source_parser.rs](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-build/src/source_parser.rs), [manifest.rs](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-build/src/manifest.rs)

---

### 6. `#[background]` macro — Async service worker entry point

```rust
#[oxichrome::background]
async fn start() {
    oxichrome::log!("Service worker alive!");
}
```

**Expands to:**
```rust
#[wasm_bindgen]
pub fn __oxichrome_bg_start() {
    spawn_local(async { start().await; });
}
```

- Named export (not `#[wasm_bindgen(start)]`) — prevents background init from running in popup/options pages that share the same Wasm binary.
- The generated `background.js` calls init functions in order: register event listeners first, then call the background function.

**Source:** [Docs — #[background]](https://oxichrome.dev/docs), [README](https://github.com/0xsouravm/oxichrome)

---

### 7. `#[on]` macro — Chrome event listener registration

```rust
#[oxichrome::on(runtime::on_installed)]
async fn handle_install(details: JsValue) {
    // ...
}
```

**Supported events (v0.2):**
| Event path | Chrome API |
|---|---|
| `runtime::on_installed` | `chrome.runtime.onInstalled` |
| `runtime::on_message` | `chrome.runtime.onMessage` |
| `storage::on_changed` | `chrome.storage.onChanged` |
| `tabs::on_updated` | `chrome.tabs.onUpdated` |
| `tabs::on_activated` | `chrome.tabs.onActivated` |

**What the macro generates** (from `event_handler.rs`):
1. Keeps the original async function.
2. Generates a `#[wasm_bindgen]` register function (`__oxichrome_register_{fn_name}`) that creates a `Closure` wrapping the async fn with `spawn_local`.
3. Calls the corresponding `js_bridge::*_add_listener` function.
4. Calls `closure.forget()` to prevent GC.

**Note:** There is **no `tabs::on_created` event** in v0.2. If you need it, you'd write custom FFI.

**Source:** [event_handler.rs codegen](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-macros/src/codegen/event_handler.rs), [Docs — #[on]](https://oxichrome.dev/docs)

---

### 8. `tabGroups` permission — Passes through macro; API needs custom FFI

**Permission in manifest:** YES, adding `"tabGroups"` to the `permissions` array in `#[extension]` works. The macro passes strings verbatim. Example:

```rust
#[oxichrome::extension(
    name = "Tab Cleaner",
    version = "0.1.0",
    permissions = ["storage", "tabs", "tabGroups"]
)]
struct TabCleaner;
```

This generates:
```json
{
  "permissions": ["storage", "tabs", "tabGroups"]
}
```

**Rust API bindings:** NO — Oxichrome v0.2 does **not** wrap `chrome.tabGroups`. You must write custom FFI following the EyeDropper pattern. Example for `tabGroups.query`:

```rust
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabGroups"], js_name = query)]
    fn chrome_tab_groups_query(query_info: &JsValue) -> js_sys::Promise;
}
```

Then wrap it in a type-safe async Rust function, following the same pattern used in `tabs.rs` and `storage.rs`.

**Relevant Chrome docs:** [chrome.tabGroups API](https://developer.chrome.com/docs/extensions/reference/api/tabGroups) — available since Chrome 137+.

**Source:** [manifest.rs (permissions field)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-build/src/manifest.rs), [Color Picker FFI example](https://github.com/0xsouravm/oxichrome/blob/main/examples/color-picker/src/lib.rs), [Chrome tabGroups API](https://developer.chrome.com/docs/extensions/reference/api/tabGroups)

---

## Sources

- **Kept:** [Oxichrome Docs (oxichrome.dev/docs)](https://oxichrome.dev/docs) — Primary documentation with all macro usage, API examples, build pipeline.
- **Kept:** [oxichrome-core/src/tabs.rs (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-core/src/tabs.rs) — Exact `tabs::query` signature and implementation.
- **Kept:** [oxichrome-core/src/storage.rs (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-core/src/storage.rs) — Exact `storage::get`/`set` signatures and implementation.
- **Kept:** [oxichrome-core/src/js_bridge.rs (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-core/src/js_bridge.rs) — Canonical `#[wasm_bindgen] extern "C"` FFI pattern for all Chrome APIs.
- **Kept:** [oxichrome-macros/src/codegen/event_handler.rs (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-macros/src/codegen/event_handler.rs) — `#[on]` macro expansion showing Closure + forget() pattern.
- **Kept:** [oxichrome-macros/src/parse.rs (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-macros/src/parse.rs) — Confirms `ExtensionArgs` only accepts `name`, `version`, `description`, `permissions`.
- **Kept:** [oxichrome-build/src/manifest.rs (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-build/src/manifest.rs) — Confirms permissions pass through verbatim; no `host_permissions` or `optional_permissions` support.
- **Kept:** [oxichrome-build/src/source_parser.rs (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/oxichrome-build/src/source_parser.rs) — AST visitor extracting metadata; confirms exactly which attrs are parsed.
- **Kept:** [Color Picker example (GitHub)](https://github.com/0xsouravm/oxichrome/blob/main/examples/color-picker/src/lib.rs) — Demonstrates custom FFI for unwrapped Web APIs (EyeDropper) — the pattern to follow for tabGroups.
- **Kept:** [Chrome tabGroups API](https://developer.chrome.com/docs/extensions/reference/api/tabGroups) — Reference for what methods and events tabGroups exposes.
- **Dropped:** docs.rs item listing pages — Redundant with GitHub source; source code provides more detail.
- **Dropped:** crates.io page — Summary only, no API detail beyond what the docs site provides.

## Gaps

1. **tabGroups Rust bindings don't exist in v0.2** — You must write custom `#[wasm_bindgen] extern "C"` blocks for every `chrome.tabGroups` method you need (`query`, `update`, `move`). The pattern is well-documented (see Finding 4 and the Color Picker example), but it's manual work.

2. **`host_permissions` not supported by `#[extension]` macro** — If you need host match patterns (e.g., `https://*.example.com/*`), you can place them in the `permissions` array (Chrome accepts host patterns there), but the clean MV3 way is `host_permissions`. You would need to post-process the generated `manifest.json` or extend `manifest.rs`.

3. **`tabs::on_created` event not wrapped** — Only `on_updated` and `on_activated` are supported. If you need to react to new tab creation, you'll need custom FFI + custom event registration (not `#[on]`).

4. **`tabs::group` method not wrapped** — Grouping tabs in Chrome is done via `chrome.tabs.group()` (which takes a `groupId` from `tabGroups`), and this is not wrapped in v0.2. You'll need custom FFI for that too.

5. **No `#[on]` for tabGroups events** — Even if you write the FFI, you can't use `#[oxichrome::on]` for custom event types; the macro only supports the 5 built-in paths. You'll need to register listeners manually using the Closure pattern shown in `event_handler.rs`.

## Supervisor coordination

No blocks encountered. Research complete. Return the brief directly.
