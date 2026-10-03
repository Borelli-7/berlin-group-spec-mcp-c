//! Code-like identifiers (operationIds, header names, schema names, paths, codes) that must be
//! matched exactly rather than through stemmed full-text tokens. Shared by the indexer (which
//! stores them) and the search adapter (which routes query terms to them), so both sides
//! normalise identically.

use std::collections::BTreeSet;

/// Upper bound of identifiers taken from one free-text record.
pub const MAX_IDENTIFIERS_PER_RECORD: usize = 256;
const MAX_IDENTIFIER_CHARS: usize = 200;
const HTTP_METHODS: [&str; 8] = [
    "GET", "PUT", "POST", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE",
];

/// Canonical form used in the index and in queries.
pub fn normalize(id: &str) -> String {
    id.trim().to_lowercase()
}

fn trim_token(t: &str) -> &str {
    t.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '`' | ',' | ';' | ':' | '.' | '(' | ')' | '[' | ']' | '<' | '>'
        )
    })
}

/// True for tokens that look like code rather than prose: hyphen/underscore-joined
/// alphanumerics (`PSU-IP-Address`, `E-07`), camelCase (`bookingStatus`) or paths (`/v2/...`).
pub fn is_code_like(t: &str) -> bool {
    let len = t.chars().count();
    if !(2..=MAX_IDENTIFIER_CHARS).contains(&len) || !t.chars().any(char::is_alphanumeric) {
        return false;
    }
    if t.starts_with('/') {
        return true;
    }
    if !t
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
    {
        return false;
    }
    let chars: Vec<char> = t.chars().collect();
    let joined = chars
        .windows(3)
        .any(|w| matches!(w[1], '-' | '_') && w[0].is_alphanumeric() && w[2].is_alphanumeric());
    let camel = chars
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase());
    joined || camel
}

/// Extracts normalised code-like identifiers from free text, including `METHOD /path` pairs.
pub fn from_text(text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for raw in text.split_whitespace() {
        let token = trim_token(raw);
        if is_code_like(token) {
            if token.starts_with('/')
                && let Some(method) = previous.filter(|m| HTTP_METHODS.contains(m))
            {
                out.insert(normalize(&format!("{method} {token}")));
            }
            out.insert(normalize(token));
        }
        previous = Some(token);
        if out.len() >= MAX_IDENTIFIERS_PER_RECORD {
            break;
        }
    }
    out.into_iter().collect()
}

/// Identifier terms to look up for a query: the whole query (for `GET /path`-style or single
/// identifier queries) plus every whitespace-separated token, all normalised.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let whole = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let whole = trim_token(&whole);
    if !whole.is_empty() && whole.chars().count() <= MAX_IDENTIFIER_CHARS {
        out.insert(normalize(whole));
    }
    for raw in query.split_whitespace() {
        let token = trim_token(raw);
        if token.chars().count() >= 2 && token.chars().count() <= MAX_IDENTIFIER_CHARS {
            out.insert(normalize(token));
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_code_like_tokens() {
        for t in [
            "PSU-IP-Address",
            "E-07",
            "bookingStatus",
            "/v2/accounts",
            "PATH_id",
        ] {
            assert!(is_code_like(t), "{t}");
        }
        for t in ["transaction", "ASPSP", "-", "a", "well-", "page 4"] {
            assert!(!is_code_like(t), "{t}");
        }
    }

    #[test]
    fn extracts_identifiers_and_method_paths() {
        let ids = from_text(
            "Call GET /v2/accounts/{account-id}/transactions with header \"PSU-IP-Address\", bookingStatus.",
        );
        assert_eq!(
            ids,
            vec![
                "/v2/accounts/{account-id}/transactions",
                "bookingstatus",
                "get /v2/accounts/{account-id}/transactions",
                "psu-ip-address",
            ]
        );
    }

    #[test]
    fn query_terms_include_whole_query_and_tokens() {
        assert_eq!(
            query_terms(" GET  /accounts/{accountId} "),
            vec!["/accounts/{accountid}", "get", "get /accounts/{accountid}"]
        );
        assert_eq!(query_terms("\"Consent-ID\""), vec!["consent-id"]);
    }
}
