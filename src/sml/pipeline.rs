//! Pure-Rust inference pipeline for sentence embeddings.
//!
//! This module ties together the pure-Rust WordPiece tokenizer with the
//! candle BERT model to produce L2-normalized sentence embeddings.
//!
//! ZERO dependency on the HuggingFace `tokenizers` crate — the entire
//! pipeline compiles to `wasm32-unknown-unknown`.
//!
//! Pipeline: text → normalize → pre-tokenize → WordPiece → tensors → BertModel.forward()
//!           → mean_pool → l2_normalize → embedding vector

use candle_core::{Device, Tensor};
#[cfg(test)]
use candle_core::DType;
#[cfg(test)]
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, HiddenAct, PositionEmbeddingType};
use crate::sml::tokenizer::{WordPieceTokenizer, PAD_TOKEN_ID};

// ── Model Config ───────────────────────────────────────────────────────────

/// all-MiniLM-L6-v2 config (the candle built-in `_all_mini_lm_l6_v2` is private).
pub fn all_mini_lm_l6_v2_config() -> Config {
    Config {
        vocab_size: 30522,
        hidden_size: 384,
        num_hidden_layers: 6,
        num_attention_heads: 12,
        intermediate_size: 1536,
        hidden_act: HiddenAct::Gelu,
        hidden_dropout_prob: 0.1,
        max_position_embeddings: 512,
        type_vocab_size: 2,
        initializer_range: 0.02,
        layer_norm_eps: 1e-12,
        pad_token_id: PAD_TOKEN_ID as usize,
        position_embedding_type: PositionEmbeddingType::Absolute,
        use_cache: true,
        classifier_dropout: None,
        model_type: Some("bert".to_string()),
    }
}

// ── Tokenization ───────────────────────────────────────────────────────────

/// Tokenize a batch of texts using the pure-Rust WordPiece tokenizer.
///
/// Returns `(input_ids, token_type_ids, attention_mask)` tensors, padded to
/// the max sequence length in the batch.
///
/// - `input_ids`: shape `(batch_size, max_seq_len)`, dtype `u32`
/// - `token_type_ids`: shape `(batch_size, max_seq_len)`, dtype `u32` (all zeros for single-sentence)
/// - `attention_mask`: shape `(batch_size, max_seq_len)`, dtype `f32`
pub fn tokenize_batch(
    tokenizer: &WordPieceTokenizer,
    texts: &[&str],
    device: &Device,
) -> candle_core::Result<(Tensor, Tensor, Tensor)> {
    // Tokenize each text individually
    let mut all_ids: Vec<Vec<u32>> = Vec::with_capacity(texts.len());
    let mut all_mask: Vec<Vec<u32>> = Vec::with_capacity(texts.len());

    for text in texts {
        let (ids, mask) = tokenizer.tokenize(text);
        all_ids.push(ids);
        all_mask.push(mask);
    }

    // Find max sequence length
    let max_len = all_ids.iter().map(|ids| ids.len()).max().unwrap_or(1);

    // Pad and flatten into vectors
    let batch_size = texts.len();
    let mut padded_ids = vec![0u32; batch_size * max_len];
    let mut padded_type_ids = vec![0u32; batch_size * max_len];
    let mut padded_mask = vec![0.0f32; batch_size * max_len];

    for (i, (ids, mask)) in all_ids.iter().zip(all_mask.iter()).enumerate() {
        let len = ids.len().min(max_len);
        for j in 0..len {
            padded_ids[i * max_len + j] = ids[j];
            padded_type_ids[i * max_len + j] = 0; // single sentence → all zero
            padded_mask[i * max_len + j] = mask[j] as f32;
        }
    }

    let input_ids = Tensor::from_vec(padded_ids, (batch_size, max_len), device)?;
    let token_type_ids = Tensor::from_vec(padded_type_ids, (batch_size, max_len), device)?;
    let attention_mask = Tensor::from_vec(padded_mask, (batch_size, max_len), device)?;

    Ok((input_ids, token_type_ids, attention_mask))
}

// ── Pooling ────────────────────────────────────────────────────────────────

/// Mean pooling over the last hidden state, weighted by attention mask.
///
/// This is the standard sentence-transformers pooling strategy.
/// `last_hidden_state`: shape `(batch, seq_len, hidden_size)`
/// `attention_mask`: shape `(batch, seq_len)`
///
/// Returns tensor of shape `(batch, hidden_size)`.
pub fn mean_pool(last_hidden_state: &Tensor, attention_mask: &Tensor) -> candle_core::Result<Tensor> {
    // Expand attention mask to [batch, seq_len, 1] → [batch, seq_len, hidden_size]
    let attention_mask_expanded = attention_mask
        .unsqueeze(2)?
        .broadcast_as(last_hidden_state.shape())?;

    // Weighted sum: mask each token position, then sum over seq_len
    let sum_embeddings = last_hidden_state
        .broadcast_mul(&attention_mask_expanded)?
        .sum(1)?; // [batch, hidden_size]

    // Compute denominator: sum of mask values per sequence → [batch, 1]
    let sum_mask = attention_mask.sum_keepdim(1)?; // [batch, 1]
    let sum_mask = sum_mask.clamp(1e-9f64, f64::MAX)?; // Avoid division by zero

    let embeddings = sum_embeddings.broadcast_div(&sum_mask)?;
    Ok(embeddings)
}

// ── Normalization ──────────────────────────────────────────────────────────

/// L2 normalize embeddings so each vector has unit norm.
pub fn l2_normalize(embeddings: &Tensor) -> candle_core::Result<Tensor> {
    let norm = embeddings.sqr()?.sum_keepdim(1)?.sqrt()?;
    let norm = norm.clamp(1e-12f64, f64::MAX)?;
    let result = embeddings.broadcast_div(&norm)?;
    Ok(result)
}

// ── Full Pipeline ──────────────────────────────────────────────────────────

/// Compute L2-normalized sentence embeddings for a batch of texts.
///
/// This is the complete, pure-Rust pipeline:
/// 1. Tokenize with WordPieceTokenizer
/// 2. Forward pass through BertModel
/// 3. Mean pooling
/// 4. L2 normalization
///
/// Returns tensor of shape `(batch_size, hidden_size)` with unit-normalized rows.
pub fn embed_batch(
    model: &BertModel,
    tokenizer: &WordPieceTokenizer,
    texts: &[&str],
    device: &Device,
) -> candle_core::Result<Tensor> {
    let (input_ids, token_type_ids, attention_mask) =
        tokenize_batch(tokenizer, texts, device)?;

    let output = model.forward(&input_ids, &token_type_ids, Some(&attention_mask))?;
    let pooled = mean_pool(&output, &attention_mask)?;
    let embeddings = l2_normalize(&pooled)?;

    Ok(embeddings)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sml::tokenizer::WordPieceTokenizer;

    fn load_test_tokenizer() -> WordPieceTokenizer {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/tokenizer.json");
        WordPieceTokenizer::from_file(path).expect("Failed to load tokenizer.json")
    }

    #[test]
    fn test_tokenize_batch_padding() {
        let tok = load_test_tokenizer();
        let device = Device::Cpu;

        // Two texts of different lengths
        let texts = ["hello", "hello world this is longer"];
        let (input_ids, _type_ids, attention_mask) =
            tokenize_batch(&tok, &texts, &device).unwrap();

        // Shape should be (2, max_len) where max_len is from the longer text
        let shape = input_ids.dims();
        assert_eq!(shape.len(), 2);
        assert_eq!(shape[0], 2);
        assert!(shape[1] >= 5); // longer text has at least 5 tokens

        // Attention mask should be 1 for real tokens, 0 for padding
        let mask: Vec<f32> = attention_mask.flatten_all().unwrap().to_vec1().unwrap();
        // For first (shorter) text, should have 1s followed by 0s
        assert!(mask[0] > 0.9);
        // Last position of shorter text should be 0 (padding)
        if shape[1] > 3 {
            // The shorter text has at most 3 tokens (CLS, hello, SEP) → position beyond is padding
            assert_eq!(mask[3], 0.0);
        }
    }

    #[test]
    fn test_mean_pool_shapes() {
        let device = Device::Cpu;
        // Simulate: batch=1, seq_len=3, hidden_size=2
        let hidden = Tensor::new(&[[[1.0f32, 2.0], [3.0, 4.0], [5.0, 6.0]]], &device).unwrap();
        let mask = Tensor::new(&[[1.0f32, 1.0, 0.0]], &device).unwrap();

        let pooled = mean_pool(&hidden, &mask).unwrap();
        let dims = pooled.dims();
        assert_eq!(dims, vec![1, 2]);

        // Mean of first two positions (third is masked): ((1+3)/2, (2+4)/2) = (2, 3)
        let vals: Vec<f32> = pooled.flatten_all().unwrap().to_vec1().unwrap();
        assert!((vals[0] - 2.0).abs() < 0.01);
        assert!((vals[1] - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_l2_normalize_unit_norm() {
        let device = Device::Cpu;
        let emb = Tensor::new(&[[3.0f32, 4.0]], &device).unwrap(); // norm = 5
        let normalized = l2_normalize(&emb).unwrap();

        let vals: Vec<f32> = normalized.flatten_all().unwrap().to_vec1().unwrap();
        assert!((vals[0] - 0.6).abs() < 0.001);
        assert!((vals[1] - 0.8).abs() < 0.001);

        // Verify unit norm
        let norm_sq = vals[0] * vals[0] + vals[1] * vals[1];
        assert!((norm_sq - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_pipeline_with_zeros() {
        // Verify the full pipeline compiles and runs with zero weights.
        // This is a shape/architecture test, not a semantic test.
        let device = Device::Cpu;
        let config = all_mini_lm_l6_v2_config();
        let vb = VarBuilder::zeros(DType::F32, &device);
        let model = BertModel::load(vb, &config).unwrap();

        let tok = load_test_tokenizer();

        let texts = ["Hello world", "Rust programming"];
        let embeddings = embed_batch(&model, &tok, &texts, &device).unwrap();

        let dims = embeddings.dims();
        assert_eq!(dims, vec![2, 384]); // batch_size=2, hidden_size=384

        // With zeros weights, embeddings should be non-NaN (architecture correct)
        let vals: Vec<f32> = embeddings.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(vals.len(), 2 * 384);
        for &v in &vals {
            assert!(!v.is_nan());
        }
    }
}
