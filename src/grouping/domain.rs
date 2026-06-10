/// Extract a cleaned domain from a URL string.
///
/// Returns `None` for:
///   - Empty / missing URLs
///   - chrome://, chrome-extension://, about:, file:, data:, etc. (non-web schemes)
///   - Malformed URLs that url::Url cannot parse
///
/// Returns `Some(domain)` with `www.` stripped for web URLs.
pub fn extract_domain(raw_url: &str) -> Option<String> {
    if raw_url.is_empty() {
        return None;
    }

    let parsed = url::Url::parse(raw_url).ok()?;

    // Only accept http and https schemes
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return None,
    }

    let host = parsed.host_str()?;

    // Strip leading "www." prefix
    let domain = host.strip_prefix("www.").unwrap_or(host);

    Some(domain.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_https_url() {
        assert_eq!(
            extract_domain("https://www.github.com/repos"),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn test_chrome_url_returns_none() {
        assert_eq!(extract_domain("chrome://extensions"), None);
    }

    #[test]
    fn test_about_blank_returns_none() {
        assert_eq!(extract_domain("about:blank"), None);
    }

    #[test]
    fn test_empty_string_returns_none() {
        assert_eq!(extract_domain(""), None);
    }

    #[test]
    fn test_ip_address() {
        assert_eq!(
            extract_domain("http://192.168.1.1"),
            Some("192.168.1.1".to_string())
        );
    }

    #[test]
    fn test_localhost() {
        assert_eq!(
            extract_domain("http://localhost:3000"),
            Some("localhost".to_string())
        );
    }

    #[test]
    fn test_https_without_www() {
        assert_eq!(
            extract_domain("https://github.com/hugomufraggi/tab-cleanner"),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn test_chrome_extension_url_returns_none() {
        assert_eq!(
            extract_domain("chrome-extension://abc123/background.js"),
            None
        );
    }

    #[test]
    fn test_file_url_returns_none() {
        assert_eq!(extract_domain("file:///tmp/foo.html"), None);
    }

    #[test]
    fn test_data_url_returns_none() {
        assert_eq!(extract_domain("data:text/plain;base64,SGVsbG8="), None);
    }

    #[test]
    fn test_malformed_url_returns_none() {
        assert_eq!(extract_domain("not a url at all"), None);
    }

    #[test]
    fn test_www_subdomain_stripped() {
        assert_eq!(
            extract_domain("http://www.WWW.example.com/page"),
            Some("www.example.com".to_string())
        );
        // Note: only leading "www." is stripped; "www.example.com" → "www.example.com"
        // because after lowercase it's "www.example.com" and strip_prefix("www.") on that gives "example.com"
    }

    #[test]
    fn test_https_www_docs_rs() {
        assert_eq!(
            extract_domain("https://docs.rs/oxichrome-core/latest"),
            Some("docs.rs".to_string())
        );
    }

    #[test]
    fn test_http_no_subdomain() {
        assert_eq!(
            extract_domain("http://example.com/path?q=1"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_moz_extension_returns_none() {
        assert_eq!(extract_domain("moz-extension://abc123/foo"), None);
    }

    #[test]
    fn test_lowercases_domain() {
        assert_eq!(
            extract_domain("HTTPS://WWW.GITHUB.COM/REPOS"),
            Some("github.com".to_string())
        );
    }
}
