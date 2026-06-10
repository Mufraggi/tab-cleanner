/// Extract meaningful keywords from a tab title.
///
/// Returns an empty `Vec` for missing / empty titles.
/// Keywords are lowercase, deduplicated, and filtered to remove
/// boilerplate separators and very short noise words.
pub fn extract_keywords(title: &str) -> Vec<String> {
    if title.trim().is_empty() {
        return vec![];
    }

    // Common boilerplate words to filter out (English-only for now)
    let boilerplate: &[&str] = &[
        "github",
        "youtube",
        "google",
        "docs",
        "wiki",
        "page",
        "tab",
        "new",
        "chrome",
        "mozilla",
        "firefox",
        "edge",
    ];

    // Split on common title separators (multi-character).
    // This turns "Pull Requests · hugomufraggi/tab-cleanner · GitHub"
    // into segments ["Pull Requests", "hugomufraggi/tab-cleanner", "GitHub"]
    let separators = [" - ", " – ", " | ", " · ", " — "];
    let mut segments = vec![title.to_string()];
    for sep in &separators {
        let mut new_segments = Vec::new();
        for seg in &segments {
            let parts: Vec<&str> = seg.split(sep).collect();
            new_segments.extend(parts.into_iter().map(|s| s.to_string()));
        }
        segments = new_segments;
    }

    // Split each segment by whitespace, then split each resulting token
    // on non-alphanumeric characters (handles "hugomufraggi/tab-cleanner"
    // → ["hugomufraggi", "tab", "cleanner"]).
    let mut tokens: Vec<String> = Vec::new();
    for seg in &segments {
        for word in seg.split_whitespace() {
            // Split on any non-alphanumeric character
            for part in word.split(|c: char| !c.is_alphanumeric()) {
                if part.is_empty() {
                    continue;
                }
                let lower = part.to_lowercase();

                // Remove tokens shorter than 3 characters
                if lower.len() < 3 {
                    continue;
                }

                // Remove tokens that are purely numeric (or contain only digits, dots, commas)
                if lower.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',') {
                    continue;
                }

                // Remove boilerplate words
                if boilerplate.contains(&lower.as_str()) {
                    continue;
                }

                tokens.push(lower);
            }
        }
    }

    // Deduplicate preserving order
    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<String> = Vec::new();
    for token in tokens {
        if seen.insert(token.clone()) {
            deduped.push(token);
        }
    }

    // Cap at 5 keywords to avoid noise dominance
    deduped.truncate(5);
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meaningful_title() {
        let result = extract_keywords("Build software better, together");
        // "Build" → "build", "software" → "software", "better," → split on "," → "better", "together" → "together"
        assert_eq!(
            result,
            vec![
                "build".to_string(),
                "software".to_string(),
                "better".to_string(),
                "together".to_string(),
            ]
        );
    }

    #[test]
    fn test_boilerplate_only_title() {
        let result = extract_keywords("YouTube");
        // "youtube" is in the boilerplate list → filtered
        assert!(result.is_empty());
    }

    #[test]
    fn test_title_with_separators() {
        let result = extract_keywords("Pull Requests · hugomufraggi/tab-cleanner · GitHub");
        // After split by " · ": ["Pull Requests", "hugomufraggi/tab-cleanner", "GitHub"]
        // After split by whitespace: ["Pull", "Requests", "hugomufraggi/tab-cleanner", "GitHub"]
        // After split on non-alphanumeric: ["Pull", "Requests", "hugomufraggi", "tab", "cleanner", "GitHub"]
        // "Pull" → "pull", "Requests" → "requests", "hugomufraggi" → "hugomufraggi",
        // "tab" → "tab" (boilerplate → filtered), "cleanner" → "cleanner",
        // "GitHub" → "github" (boilerplate → filtered)
        assert_eq!(
            result,
            vec![
                "pull".to_string(),
                "requests".to_string(),
                "hugomufraggi".to_string(),
                "cleanner".to_string(),
            ]
        );
    }

    #[test]
    fn test_empty_title() {
        let result = extract_keywords("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_whitespace_only_title() {
        let result = extract_keywords("  ");
        assert!(result.is_empty());
    }

    #[test]
    fn test_short_tokens_filtered() {
        let result = extract_keywords("a an of in to be");
        // All tokens are < 3 chars → empty
        assert!(result.is_empty());
    }

    #[test]
    fn test_numeric_tokens_filtered() {
        let result = extract_keywords("Page 42 of 100");
        // "page" → boilerplate → filtered
        // "42" → purely numeric → filtered
        // "of" → < 3 chars → filtered
        // "100" → purely numeric → filtered
        assert!(result.is_empty());
    }

    #[test]
    fn test_boilerplate_filtered() {
        let result = extract_keywords("GitHub - My Repository");
        // "GitHub" → "github" → boilerplate → filtered
        // "My" → "my" → 2 chars → filtered
        // "Repository" → "repository" → kept
        assert_eq!(result, vec!["repository".to_string()]);
    }

    #[test]
    fn test_limit_five_keywords() {
        let title = "one two three four five six seven eight nine ten";
        let result = extract_keywords(title);
        // "one" (3 chars), "two" (3), "three" (5), "four" (4), "five" (4),
        // "six" (3), "seven" (5), "eight" (5), "nine" (4), "ten" (3)
        // None are boilerplate or numeric → all pass → capped at 5
        assert_eq!(result.len(), 5);
        assert_eq!(
            result,
            vec![
                "one".to_string(),
                "two".to_string(),
                "three".to_string(),
                "four".to_string(),
                "five".to_string(),
            ]
        );
    }

    #[test]
    fn test_no_duplicates() {
        let result = extract_keywords("Rust Rust Rust cargo cargo");
        // "Rust" → "rust", "cargo" → "cargo"
        // Dedup preserving order: ["rust", "cargo"]
        assert_eq!(
            result,
            vec!["rust".to_string(), "cargo".to_string()]
        );
    }

    #[test]
    fn test_mixed_case_keywords() {
        let result = extract_keywords("Hello WORLD");
        assert_eq!(
            result,
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn test_only_boilerplate_and_noise() {
        let result = extract_keywords("New Tab");
        // "New" → "new" → boilerplate → filtered
        // "Tab" → "tab" → boilerplate → filtered
        assert!(result.is_empty());
    }
}
