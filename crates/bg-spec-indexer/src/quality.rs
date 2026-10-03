//! Deterministic extraction-quality signals and search-text cleaning for paged sources.
//!
//! Page text served by `read_source` stays verbatim. Cleaning (running header/footer removal and
//! hyphenation repair) only affects the text that is chunked and indexed for search.

use std::collections::{BTreeMap, BTreeSet};

/// Margin lines inspected at the top and bottom of every page.
const MARGIN_LINES: usize = 2;
/// Minimum number of pages before running headers/footers are detected.
const MIN_PAGES_FOR_MARGINS: usize = 4;
/// Fraction of non-whitespace characters that are garbage above which a page is low quality.
const MAX_GARBAGE_RATIO: f32 = 0.05;
/// Fraction of one-character tokens (letter-spaced text) above which a page is low quality.
const MAX_FRAGMENT_RATIO: f32 = 0.5;
const MIN_TOKENS_FOR_FRAGMENTS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageQuality {
    /// Replacement, control and private-use characters per non-whitespace character.
    pub garbage_ratio: f32,
    /// One-character alphabetic tokens per token.
    pub fragment_ratio: f32,
    pub low_quality: bool,
}

fn is_garbage(c: char) -> bool {
    c == '\u{FFFD}'
        || (c.is_control() && !c.is_whitespace())
        || matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

pub fn page_quality(text: &str) -> PageQuality {
    let significant = text.chars().filter(|c| !c.is_whitespace()).count();
    let garbage = text.chars().filter(|c| is_garbage(*c)).count();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let fragments = tokens
        .iter()
        .filter(|t| t.chars().count() == 1 && t.chars().all(char::is_alphabetic))
        .count();
    let garbage_ratio = if significant == 0 {
        0.0
    } else {
        garbage as f32 / significant as f32
    };
    let fragment_ratio = if tokens.is_empty() {
        0.0
    } else {
        fragments as f32 / tokens.len() as f32
    };
    PageQuality {
        garbage_ratio,
        fragment_ratio,
        low_quality: garbage_ratio > MAX_GARBAGE_RATIO
            || (tokens.len() >= MIN_TOKENS_FOR_FRAGMENTS && fragment_ratio > MAX_FRAGMENT_RATIO),
    }
}

/// Comparison key of a margin line: digits collapsed (page numbers), whitespace normalised.
fn margin_key(line: &str) -> Option<String> {
    let mut key = String::new();
    let mut last_digit = false;
    for c in line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
    {
        if c.is_ascii_digit() {
            if !last_digit {
                key.push('#');
            }
            last_digit = true;
        } else {
            key.push(c);
            last_digit = false;
        }
    }
    (!key.is_empty()).then_some(key)
}

fn margin_indices(lines: &[&str]) -> Vec<usize> {
    let non_empty: Vec<usize> = (0..lines.len())
        .filter(|&i| !lines[i].trim().is_empty())
        .collect();
    let mut idx: BTreeSet<usize> = non_empty.iter().take(MARGIN_LINES).copied().collect();
    idx.extend(non_empty.iter().rev().take(MARGIN_LINES).copied());
    idx.into_iter().collect()
}

/// Margin keys (first/last non-empty lines) repeated on at least half of the pages (min. 3).
pub fn repeated_margins(pages: &[&str]) -> BTreeSet<String> {
    if pages.len() < MIN_PAGES_FOR_MARGINS {
        return BTreeSet::new();
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for page in pages {
        let lines: Vec<&str> = page.lines().collect();
        let keys: BTreeSet<String> = margin_indices(&lines)
            .into_iter()
            .filter_map(|i| margin_key(lines[i]))
            .collect();
        for k in keys {
            *counts.entry(k).or_default() += 1;
        }
    }
    let threshold = pages.len().div_ceil(2).max(3);
    counts
        .into_iter()
        .filter(|(_, n)| *n >= threshold)
        .map(|(k, _)| k)
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub text: String,
    pub removed_margin_lines: usize,
    pub hyphenation_repairs: usize,
}

/// Joins `exam-\nple` → `example` when both fragments are plain lowercase-continued words.
/// Identifiers such as `X-Request-\nID` or `{account-\nid}` are left untouched.
fn repair_hyphenation(lines: Vec<String>) -> (Vec<String>, usize) {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut repairs = 0;
    let mut iter = lines.into_iter().peekable();
    while let Some(mut line) = iter.next() {
        while let Some(next) = iter.peek() {
            let Some(stem) = line.strip_suffix('-') else {
                break;
            };
            let left = stem.rsplit(char::is_whitespace).next().unwrap_or_default();
            let next_trim = next.trim_start();
            let right: String = next_trim
                .chars()
                .take_while(|c| c.is_alphabetic())
                .collect();
            let after = next_trim[right.len()..].chars().next();
            let joinable = left.chars().count() >= 2
                && left.chars().all(char::is_alphabetic)
                && left.chars().last().is_some_and(char::is_lowercase)
                && right.chars().next().is_some_and(char::is_lowercase)
                && after.is_none_or(|c| c.is_whitespace() || ".,;:)!?".contains(c));
            if !joinable {
                break;
            }
            let next = iter.next().unwrap_or_default();
            line = format!("{stem}{}", next.trim_start());
            repairs += 1;
        }
        out.push(line);
    }
    (out, repairs)
}

/// Search text of a page: repeated margins removed, hyphenation repaired.
pub fn clean_page(text: &str, margins: &BTreeSet<String>) -> Cleaned {
    let lines: Vec<&str> = text.lines().collect();
    let drop: BTreeSet<usize> = if margins.is_empty() {
        BTreeSet::new()
    } else {
        margin_indices(&lines)
            .into_iter()
            .filter(|&i| margin_key(lines[i]).is_some_and(|k| margins.contains(&k)))
            .collect()
    };
    let kept: Vec<String> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, l)| (*l).to_owned())
        .collect();
    let (kept, hyphenation_repairs) = repair_hyphenation(kept);
    Cleaned {
        text: kept.join("\n").trim().to_owned(),
        removed_margin_lines: drop.len(),
        hyphenation_repairs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_garbage_and_letter_spacing() {
        assert!(!page_quality("The ASPSP SHALL return the balances.").low_quality);
        assert!(page_quality("\u{FFFD}\u{FFFD} abc \u{E000}").low_quality);
        let spaced = "T h e A S P S P s h a l l r e t u r n t h e b a l a n c e s";
        assert!(page_quality(spaced).low_quality);
    }

    #[test]
    fn detects_and_removes_running_headers_and_footers() {
        let titles = ["Consent", "Accounts", "Section", "Payments", "Funds"];
        let pages: Vec<String> = (1..=5)
            .map(|n| {
                format!(
                    "Implementation Guidelines v2.0\n\n4.{n} {t}\nAbout {t}.\n\nPage {n} of 5",
                    t = titles[n - 1]
                )
            })
            .collect();
        let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
        let margins = repeated_margins(&refs);
        assert!(margins.contains("Implementation Guidelines v#.#"));
        assert!(margins.contains("Page # of #"));
        assert!(!margins.iter().any(|m| m.contains("Section")));
        let c = clean_page(&pages[2], &margins);
        assert_eq!(c.text, "4.3 Section\nAbout Section.");
        assert_eq!(c.removed_margin_lines, 2);
        assert!(repeated_margins(&refs[..3]).is_empty());
    }

    #[test]
    fn repairs_only_plain_word_hyphenation() {
        let c = clean_page(
            "the transac-\ntion list\nX-Request-\nID header\n{account-\nid}\nre-\nissue.",
            &BTreeSet::new(),
        );
        assert_eq!(
            c.text,
            "the transaction list\nX-Request-\nID header\n{account-\nid}\nreissue."
        );
        assert_eq!(c.hyphenation_repairs, 2);
    }
}
