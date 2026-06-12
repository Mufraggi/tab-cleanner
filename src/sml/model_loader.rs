//! Model loading from safetensors bytes + cached inference.
//!
//! Provides a two-step API:
//! 1. `load_model_from_bytes(weights_bytes)` → parses safetensors and caches the model
//! 2. `embed_cached(tokenizer_json, texts_json)` → runs inference on the cached model
//!
//! The model is stored in a `thread_local!` → `RefCell<Option<CachedModel>>`.
//! This is safe in WASM's single-threaded context.

use std::cell::RefCell;

use candle_core::{Device, DType};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::BertModel;

use crate::sml::tokenizer::WordPieceTokenizer;
use crate::sml::pipeline;

/// Global cached model. In WASM we're single-threaded, so RefCell is safe.
struct CachedModel {
    model: BertModel,
}

thread_local! {
    static CACHED_MODEL: RefCell<Option<CachedModel>> = RefCell::new(None);
}

/// Load REAL model weights from safetensors bytes into an internal cache.
///
/// This uses `VarBuilder::from_buffered_safetensors` which exists in candle-nn 0.10+.
///
/// After this call, `embed_cached()` can be used for inference.
///
/// # Errors
/// Returns `Err(String)` if the safetensors parsing fails or the model
/// architecture doesn't match `all-mini_lm_l6_v2_config`.
pub fn load_model_from_bytes(weights_bytes: Vec<u8>) -> Result<(), String> {
    let device = Device::Cpu;
    let config = pipeline::all_mini_lm_l6_v2_config();

    let vb = VarBuilder::from_buffered_safetensors(weights_bytes, DType::F32, &device)
        .map_err(|e| format!("Failed to load weights from buffer: {}", e))?;

    let model = BertModel::load(vb, &config)
        .map_err(|e| format!("Failed to build BertModel: {}", e))?;

    CACHED_MODEL.with(|cache| {
        *cache.borrow_mut() = Some(CachedModel { model });
    });

    Ok(())
}

/// Run inference using the cached model (loaded via `load_model_from_bytes`).
///
/// # Parameters
/// - `tokenizer_json`: The full contents of `tokenizer.json` (466 KB).
/// - `texts_json`: A JSON array of strings, e.g. `["Hello world", "Rust"]`.
///
/// # Returns
/// Returns flattened `Vec<f32>` of shape `(batch_size * 384)` — L2-normalized.
/// Each row is 384 floats. The caller can slice into rows for cosine similarity.
///
/// # Errors
/// Returns `Err(String)` if the model hasn't been loaded, tokenizer parsing fails,
/// or inference fails.
pub fn embed_cached(
    tokenizer_json: &str,
    texts_json: &str,
) -> Result<Vec<f32>, String> {
    let device = Device::Cpu;

    let texts: Vec<String> = serde_json::from_str(texts_json)
        .map_err(|e| format!("Failed to parse texts JSON: {}", e))?;

    let tokenizer = WordPieceTokenizer::from_json(tokenizer_json)
        .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

    let embeddings = CACHED_MODEL.with(|cache| {
        let cache = cache.borrow();
        let cached = cache.as_ref()
            .ok_or_else(|| "Model not loaded. Call load_model_from_bytes first.".to_string())?;
        pipeline::embed_batch(&cached.model, &tokenizer, &text_refs, &device)
            .map_err(|e| format!("Embedding failed: {}", e))
    })?;

    let flat: Vec<f32> = embeddings.flatten_all()
        .map_err(|e| format!("Flatten failed: {}", e))?
        .to_vec1()
        .map_err(|e| format!("to_vec1 failed: {}", e))?;

    Ok(flat)
}


