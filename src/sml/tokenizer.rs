//! Pure-Rust WordPiece tokenizer, compatible with all-MiniLM-L6-v2 (BERT uncased).
//!
//! This module replaces the HuggingFace `tokenizers` crate (which depends on
//! `onig` C code and doesn't compile for `wasm32-unknown-unknown`) with a
//! zero-C-dependency, WASM-compatible implementation.
//!
//! The tokenizer:
//! 1. Normalizes text: lowercase, strip accents, clean whitespace, CJK spacing
//! 2. Pre-tokenizes: splits on whitespace then on punctuation
//! 3. WordPiece: greedy longest-match-first against the BERT vocab
//! 4. Adds [CLS] (101) prefix and [SEP] (102) suffix
//!
//! Reference: HuggingFace `tokenizers` BertNormalizer + BertPreTokenizer + WordPiece.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use unicode_normalization::UnicodeNormalization;

/// Special token IDs for BERT uncased.
pub const CLS_TOKEN_ID: u32 = 101;
pub const SEP_TOKEN_ID: u32 = 102;
pub const UNK_TOKEN_ID: u32 = 100;
pub const PAD_TOKEN_ID: u32 = 0;

/// A parsed WordPiece vocabulary.
#[derive(Debug, Clone)]
pub struct WordPieceVocab {
    /// Token string → token ID.
    pub token_to_id: HashMap<String, u32>,
    /// The prefix used for continuation subwords (e.g., "##").
    pub continuing_subword_prefix: String,
    /// Max characters per input word before treating as [UNK].
    pub max_input_chars_per_word: usize,
}

/// A pure-Rust WordPiece tokenizer.
#[derive(Debug, Clone)]
pub struct WordPieceTokenizer {
    vocab: WordPieceVocab,
}

/// Minimal structure to parse the `model` section of `tokenizer.json`.
#[derive(Debug, Deserialize)]
struct TokenizerJson {
    model: ModelSection,
}

#[derive(Debug, Deserialize)]
struct ModelSection {
    vocab: HashMap<String, u32>,
    #[serde(default = "default_continuing_subword_prefix")]
    continuing_subword_prefix: String,
    #[serde(default = "default_max_input_chars_per_word")]
    max_input_chars_per_word: usize,
}

fn default_continuing_subword_prefix() -> String {
    "##".to_string()
}

fn default_max_input_chars_per_word() -> usize {
    100
}

// ── Public API ──────────────────────────────────────────────────────────────

impl WordPieceTokenizer {
    /// Load the vocabulary from a HuggingFace `tokenizer.json` file.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let data = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read tokenizer.json: {}", e))?;
        Self::from_json(&data)
    }

    /// Load the vocabulary from a `tokenizer.json` string (WASM-friendly).
    pub fn from_json(json: &str) -> Result<Self, String> {
        let parsed: TokenizerJson = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse tokenizer.json: {}", e))?;

        let vocab = WordPieceVocab {
            token_to_id: parsed.model.vocab,
            continuing_subword_prefix: parsed.model.continuing_subword_prefix,
            max_input_chars_per_word: parsed.model.max_input_chars_per_word,
        };

        Ok(WordPieceTokenizer { vocab })
    }

    /// Tokenize a single text into `(token_ids, attention_mask)`.
    ///
    /// The output always includes `[CLS]` at position 0 and `[SEP]` at the
    /// last non-padding position. No padding is added — the caller should pad
    /// to the max sequence length of a batch.
    pub fn tokenize(&self, text: &str) -> (Vec<u32>, Vec<u32>) {
        // 1. Normalize
        let normalized = normalize(text);

        // 2. Pre-tokenize
        let words = pre_tokenize(&normalized);

        // 3. WordPiece each word
        let mut token_ids = Vec::with_capacity(words.len() + 10);
        token_ids.push(CLS_TOKEN_ID);

        for word in &words {
            if word.is_empty() {
                continue;
            }
            self.wordpiece_tokenize(word, &mut token_ids);
        }

        token_ids.push(SEP_TOKEN_ID);

        // 4. Attention mask (all 1s)
        let attention_mask = vec![1u32; token_ids.len()];

        (token_ids, attention_mask)
    }

    /// WordPiece-tokenize a single pre-tokenized word.
    ///
    /// Uses greedy longest-match-first against the vocabulary.
    /// If no match is found, emits `[UNK]`.
    fn wordpiece_tokenize(&self, word: &str, output: &mut Vec<u32>) {
        if word.is_empty() {
            return;
        }

        // If the word exceeds the max character count, emit [UNK].
        if word.chars().count() > self.vocab.max_input_chars_per_word {
            output.push(UNK_TOKEN_ID);
            return;
        }

        // Check if the whole word is in the vocab.
        if let Some(&id) = self.vocab.token_to_id.get(word) {
            output.push(id);
            return;
        }

        let chars: Vec<char> = word.chars().collect();
        let len = chars.len();
        let mut start = 0;
        let mut is_first = true;
        let mut found_any = false;

        while start < len {
            let mut end = len;
            let mut matched = false;

            while end > start {
                let sub: String = chars[start..end].iter().collect();
                let lookup = if is_first {
                    sub.clone()
                } else {
                    format!("{}{}", self.vocab.continuing_subword_prefix, sub)
                };

                if self.vocab.token_to_id.contains_key(&lookup) {
                    output.push(self.vocab.token_to_id[&lookup]);
                    matched = true;
                    found_any = true;
                    break;
                }
                end -= 1;
            }

            if !matched {
                // No subword match → emit [UNK] and stop processing this word.
                output.push(UNK_TOKEN_ID);
                return;
            }

            start = end;
            is_first = false;
        }

        // Safety: if we somehow didn't find anything (shouldn't happen since
        // we checked `matched` above), emit [UNK].
        if !found_any {
            output.push(UNK_TOKEN_ID);
        }
    }
}

// ── Normalization (BertNormalizer) ──────────────────────────────────────────

/// Apply BERT normalization:
/// 1. Lowercase (Unicode-aware)
/// 2. Strip accents (NFD + remove nonspacing marks)
/// 3. Clean text (replace control chars, normalize whitespace)
/// 4. Handle Chinese characters (add spaces around CJK)
fn normalize(text: &str) -> String {
    let lower = text.to_lowercase();

    // Strip accents: NFD decomposition → filter out Mn (Nonspacing Mark) category
    let ascii_fold = lower.nfd().filter(|c| !is_mark(*c)).collect::<String>();

    // NFD may introduce some combining characters as separate characters;
    // re-compose to NFC for cleaner output.
    let nfc = ascii_fold.nfc().collect::<String>();

    // Clean text and handle CJK
    clean_text(&nfc)
}

/// Check if a character is a nonspacing mark (Unicode category Mn).
fn is_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}' |  // Combining Diacritical Marks
        '\u{0483}'..='\u{0489}' |  // Combining Cyrillic Marks
        '\u{0591}'..='\u{05BD}' |  // Hebrew accents
        '\u{05BF}' |
        '\u{05C1}'..='\u{05C2}' |
        '\u{05C4}'..='\u{05C5}' |
        '\u{05C7}' |
        '\u{0610}'..='\u{061A}' |  // Arabic marks
        '\u{064B}'..='\u{065F}' |
        '\u{0670}' |
        '\u{06D6}'..='\u{06DC}' |
        '\u{06DF}'..='\u{06E4}' |
        '\u{06E7}'..='\u{06E8}' |
        '\u{06EA}'..='\u{06ED}' |
        '\u{0711}' |
        '\u{0730}'..='\u{074A}' |
        '\u{07A6}'..='\u{07B0}' |
        '\u{0900}'..='\u{0902}' |  // Devanagari
        '\u{093A}'..='\u{093C}' |
        '\u{0941}'..='\u{0948}' |
        '\u{094D}' |
        '\u{0951}'..='\u{0957}' |
        '\u{0962}'..='\u{0963}' |
        '\u{0981}' |
        '\u{09BC}' |
        '\u{09C1}'..='\u{09C4}' |
        '\u{09CD}' |
        '\u{09E2}'..='\u{09E3}' |
        '\u{0A01}'..='\u{0A02}' |
        '\u{0A3C}' |
        '\u{0A41}'..='\u{0A42}' |
        '\u{0A47}'..='\u{0A48}' |
        '\u{0A4B}'..='\u{0A4D}' |
        '\u{0A70}'..='\u{0A71}' |
        '\u{0A81}'..='\u{0A82}' |
        '\u{0ABC}' |
        '\u{0AC1}'..='\u{0AC5}' |
        '\u{0AC7}'..='\u{0AC8}' |
        '\u{0ACD}' |
        '\u{0AE2}'..='\u{0AE3}' |
        '\u{0B01}' |
        '\u{0B3C}' |
        '\u{0B3F}' |
        '\u{0B41}'..='\u{0B44}' |
        '\u{0B4D}' |
        '\u{0B56}' |
        '\u{0B62}'..='\u{0B63}' |
        '\u{0B82}' |
        '\u{0BC0}' |
        '\u{0BCD}' |
        '\u{0C3E}'..='\u{0C40}' |
        '\u{0C46}'..='\u{0C48}' |
        '\u{0C4A}'..='\u{0C4D}' |
        '\u{0C55}'..='\u{0C56}' |
        '\u{0CBC}' |
        '\u{0CBF}' |
        '\u{0CC6}' |
        '\u{0CCC}'..='\u{0CCD}' |
        '\u{0CE2}'..='\u{0CE3}' |
        '\u{0D41}'..='\u{0D44}' |
        '\u{0D4D}' |
        '\u{0DCA}' |
        '\u{0DD2}'..='\u{0DD4}' |
        '\u{0DD6}' |
        '\u{0E31}' |
        '\u{0E34}'..='\u{0E3A}' |
        '\u{0E47}'..='\u{0E4E}' |
        '\u{0EB1}' |
        '\u{0EB4}'..='\u{0EB9}' |
        '\u{0EBB}'..='\u{0EBC}' |
        '\u{0EC8}'..='\u{0ECD}' |
        '\u{0F18}'..='\u{0F19}' |
        '\u{0F35}' |
        '\u{0F37}' |
        '\u{0F39}' |
        '\u{0F71}'..='\u{0F7E}' |
        '\u{0F80}'..='\u{0F84}' |
        '\u{0F86}'..='\u{0F87}' |
        '\u{0F90}'..='\u{0F97}' |
        '\u{0F99}'..='\u{0FBC}' |
        '\u{0FC6}' |
        '\u{102D}'..='\u{1030}' |
        '\u{1032}'..='\u{1037}' |
        '\u{1039}'..='\u{103A}' |
        '\u{103D}'..='\u{103E}' |
        '\u{1058}'..='\u{1059}' |
        '\u{105E}'..='\u{1061}' |
        '\u{1071}'..='\u{1074}' |
        '\u{1082}' |
        '\u{1085}'..='\u{1086}' |
        '\u{108D}' |
        '\u{109D}' |
        '\u{135D}'..='\u{135F}' |
        '\u{1712}'..='\u{1714}' |
        '\u{1732}'..='\u{1734}' |
        '\u{1752}'..='\u{1753}' |
        '\u{1772}'..='\u{1773}' |
        '\u{17B4}'..='\u{17B5}' |
        '\u{17B7}'..='\u{17BD}' |
        '\u{17C6}' |
        '\u{17C9}'..='\u{17D3}' |
        '\u{17DD}' |
        '\u{180B}'..='\u{180D}' |
        '\u{18A9}' |
        '\u{1920}'..='\u{1922}' |
        '\u{1927}'..='\u{1928}' |
        '\u{1932}' |
        '\u{1939}'..='\u{193B}' |
        '\u{1A17}'..='\u{1A18}' |
        '\u{1B00}'..='\u{1B03}' |
        '\u{1B34}' |
        '\u{1B36}'..='\u{1B3A}' |
        '\u{1B3C}' |
        '\u{1B42}' |
        '\u{1B6B}'..='\u{1B73}' |
        '\u{1DC0}'..='\u{1DF5}' |   // Combining Diacritical Marks Supplement
        '\u{1DFC}'..='\u{1DFF}' |
        '\u{200C}'..='\u{200D}' |   // ZWJ/ZWNJ
        '\u{20D0}'..='\u{20F0}' |   // Combining Diacritical Marks for Symbols
        '\u{2CEF}'..='\u{2CF1}' |
        '\u{2DE0}'..='\u{2DFF}' |   // Cyrillic Extended-A
        '\u{A66F}'..='\u{A672}' |
        '\u{A67C}'..='\u{A67D}' |
        '\u{A802}' |
        '\u{A806}' |
        '\u{A80B}' |
        '\u{A825}'..='\u{A826}' |
        '\u{FE00}'..='\u{FE0F}' |   // Variation Selectors
        '\u{FE20}'..='\u{FE2F}' |   // Combining Half Marks
        '\u{1D165}'..='\u{1D169}' |
        '\u{1D16D}'..='\u{1D172}' |
        '\u{1D17B}'..='\u{1D182}' |
        '\u{1D185}'..='\u{1D18B}' |
        '\u{1D1AA}'..='\u{1D1AD}' |
        '\u{1D242}'..='\u{1D244}'
    )
}

/// Clean text: replace control characters with spaces, normalize whitespace,
/// and add spaces around CJK characters.
fn clean_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 10);
    let mut prev_was_whitespace = true; // trim leading whitespace

    for ch in text.chars() {
        if is_control(ch) {
            // Replace control chars with space
            if !prev_was_whitespace {
                result.push(' ');
                prev_was_whitespace = true;
            }
        } else if is_cjk(ch) {
            // Add space before CJK character
            if !prev_was_whitespace {
                result.push(' ');
            }
            result.push(ch);
            result.push(' ');
            prev_was_whitespace = true;
        } else if ch.is_whitespace() {
            if !prev_was_whitespace {
                result.push(' ');
                prev_was_whitespace = true;
            }
        } else {
            result.push(ch);
            prev_was_whitespace = false;
        }
    }

    // Trim trailing whitespace
    let trimmed = result.trim_end().to_string();
    trimmed
}

/// Check if a character is a control character (excluding \t, \n, \r which
/// are handled as whitespace above).
fn is_control(ch: char) -> bool {
    // BERT normalizer replaces control chars except \t \n \r
    if ch == '\t' || ch == '\n' || ch == '\r' {
        return false;
    }
    // Unicode general category Cc (control) or other control-like
    // We use a simpler check: code points below 0x20 (excluding \t \n \r) are control chars.
    // Also check for Unicode category Cf, Co, Cs, Cn (but these are rare in real text).
    (ch as u32) < 0x20
}

/// Check if a character is CJK (Chinese, Japanese, Korean).
fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' |   // CJK Unified Ideographs
        '\u{3400}'..='\u{4DBF}' |   // CJK Unified Ideographs Extension A
        '\u{3040}'..='\u{309F}' |   // Hiragana
        '\u{30A0}'..='\u{30FF}' |   // Katakana
        '\u{AC00}'..='\u{D7AF}' |   // Hangul Syllables
        '\u{1100}'..='\u{11FF}' |   // Hangul Jamo
        '\u{2E80}'..='\u{2EFF}' |   // CJK Radicals Supplement
        '\u{2F00}'..='\u{2FDF}' |   // Kangxi Radicals
        '\u{3000}'..='\u{303F}' |   // CJK Symbols and Punctuation
        '\u{31C0}'..='\u{31EF}' |   // CJK Strokes
        '\u{3200}'..='\u{32FF}' |   // Enclosed CJK Letters and Months
        '\u{3300}'..='\u{33FF}' |   // CJK Compatibility
        '\u{F900}'..='\u{FAFF}' |   // CJK Compatibility Ideographs
        '\u{FE30}'..='\u{FE4F}' |   // CJK Compatibility Forms
        '\u{FF00}'..='\u{FFEF}'     // Halfwidth and Fullwidth Forms
    )
}

// ── Pre-tokenization (BertPreTokenizer) ─────────────────────────────────────

/// Split normalized text into words, separating punctuation as individual tokens.
fn pre_tokenize(text: &str) -> Vec<String> {
    let mut words = Vec::new();

    // First split on whitespace
    for whitespace_token in text.split_whitespace() {
        if whitespace_token.is_empty() {
            continue;
        }

        // Split on punctuation within each whitespace-delimited token
        split_punctuation(whitespace_token, &mut words);
    }

    words
}

/// Split a whitespace-delimited token on punctuation, emitting punctuation
/// characters as separate tokens.
fn split_punctuation(token: &str, output: &mut Vec<String>) {
    let chars: Vec<char> = token.chars().collect();
    let len = chars.len();
    let mut start = 0;

    for i in 0..len {
        if is_punctuation(chars[i]) {
            // Emit the text before the punctuation (if any)
            if i > start {
                output.push(chars[start..i].iter().collect());
            }
            // Emit the punctuation as its own token
            output.push(chars[i].to_string());
            start = i + 1;
        }
    }

    // Emit any remaining text after the last punctuation
    if start < len {
        output.push(chars[start..len].iter().collect());
    }
}

/// Check if a character is punctuation that the BERT pre-tokenizer splits on.
fn is_punctuation(ch: char) -> bool {
    // Unicode general category P (all punctuation)
    if ch.is_ascii_punctuation() {
        return true;
    }

    // Additional Unicode punctuation categories
    matches!(
        ch,
        // Currency symbols (split in BERT)
        '$' |
        // Plus and related (splits in BERT)
        '+' | '<' | '=' | '>' | '@' | '^' | '`' | '|' | '~' |
        // C0 controls that act as punctuation in some contexts
        _ if {
            let cat = unicode_category(ch);
            matches!(cat, 'P')
        }
    )
}

/// Minimal Unicode general category check (only need P* categories).
fn unicode_category(ch: char) -> char {
    // Use built-in character property checks where possible
    if ch.is_ascii_punctuation() {
        return 'P';
    }

    // For non-ASCII, we use a simplified approach:
    // Check Unicode blocks that are punctuation

    // General Punctuation block
    if matches!(ch, '\u{2000}'..='\u{206F}') {
        // Dash, quote, etc.
        if matches!(ch,
            '\u{2010}'..='\u{2027}' | // Dashes, quotes
            '\u{2030}'..='\u{205E}'   // Per-mille, etc.
        ) {
            return 'P';
        }
    }

    // CJK Symbols
    if matches!(ch, '\u{3000}'..='\u{303F}') {
        return 'P';
    }

    // For all other non-ASCII, we conservatively return 'L' (letter)
    // to avoid over-splitting. The BERT pre-tokenizer is more nuanced
    // but for our practical purposes (Western + common Unicode), this is fine.
    'L'
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Load vocab from the tokenizer.json located in the test-data directory.
    fn load_test_tokenizer() -> WordPieceTokenizer {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/tokenizer.json");
        WordPieceTokenizer::from_file(path).expect("Failed to load tokenizer.json for tests")
    }

    /// Helper: tokenize and return non-padded IDs (strip trailing zeros).
    fn tokenize_ids(tok: &WordPieceTokenizer, text: &str) -> Vec<u32> {
        let (ids, _mask) = tok.tokenize(text);
        ids
    }

    // ── Normalization tests ────────────────────────────────────────────────

    #[test]
    fn test_normalize_lowercase() {
        assert_eq!(normalize("Hello"), "hello");
        assert_eq!(normalize("HELLO"), "hello");
    }

    #[test]
    fn test_normalize_strip_accents() {
        assert_eq!(normalize("Café"), "cafe");
        assert_eq!(normalize("résumé"), "resume");
        assert_eq!(normalize("naïve"), "naive");
        assert_eq!(normalize("Über"), "uber");
        assert_eq!(normalize("Ångström"), "angstrom");
        assert_eq!(normalize("São"), "sao");
        assert_eq!(normalize("Déjà"), "deja");
        assert_eq!(normalize("garçon"), "garcon");
        assert_eq!(normalize("fiancée"), "fiancee");
    }

    #[test]
    fn test_normalize_clean_whitespace() {
        let s = normalize("  hello   world  ");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_normalize_cjk() {
        let s = normalize("Hello世界");
        assert_eq!(s, "hello 世 界");
    }

    // ── Pre-tokenization tests ─────────────────────────────────────────────

    #[test]
    fn test_pre_tokenize_simple() {
        let tokens = pre_tokenize("hello world");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_pre_tokenize_punctuation() {
        let tokens = pre_tokenize("hello-world");
        assert_eq!(tokens, vec!["hello", "-", "world"]);
    }

    #[test]
    fn test_pre_tokenize_multiple_punctuation() {
        let tokens = pre_tokenize("a.b!c?");
        assert_eq!(tokens, vec!["a", ".", "b", "!", "c", "?"]);
    }

    #[test]
    fn test_pre_tokenize_slash() {
        let tokens = pre_tokenize("foo/bar/baz");
        assert_eq!(tokens, vec!["foo", "/", "bar", "/", "baz"]);
    }

    // ── WordPiece tests ────────────────────────────────────────────────────

    #[test]
    fn test_tokenize_empty() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "");
        // [CLS]=101, [SEP]=102
        assert_eq!(ids, vec![101, 102]);
    }

    #[test]
    fn test_tokenize_simple_word() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "hello");
        // [CLS] hello [SEP]  →  [101, 7592, 102]
        assert_eq!(ids, vec![101, 7592, 102]);
    }

    #[test]
    fn test_tokenize_github() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "GitHub");
        // Normalized: "github" → [CLS] gi ##th ##ub [SEP]
        // Reference: [101, 21025, 2705, 12083, 102]
        assert_eq!(ids, vec![101, 21025, 2705, 12083, 102]);
    }

    #[test]
    fn test_tokenize_youtube() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "YouTube");
        // Normalized: "youtube" → [CLS] youtube [SEP]
        // Reference: [101, 7858, 102]
        assert_eq!(ids, vec![101, 7858, 102]);
    }

    #[test]
    fn test_tokenize_hello_world() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "Hello World");
        // [CLS] hello world [SEP]
        // Reference: [101, 7592, 2088, 102]
        assert_eq!(ids, vec![101, 7592, 2088, 102]);
    }

    #[test]
    fn test_tokenize_stack_overflow() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "Stack Overflow - Where Developers Learn");
        // Reference: [101, 9991, 2058, 12314, 1011, 2073, 9797, 4553, 102]
        assert_eq!(
            ids,
            vec![101, 9991, 2058, 12314, 1011, 2073, 9797, 4553, 102]
        );
    }

    #[test]
    fn test_tokenize_amazon() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "Amazon.com: Online Shopping");
        // Reference: [101, 9733, 1012, 4012, 1024, 3784, 6023, 102]
        assert_eq!(
            ids,
            vec![101, 9733, 1012, 4012, 1024, 3784, 6023, 102]
        );
    }

    #[test]
    fn test_tokenize_twitter() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "Twitter. It's what's happening.");
        // Reference: [101, 10474, 1012, 2009, 1005, 1055, 2054, 1005, 1055, 6230, 1012, 102]
        assert_eq!(
            ids,
            vec![101, 10474, 1012, 2009, 1005, 1055, 2054, 1005, 1055, 6230, 1012, 102]
        );
    }

    #[test]
    fn test_tokenize_mdn() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "MDN Web Docs");
        // Reference: [101, 9108, 2078, 4773, 9986, 2015, 102]
        assert_eq!(
            ids,
            vec![101, 9108, 2078, 4773, 9986, 2015, 102]
        );
    }

    #[test]
    fn test_tokenize_crates_io() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "crates.io: Rust Package Registry");
        // Reference: [101, 27619, 1012, 22834, 1024, 18399, 7427, 15584, 102]
        assert_eq!(
            ids,
            vec![101, 27619, 1012, 22834, 1024, 18399, 7427, 15584, 102]
        );
    }

    #[test]
    fn test_tokenize_a() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "a");
        // Reference: [101, 1037, 102]  ("a" → id 1037 in BERT vocab)
        assert_eq!(ids, vec![101, 1037, 102]);
    }

    #[test]
    fn test_tokenize_chatgpt() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "ChatGPT");
        // Normalized: "chatgpt" → [CLS] chat ##gp ##t [SEP]
        // Reference: [101, 11834, 21600, 2102, 102]
        assert_eq!(ids, vec![101, 11834, 21600, 2102, 102]);
    }

    #[test]
    fn test_tokenize_cpp() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "C++ Programming");
        // Normalized: "c++ programming" → [CLS] c + + programming [SEP]
        // Reference: [101, 1039, 1009, 1009, 4730, 102]
        assert_eq!(ids, vec![101, 1039, 1009, 1009, 4730, 102]);
    }

    #[test]
    fn test_tokenize_nodejs() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "node.js tutorial");
        // Reference: [101, 13045, 1012, 1046, 2015, 14924, 4818, 102]
        assert_eq!(
            ids,
            vec![101, 13045, 1012, 1046, 2015, 14924, 4818, 102]
        );
    }

    #[test]
    fn test_tokenize_react_router() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "react-router-dom");
        // Reference: [101, 10509, 1011, 2799, 2099, 1011, 14383, 102]
        assert_eq!(
            ids,
            vec![101, 10509, 1011, 2799, 2099, 1011, 14383, 102]
        );
    }

    #[test]
    fn test_tokenize_kubernetes() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "kubernetes vs docker swarm");
        // Reference: [101, 13970, 5677, 7159, 2229, 5443, 8946, 2121, 21708, 102]
        assert_eq!(
            ids,
            vec![101, 13970, 5677, 7159, 2229, 5443, 8946, 2121, 21708, 102]
        );
    }

    #[test]
    fn test_tokenize_french() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "Réserver un hôtel à Lisbonne");
        // Normalized: "reserver un hotel a lisbonne"
        // "reserver" → [CLS] reserve ##r [SEP-like continuation]
        // Reference: [101, 3914, 2099, 4895, 3309, 1037, 11929, 2638, 102]
        assert_eq!(
            ids,
            vec![101, 3914, 2099, 4895, 3309, 1037, 11929, 2638, 102]
        );
    }

    #[test]
    fn test_tokenize_github_dash() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "GitHub - rust-lang/rust");
        // Reference: [101, 21025, 2705, 12083, 1011, 18399, 1011, 11374, 1013, 18399, 102]
        assert_eq!(
            ids,
            vec![101, 21025, 2705, 12083, 1011, 18399, 1011, 11374, 1013, 18399, 102]
        );
    }

    // ── Edge case tests ────────────────────────────────────────────────────

    #[test]
    fn test_tokenize_accented() {
        let tok = load_test_tokenizer();
        let ids = tokenize_ids(&tok, "Café");
        // "cafe" → cafe is in vocab
        assert!(ids.len() >= 2); // at least [CLS] ... [SEP]
        // cafe should be tokenized as a known word
        assert!(!ids.contains(&UNK_TOKEN_ID), "cafe should not produce [UNK]");
    }

    #[test]
    fn test_tokenize_unknown_chars() {
        let tok = load_test_tokenizer();
        // Emoji characters are truly out-of-vocab for BERT
        let ids = tokenize_ids(&tok, "\u{1F600}\u{1F600}");
        // Should contain [UNK] (100) between [CLS] and [SEP]
        assert!(ids.contains(&UNK_TOKEN_ID));
        // Reference: [101, 100, 102] for 😀😀
        assert_eq!(ids, vec![101, 100, 102]);
    }
}
