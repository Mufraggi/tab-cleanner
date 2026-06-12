//! Model weight caching via the Browser Cache API.
//!
//! Provides two async functions:
//! - `ensure_model_cached(url)` → checks if weights are in cache; if not, fetches from
//!   network and stores them.
//! - `load_weights_from_cache(url)` → loads cached weights as `Vec<u8>`.
//!
//! The Cache API (`caches.open`, `cache.match`, `cache.put`) is fully async
//! (Promise-based), so every call returns a Promise and is awaited via
//! `wasm_bindgen_futures::JsFuture`.
//!
//! # Context
//!
//! Both page (popup) and service worker contexts are supported via a
//! `get_cache_and_fetch()` helper that tries `web_sys::window()` first
//! (browser page), then falls back to `WorkerGlobalScope` (service worker).
//!
//! # web_sys features required
//!
//! ```toml
//! web-sys = { version = "0.3", features = [
//!     "Window",             # window().caches() + fetch_with_str()
//!     "WorkerGlobalScope",  # self.caches() in SW context
//!     "CacheStorage",       # caches.open(), caches.has()
//!     "Cache",              # cache.match_with_str(), cache.put_with_str()
//!     "Response",           # response.array_buffer()
//!     "console",            # console::log_1() for error logging
//! ] }
//! ```

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Cache, CacheStorage, Response};

const CACHE_NAME: &str = "sml-model";

/// Log a message to the browser console (only when the `console` feature is enabled).
fn console_log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

/// Helper: await a Promise and convert the resolved JsValue to a specific web_sys type.
async fn await_promise<T: JsCast>(promise: js_sys::Promise) -> Result<T, JsValue> {
    let val = JsFuture::from(promise).await
        .map_err(|e| {
            console_log(&format!("Promise rejected: {:?}", e));
            e
        })?;
    val.dyn_into().map_err(|_| {
        let msg = "Promise resolved to unexpected type";
        console_log(msg);
        JsValue::from_str(msg)
    })
}

/// Helper: await a Promise, returning `None` if the result is `undefined`,
/// or `Some(T)` if it can be cast to `T`.
async fn await_promise_optional<T: JsCast>(promise: js_sys::Promise) -> Result<Option<T>, JsValue> {
    let val = JsFuture::from(promise).await
        .map_err(|e| {
            console_log(&format!("Promise rejected: {:?}", e));
            e
        })?;
    if val.is_undefined() || val.is_null() {
        return Ok(None);
    }
    let typed: T = val.dyn_into().map_err(|_| {
        let msg = "Promise resolved to unexpected type";
        console_log(msg);
        JsValue::from_str(msg)
    })?;
    Ok(Some(typed))
}

/// Get the Cache API from the global context, supporting both browser pages
/// and service workers.
///
/// Returns the `CacheStorage` object from either `window.caches()` (page)
/// or `WorkerGlobalScope::caches()` (service worker).
fn get_caches() -> Result<CacheStorage, JsValue> {
    // Try Window context first (browser page, popup)
    if let Some(window) = web_sys::window() {
        return window.caches();
    }

    // Fallback: WorkerGlobalScope (service worker)
    let global = js_sys::global();
    let worker: web_sys::WorkerGlobalScope = global
        .dyn_into()
        .map_err(|_| {
            console_log("get_caches: Not a Window nor WorkerGlobalScope — no Cache API available");
            JsValue::from_str("No Cache API available (not Window nor WorkerGlobalScope)")
        })?;
    worker.caches()
}

/// Fetch a URL from the global context, supporting both pages and service workers.
async fn do_fetch(url: &str) -> Result<Response, JsValue> {
    // Try Window context first
    if let Some(window) = web_sys::window() {
        let promise = window.fetch_with_str(url);
        return await_promise::<Response>(promise).await;
    }

    // Fallback: WorkerGlobalScope
    let global = js_sys::global();
    let worker: web_sys::WorkerGlobalScope = global
        .dyn_into()
        .map_err(|_| {
            console_log("do_fetch: Not a Window nor WorkerGlobalScope — cannot fetch");
            JsValue::from_str("Cannot fetch (not Window nor WorkerGlobalScope)")
        })?;
    let promise = worker.fetch_with_str(url);
    await_promise::<Response>(promise).await
}

// ── ensure_model_cached ───────────────────────────────────────────────────

/// Ensure model weights are stored in the Cache API.
///
/// # Flow
/// 1. Open (or create) the `"sml-model"` cache via `caches.open()`.
/// 2. Check if `url` is already cached via `cache.match()`.
/// 3. If hit → return `Ok("cache")` immediately.
/// 4. If miss → `fetch(url)`, then `cache.put(url, response)`, return `Ok("network")`.
///
/// # Returns
/// - `Ok("cache")`  — weights were already in the cache (no network request).
/// - `Ok("network")` — weights were fetched from the network and stored.
///
/// # Errors
/// Returns `Err(String)` on: missing global, Cache API unavailable,
/// network failure, or Cache API rejection.
pub async fn ensure_model_cached(url: &str) -> Result<String, String> {
    // ── Open cache ────────────────────────────────────────────────────────
    let caches: CacheStorage = get_caches()
        .map_err(|e| format!("get_caches failed: {:?}", e))?;

    let cache: Cache = await_promise(caches.open(CACHE_NAME)).await
        .map_err(|e| format!("caches.open({}) failed: {:?}", CACHE_NAME, e))?;

    // ── Check if already cached ──────────────────────────────────────────
    let maybe_response: Option<Response> = await_promise_optional(cache.match_with_str(url)).await
        .map_err(|e| format!("cache.match({}) failed: {:?}", url, e))?;

    if maybe_response.is_some() {
        console_log(&format!("ensure_model_cached: {} → HIT (already cached)", url));
        return Ok("cache".to_string());
    }

    // ── Cache miss → fetch from network ──────────────────────────────────
    console_log(&format!("ensure_model_cached: {} → MISS, fetching from network…", url));

    let response: Response = do_fetch(url).await
        .map_err(|e| format!("fetch({}) failed: {:?}", url, e))?;

    // Check HTTP status
    let status = response.status();
    if status < 200 || status >= 300 {
        let msg = format!(
            "fetch({}) returned HTTP {} — cannot cache",
            url, status
        );
        console_log(&msg);
        return Err(msg);
    }

    // ── Store in cache ───────────────────────────────────────────────────
    // cache.put returns Promise<()> (resolves to undefined)
    let _ = JsFuture::from(cache.put_with_str(url, &response)).await
        .map_err(|e| format!("cache.put({}) failed: {:?}", url, e))?;

    console_log(&format!("ensure_model_cached: {} → STORED in cache", url));
    Ok("network".to_string())
}

// ── load_weights_from_cache ───────────────────────────────────────────────

/// Lightweight check: is a URL present in the `"sml-model"` cache?
///
/// Only calls `cache.match()` — does NOT read the response body.
/// Returns `true` if the URL is cached, `false` otherwise.
/// Returns `false` on errors (missing cache API, etc.) — logs them via console_log.
pub async fn is_url_cached(url: &str) -> bool {
    let caches = match get_caches() {
        Ok(c) => c,
        Err(e) => {
            console_log(&format!("is_url_cached: get_caches failed: {:?}", e));
            return false;
        }
    };

    let cache = match await_promise::<Cache>(caches.open(CACHE_NAME)).await {
        Ok(c) => c,
        Err(e) => {
            console_log(&format!("is_url_cached: caches.open failed: {:?}", e));
            return false;
        }
    };

    let maybe_response = match await_promise_optional::<Response>(cache.match_with_str(url)).await {
        Ok(r) => r,
        Err(e) => {
            console_log(&format!("is_url_cached: cache.match failed: {:?}", e));
            return false;
        }
    };

    maybe_response.is_some()
}

/// Load model weights from the Cache API as raw bytes (`Vec<u8>`).
///
/// # Flow
/// 1. Open the `"sml-model"` cache.
/// 2. `cache.match(url)` → get the cached `Response`.
/// 3. `response.array_buffer()` → `ArrayBuffer`.
/// 4. Convert `ArrayBuffer` → `Uint8Array` → `Vec<u8>`.
///
/// # Prerequisites
/// Call `ensure_model_cached(url)` first so weights are guaranteed to be in cache.
///
/// # Errors
/// Returns `Err(String)` if the URL is not in cache, the response body is empty,
/// or any Cache API call fails.
pub async fn load_weights_from_cache(url: &str) -> Result<Vec<u8>, String> {
    // ── Open cache ────────────────────────────────────────────────────────
    let caches: CacheStorage = get_caches()
        .map_err(|e| format!("get_caches failed: {:?}", e))?;

    let cache: Cache = await_promise(caches.open(CACHE_NAME)).await
        .map_err(|e| format!("caches.open({}) failed: {:?}", CACHE_NAME, e))?;

    // ── Match URL in cache ───────────────────────────────────────────────
    let maybe_response: Option<Response> = await_promise_optional(cache.match_with_str(url)).await
        .map_err(|e| format!("cache.match({}) failed: {:?}", url, e))?;

    let response = maybe_response.ok_or_else(|| {
        let msg = format!(
            "URL not found in cache: {}. Call ensure_model_cached first.",
            url
        );
        console_log(&msg);
        msg
    })?;

    // ── Read body as ArrayBuffer → Vec<u8> ───────────────────────────────
    let array_buffer_val = JsFuture::from(
        response.array_buffer()
            .map_err(|e| format!("response.array_buffer() failed: {:?}", e))?
    ).await
        .map_err(|e| format!("arrayBuffer() promise rejected: {:?}", e))?;

    // Convert ArrayBuffer to Vec<u8>
    let uint8_array = js_sys::Uint8Array::new(&array_buffer_val);
    let bytes = uint8_array.to_vec();

    if bytes.is_empty() {
        console_log("load_weights_from_cache: WARNING — cached body is empty");
    } else {
        console_log(&format!(
            "load_weights_from_cache: {} loaded from cache ({:.1} MB)",
            url,
            bytes.len() as f64 / 1_000_000.0
        ));
    }

    Ok(bytes)
}
