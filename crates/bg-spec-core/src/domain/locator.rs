use crate::{CoreError, Result, openapi_path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Addressable location inside a source document.
///
/// Textual forms (stable, round-trippable):
///
/// | Form | Meaning |
/// |------|---------|
/// | `document` | whole document (metadata + page overview) |
/// | `page:12` | one page of a paged document |
/// | `page:12-14` | inclusive page range |
/// | `section:4.2.1` | section (heading number or title prefix) |
/// | `chunk:<chunk_id>` | one indexed evidence chunk |
/// | `op:GET /accounts/{accountId}/transactions` | one OpenAPI operation |
/// | `path:/accounts/{accountId}/transactions` | all operations of an OpenAPI path |
/// | `schema:Transaction` | one OpenAPI component schema |
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub enum SourceLocator {
    Document,
    Page(u32),
    PageRange(u32, u32),
    Section(String),
    Chunk(String),
    Operation { method: String, path: String },
    Path(String),
    Schema(String),
}

/// Upper bound for page ranges read in a single call.
pub const MAX_PAGE_RANGE: u32 = 50;

fn invalid(locator: &str, reason: impl Into<String>) -> CoreError {
    CoreError::InvalidLocator {
        locator: locator.to_owned(),
        reason: reason.into(),
    }
}

fn parse_page(locator: &str, s: &str) -> Result<u32> {
    let n: u32 = s
        .trim()
        .parse()
        .map_err(|_| invalid(locator, format!("'{s}' is not a page number")))?;
    if n == 0 {
        return Err(invalid(locator, "page numbers start at 1"));
    }
    Ok(n)
}

impl FromStr for SourceLocator {
    type Err = CoreError;

    fn from_str(raw: &str) -> Result<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("document") {
            return Ok(Self::Document);
        }
        let (scheme, rest) = s
            .split_once(':')
            .ok_or_else(|| invalid(raw, "expected '<scheme>:<value>' or 'document'"))?;
        let rest = rest.trim();
        if rest.is_empty() {
            return Err(invalid(raw, "empty locator value"));
        }
        match scheme.trim().to_ascii_lowercase().as_str() {
            "page" | "pages" => match rest.split_once('-') {
                Some((a, b)) => {
                    let (a, b) = (parse_page(raw, a)?, parse_page(raw, b)?);
                    if b < a {
                        return Err(invalid(raw, "page range end is before start"));
                    }
                    if b - a + 1 > MAX_PAGE_RANGE {
                        return Err(invalid(
                            raw,
                            format!("page range larger than {MAX_PAGE_RANGE} pages"),
                        ));
                    }
                    Ok(if a == b {
                        Self::Page(a)
                    } else {
                        Self::PageRange(a, b)
                    })
                }
                None => Ok(Self::Page(parse_page(raw, rest)?)),
            },
            "section" => Ok(Self::Section(rest.to_owned())),
            "chunk" => Ok(Self::Chunk(rest.to_owned())),
            "op" | "operation" => {
                let (method, path) = rest
                    .split_once(char::is_whitespace)
                    .ok_or_else(|| invalid(raw, "expected 'op:<METHOD> <path>'"))?;
                let method = openapi_path::normalize_method(method)
                    .map_err(|e| invalid(raw, e.to_string()))?;
                let path = openapi_path::validate_path(path.trim())
                    .map_err(|e| invalid(raw, e.to_string()))?;
                Ok(Self::Operation { method, path })
            }
            "path" => Ok(Self::Path(
                openapi_path::validate_path(rest).map_err(|e| invalid(raw, e.to_string()))?,
            )),
            "schema" => {
                if rest.len() > 256 || rest.contains(char::is_whitespace) {
                    return Err(invalid(raw, "invalid schema name"));
                }
                Ok(Self::Schema(rest.to_owned()))
            }
            other => Err(invalid(raw, format!("unknown locator scheme '{other}'"))),
        }
    }
}

impl fmt::Display for SourceLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document => f.write_str("document"),
            Self::Page(p) => write!(f, "page:{p}"),
            Self::PageRange(a, b) => write!(f, "page:{a}-{b}"),
            Self::Section(s) => write!(f, "section:{s}"),
            Self::Chunk(c) => write!(f, "chunk:{c}"),
            Self::Operation { method, path } => write!(f, "op:{method} {path}"),
            Self::Path(p) => write!(f, "path:{p}"),
            Self::Schema(s) => write!(f, "schema:{s}"),
        }
    }
}

impl TryFrom<String> for SourceLocator {
    type Error = CoreError;
    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}

impl From<SourceLocator> for String {
    fn from(l: SourceLocator) -> Self {
        l.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for s in [
            "document",
            "page:12",
            "page:3-5",
            "section:4.2.1",
            "chunk:src:p1:c0",
            "op:GET /accounts/{accountId}/transactions",
            "path:/accounts/{accountId}/transactions",
            "schema:Transaction",
        ] {
            let l: SourceLocator = s.parse().unwrap();
            assert_eq!(l.to_string(), s);
        }
    }

    #[test]
    fn normalizes() {
        assert_eq!(
            "op:get  /accounts".parse::<SourceLocator>().unwrap(),
            SourceLocator::Operation {
                method: "GET".into(),
                path: "/accounts".into()
            }
        );
        assert_eq!(
            "page:4-4".parse::<SourceLocator>().unwrap(),
            SourceLocator::Page(4)
        );
    }

    #[test]
    fn rejects_bad_input() {
        for s in [
            "",
            "page:0",
            "page:x",
            "page:5-3",
            "page:1-100",
            "op:FETCH /a",
            "op:GET",
            "path:accounts",
            "path:/../etc",
            "file:/etc/passwd",
            "schema:a b",
        ] {
            assert!(s.parse::<SourceLocator>().is_err(), "{s} should fail");
        }
    }
}
