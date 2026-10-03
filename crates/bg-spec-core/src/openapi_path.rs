//! Helpers for OpenAPI paths and HTTP methods.

use crate::{CoreError, Result};

pub const HTTP_METHODS: &[&str] = &[
    "GET", "PUT", "POST", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE",
];

/// Upper-cases and validates an HTTP method.
pub fn normalize_method(method: &str) -> Result<String> {
    let m = method.trim().to_ascii_uppercase();
    if HTTP_METHODS.contains(&m.as_str()) {
        Ok(m)
    } else {
        Err(CoreError::InvalidInput(format!(
            "unsupported HTTP method '{method}' (expected one of {})",
            HTTP_METHODS.join(", ")
        )))
    }
}

/// Validates an OpenAPI path template (must start with `/`, no traversal, no whitespace).
pub fn validate_path(path: &str) -> Result<String> {
    let p = path.trim();
    let ok = p.starts_with('/')
        && p.len() <= 1024
        && !p.chars().any(|c| c.is_whitespace() || c.is_control())
        && !p.split('/').any(|seg| seg == ".." || seg == ".");
    if ok {
        Ok(p.to_owned())
    } else {
        Err(CoreError::InvalidInput(format!(
            "invalid OpenAPI path '{path}' (must start with '/', no whitespace or dot segments)"
        )))
    }
}

/// Canonical matching key for a path template.
///
/// * strips `prefix` (e.g. `/v1`) when present,
/// * replaces every `{param}` segment with `{}` so that `{account-id}` and `{accountId}` match,
/// * removes a trailing slash.
pub fn path_key(path: &str, prefix: Option<&str>) -> String {
    let mut p = path.trim();
    if let Some(prefix) = prefix
        .map(|x| x.trim_end_matches('/'))
        .filter(|x| !x.is_empty())
        && let Some(rest) = p.strip_prefix(prefix)
        && (rest.is_empty() || rest.starts_with('/'))
    {
        p = rest;
    }
    let segments: Vec<&str> = p
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.starts_with('{') && s.ends_with('}') {
                "{}"
            } else {
                s
            }
        })
        .collect();
    format!("/{}", segments.join("/"))
}

/// Names of templated path parameters in order of appearance.
pub fn template_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
        .map(str::to_owned)
        .collect()
}

/// Template parameter names with case, `-` and `_` ignored (`account-id` ≡ `accountId`).
pub fn param_shape(path: &str) -> Vec<String> {
    template_params(path)
        .iter()
        .map(|p| {
            p.chars()
                .filter(|c| !matches!(c, '-' | '_'))
                .flat_map(char::to_lowercase)
                .collect()
        })
        .collect()
}

/// How well `candidate` matches the `requested` template among operations sharing one
/// canonical key: 3 identical path, 2 identical parameter names, 1 same [`param_shape`], 0 key only.
pub fn template_score(requested: &str, candidate: &str) -> u8 {
    if requested == candidate {
        3
    } else if template_params(requested) == template_params(candidate) {
        2
    } else if param_shape(requested) == param_shape(candidate) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods() {
        assert_eq!(normalize_method("get").unwrap(), "GET");
        assert!(normalize_method("CONNECT").is_err());
    }

    #[test]
    fn template_scores() {
        let pay = "/v2/{payment-service}/{payment-product}/{paymentId}";
        let auth = "/v2/{resource-path}/{resourceId}/{authorisation-category}";
        assert_eq!(template_score(pay, pay), 3);
        assert_eq!(
            template_score("/{payment-service}/{payment-product}/{paymentId}", pay),
            2
        );
        assert_eq!(
            template_score("/v2/{paymentService}/{payment_product}/{payment-id}", pay),
            1
        );
        assert_eq!(template_score(pay, auth), 0);
        assert_eq!(
            param_shape("/a/{Account-Id}/{x_y}"),
            vec!["accountid", "xy"]
        );
    }

    #[test]
    fn keys() {
        assert_eq!(
            path_key("/v1/accounts/{account-id}/transactions", Some("/v1")),
            "/accounts/{}/transactions"
        );
        assert_eq!(
            path_key("/accounts/{accountId}/transactions/", None),
            "/accounts/{}/transactions"
        );
        assert_eq!(path_key("/v10/accounts", Some("/v1")), "/v10/accounts");
        assert_eq!(path_key("/v1", Some("/v1")), "/");
    }

    #[test]
    fn v1_and_v2_keys_match_across_versions() {
        assert_eq!(
            path_key("/v1/accounts/{account-id}/transactions", Some("/v1")),
            path_key("/v2/accounts/{account-id}/transactions", Some("/v2"))
        );
        assert_eq!(
            path_key("/v2/accounts/{account-id}/balances", Some("/v2")),
            path_key("/accounts/{accountId}/balances", Some("/v2"))
        );
        // Prefixes are per version: a /v2 path under the /v1 version is kept as is.
        assert_eq!(
            path_key("/v2/consents/confirmation-of-funds", Some("/v1")),
            "/v2/consents/confirmation-of-funds"
        );
    }

    #[test]
    fn params() {
        assert_eq!(
            template_params("/a/{x}/b/{y-z}"),
            vec!["x".to_owned(), "y-z".to_owned()]
        );
    }

    #[test]
    fn paths() {
        assert!(validate_path("/accounts/{id}").is_ok());
        assert!(validate_path("accounts").is_err());
        assert!(validate_path("/a/../b").is_err());
    }
}
