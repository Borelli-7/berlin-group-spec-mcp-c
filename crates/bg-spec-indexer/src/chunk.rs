//! Page-aware, structure-preserving chunking: page → section → paragraph → sentence.
//!
//! A heading stack is kept per document, so every chunk carries its full `section_path`
//! (`["4 Account Information Service", "4.2 Read Transaction List"]`). Sizes are measured in
//! characters, not bytes.
//!
//! Chunks never cross page boundaries (so every chunk has an exact page citation) and
//! never split a sentence unless the sentence alone exceeds the maximum size. Normative
//! statements (MUST/SHALL/...) are therefore kept intact whenever a boundary exists.

/// Chunking limits (characters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSettings {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

impl Default for ChunkSettings {
    fn default() -> Self {
        Self {
            max_chars: 6000,
            overlap_chars: 500,
        }
    }
}

/// Text of one extracted page.
#[derive(Debug, Clone, Copy)]
pub struct PageText<'a> {
    pub number: u32,
    pub text: &'a str,
}

/// A chunk before provenance is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunk {
    pub page: u32,
    pub section: Option<String>,
    /// Enclosing headings from outermost to innermost (the last one equals `section`).
    pub section_path: Vec<String>,
    /// 0-based position within the page.
    pub ordinal: u32,
    pub text: String,
}

/// Nesting information of a detected heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadingLevel {
    /// `4.2.1` → `[4, 2, 1]`.
    Numbered(Vec<u8>),
    /// `Annex B …` / `Appendix …`: top level.
    Annex,
    /// Markdown `##` → 2.
    Markdown(usize),
}

impl HeadingLevel {
    /// True when a heading at `self` can contain a heading at `child`.
    fn contains(&self, child: &Self) -> bool {
        match (self, child) {
            (Self::Numbered(p), Self::Numbered(c)) => p.len() < c.len() && c.starts_with(p),
            (Self::Markdown(p), Self::Markdown(c)) => p < c,
            _ => false,
        }
    }
}

/// Detects numbered headings such as `4.2.1 Read Transaction List` or `Annex B Error Codes`.
/// Returns the normalized section label (`4.2.1 Read Transaction List`).
pub fn detect_heading(line: &str) -> Option<String> {
    detect_heading_level(line).map(|(label, _)| label)
}

/// Like [`detect_heading`], also returning the heading's nesting level.
pub fn detect_heading_level(line: &str) -> Option<(String, HeadingLevel)> {
    let line = line.trim();
    if let Some(md) = line.strip_prefix('#') {
        let title = md.trim_start_matches('#').trim();
        let depth = 1 + md.len() - md.trim_start_matches('#').len();
        return (!title.is_empty() && md.starts_with([' ', '#']))
            .then(|| (title.to_owned(), HeadingLevel::Markdown(depth)));
    }
    if line.len() < 3 || line.len() > 120 || line.ends_with(['.', ',', ';', ':']) {
        return None;
    }
    let (label, title) = line.split_once(char::is_whitespace)?;
    let title = title.trim();
    let first = title.chars().next()?;
    if !first.is_uppercase() || title.split_whitespace().count() > 14 {
        return None;
    }
    let number = label.strip_suffix('.').unwrap_or(label);
    let numeric = !number.is_empty()
        && number.split('.').count() <= 6
        && number
            .split('.')
            .all(|p| !p.is_empty() && p.len() <= 2 && p.bytes().all(|b| b.is_ascii_digit()));
    if numeric {
        let parts = number.split('.').filter_map(|p| p.parse().ok()).collect();
        return Some((format!("{number} {title}"), HeadingLevel::Numbered(parts)));
    }
    if matches!(label, "Annex" | "Appendix" | "ANNEX" | "APPENDIX") {
        return Some((line.to_owned(), HeadingLevel::Annex));
    }
    None
}

/// Open headings of a document, outermost first.
#[derive(Debug, Default)]
struct HeadingStack(Vec<(String, HeadingLevel)>);

impl HeadingStack {
    fn push(&mut self, label: String, level: HeadingLevel) {
        while self
            .0
            .last()
            .is_some_and(|(_, open)| !open.contains(&level))
        {
            self.0.pop();
        }
        self.0.push((label, level));
    }

    fn path(&self) -> Vec<String> {
        self.0.iter().map(|(l, _)| l.clone()).collect()
    }
}

fn clen(s: &str) -> usize {
    s.chars().count()
}

/// Byte offset of the `n`-th character (or the end).
fn byte_at(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map_or(s.len(), |(i, _)| i)
}

enum Block {
    Heading(String, HeadingLevel),
    Paragraph(String),
}

fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Paragraph(para.join("\n")));
            para.clear();
        }
    };
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            flush(&mut para, &mut out);
        } else if let Some((h, level)) = detect_heading_level(line) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(h, level));
        } else {
            para.push(line);
        }
    }
    flush(&mut para, &mut out);
    out
}

/// Splits text into sentences, keeping terminators and trailing whitespace attached.
fn sentences(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b'.' | b'!' | b'?') {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let boundary = j > i + 1
                && (j == bytes.len()
                    || bytes[j].is_ascii_uppercase()
                    || bytes[j].is_ascii_digit()
                    || bytes[j] == b'"');
            if boundary {
                out.push(&text[start..j]);
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Hard-splits an over-long unit at whitespace (last resort).
fn hard_split(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while clen(rest) > max {
        let cut = byte_at(rest, max);
        let cut = rest[..cut]
            .rfind(char::is_whitespace)
            .filter(|&c| c > 0)
            .unwrap_or(cut);
        out.push(rest[..cut].to_owned());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out
}

/// Units with their separator to the previous unit.
fn units(paragraphs: &[String], max: usize) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for p in paragraphs {
        if clen(p) <= max {
            out.push(("\n\n", p.clone()));
            continue;
        }
        let mut sep = "\n\n";
        for s in sentences(p) {
            if clen(s) <= max {
                out.push((sep, s.trim_end().to_owned()));
            } else {
                for piece in hard_split(s, max) {
                    out.push((sep, piece));
                    sep = " ";
                }
            }
            sep = " ";
        }
    }
    out
}

/// Sentence-aligned tail of `text` of at most `overlap` characters.
fn overlap_tail(text: &str, overlap: usize) -> &str {
    if overlap == 0 || text.is_empty() {
        return "";
    }
    let start = byte_at(text, clen(text).saturating_sub(overlap));
    let tail = &text[start..];
    if start == 0 {
        return tail;
    }
    let sentence_start = sentences(tail)
        .first()
        .map(|first| first.len())
        .filter(|&n| n < tail.len());
    match sentence_start {
        Some(n) => tail[n..].trim_start(),
        None => tail
            .find(char::is_whitespace)
            .map_or("", |n| tail[n..].trim_start()),
    }
}

fn pack(paragraphs: &[String], settings: ChunkSettings) -> Vec<String> {
    let max = settings.max_chars.max(64);
    let overlap = settings.overlap_chars.min(max / 2);
    let mut chunks: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut buf_chars = 0;
    for (sep, unit) in units(paragraphs, max) {
        let unit_chars = clen(&unit);
        let sep_chars = sep.len();
        let needed = if buf.is_empty() {
            unit_chars
        } else {
            buf_chars + sep_chars + unit_chars
        };
        if needed > max && !buf.is_empty() {
            let tail = overlap_tail(&buf, overlap).to_owned();
            chunks.push(std::mem::take(&mut buf));
            buf_chars = 0;
            let tail_chars = clen(&tail);
            if !tail.is_empty() && tail_chars + 1 + unit_chars <= max {
                buf = tail;
                buf_chars = tail_chars;
            }
        }
        if !buf.is_empty() {
            let joiner = if buf_chars + sep_chars + unit_chars <= max {
                sep
            } else {
                " "
            };
            buf.push_str(joiner);
            buf_chars += joiner.len();
        }
        buf.push_str(&unit);
        buf_chars += unit_chars;
    }
    if !buf.trim().is_empty() {
        chunks.push(buf);
    }
    chunks
}

/// Chunks extracted pages. `initial_section` carries the section open at the start of
/// the first page; the section open at the end is carried across pages.
pub fn chunk_pages(pages: &[PageText<'_>], settings: ChunkSettings) -> Vec<RawChunk> {
    let mut out = Vec::new();
    let mut stack = HeadingStack::default();
    for page in pages {
        let mut ordinal = 0u32;
        let mut group: Vec<String> = Vec::new();
        let mut emit = |stack: &HeadingStack, group: &mut Vec<String>, ordinal: &mut u32| {
            let section_path = stack.path();
            for text in pack(group, settings) {
                out.push(RawChunk {
                    page: page.number,
                    section: section_path.last().cloned(),
                    section_path: section_path.clone(),
                    ordinal: *ordinal,
                    text,
                });
                *ordinal += 1;
            }
            group.clear();
        };
        for block in blocks(page.text) {
            match block {
                Block::Heading(h, level) => {
                    emit(&stack, &mut group, &mut ordinal);
                    group.push(h.clone());
                    stack.push(h, level);
                }
                Block::Paragraph(p) => group.push(p),
            }
        }
        emit(&stack, &mut group, &mut ordinal);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(max: usize, overlap: usize) -> ChunkSettings {
        ChunkSettings {
            max_chars: max,
            overlap_chars: overlap,
        }
    }

    #[test]
    fn detects_headings() {
        assert_eq!(
            detect_heading("4.2.1 Read Transaction List").as_deref(),
            Some("4.2.1 Read Transaction List")
        );
        assert_eq!(
            detect_heading("5. Error Codes").as_deref(),
            Some("5 Error Codes")
        );
        assert_eq!(
            detect_heading("Annex B Error Codes").as_deref(),
            Some("Annex B Error Codes")
        );
        assert!(detect_heading("1. The ASPSP shall reject the request.").is_none());
        assert!(detect_heading("200 transactions were returned").is_none());
        assert!(detect_heading("2024 Berlin Group").is_none());
        assert!(detect_heading("Plain text line").is_none());
        assert_eq!(
            detect_heading("## E-07 Access frequency").as_deref(),
            Some("E-07 Access frequency")
        );
        assert!(detect_heading("#hashtag").is_none());
    }

    #[test]
    fn sections_split_and_carry_over_pages() {
        let p1 = "1 Introduction\nIntro text.\n\n2 Transactions\nThe ASPSP SHALL return bookings.";
        let p2 = "Continued text on page two.\n\n2.1 Filters\nThe TPP MAY filter.";
        let chunks = chunk_pages(
            &[
                PageText {
                    number: 1,
                    text: p1,
                },
                PageText {
                    number: 2,
                    text: p2,
                },
            ],
            ChunkSettings::default(),
        );
        let got: Vec<_> = chunks
            .iter()
            .map(|c| (c.page, c.section.as_deref(), c.ordinal))
            .collect();
        assert_eq!(
            got,
            vec![
                (1, Some("1 Introduction"), 0),
                (1, Some("2 Transactions"), 1),
                (2, Some("2 Transactions"), 0),
                (2, Some("2.1 Filters"), 1),
            ]
        );
        assert!(
            chunks[1]
                .text
                .starts_with("2 Transactions\n\nThe ASPSP SHALL")
        );
    }

    #[test]
    fn builds_section_paths_from_heading_levels() {
        let p1 = "4 Account Information\nIntro.\n\n4.2 Transactions\nList.\n\n4.2.1 Filters\nText.";
        let p2 =
            "Still filters.\n\n4.3 Balances\nBal.\n\n5 Payments\nPay.\n\nAnnex A Codes\nCodes.";
        let p3 = "# Errata\n\n## E-07 Frequency\nA.\n\n## E-08 Status\nB.";
        let chunks = chunk_pages(
            &[
                PageText {
                    number: 1,
                    text: p1,
                },
                PageText {
                    number: 2,
                    text: p2,
                },
                PageText {
                    number: 3,
                    text: p3,
                },
            ],
            ChunkSettings::default(),
        );
        let paths: Vec<Vec<&str>> = chunks
            .iter()
            .map(|c| c.section_path.iter().map(String::as_str).collect())
            .collect();
        assert_eq!(
            paths,
            vec![
                vec!["4 Account Information"],
                vec!["4 Account Information", "4.2 Transactions"],
                vec!["4 Account Information", "4.2 Transactions", "4.2.1 Filters"],
                vec!["4 Account Information", "4.2 Transactions", "4.2.1 Filters"],
                vec!["4 Account Information", "4.3 Balances"],
                vec!["5 Payments"],
                vec!["Annex A Codes"],
                vec!["Errata"],
                vec!["Errata", "E-07 Frequency"],
                vec!["Errata", "E-08 Status"],
            ]
        );
        assert!(
            chunks
                .iter()
                .all(|c| c.section.as_ref() == c.section_path.last())
        );
    }

    #[test]
    fn sizes_are_measured_in_characters() {
        let text = "Äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü. ".repeat(20);
        let chunks = chunk_pages(
            &[PageText {
                number: 1,
                text: &text,
            }],
            settings(100, 0),
        );
        assert!(chunks.iter().all(|c| c.text.chars().count() <= 100));
        assert!(chunks.iter().any(|c| c.text.len() > 100));
    }

    #[test]
    fn respects_max_and_never_splits_normative_sentences() {
        let normative =
            "The ASPSP SHALL reject a request without the mandatory bookingStatus parameter.";
        let filler = "Informative background sentence number x.";
        let mut para = String::new();
        for i in 0..40 {
            para.push_str(if i % 5 == 0 { normative } else { filler });
            para.push(' ');
        }
        let chunks = chunk_pages(
            &[PageText {
                number: 3,
                text: &para,
            }],
            settings(400, 100),
        );
        assert!(chunks.len() > 3);
        for c in &chunks {
            assert!(c.text.len() <= 400, "chunk too long: {}", c.text.len());
            assert_eq!(c.page, 3);
            // Every occurrence of the normative keyword is inside a full normative sentence.
            let n_keyword = c.text.matches("SHALL").count();
            assert_eq!(
                c.text.matches(normative).count(),
                n_keyword,
                "split normative sentence in {:?}",
                c.text
            );
        }
    }

    #[test]
    fn overlap_is_sentence_aligned() {
        let text =
            "Alpha one is first. Beta two is second. Gamma three is third. Delta four is fourth.";
        let chunks = chunk_pages(&[PageText { number: 1, text }], settings(64, 30));
        assert!(chunks.len() >= 2);
        for c in &chunks[1..] {
            let first = c.text.chars().next().unwrap();
            assert!(
                first.is_uppercase(),
                "chunk should start at a sentence: {:?}",
                c.text
            );
        }
        assert!(
            chunks[1].text.contains("Beta two is second.") || chunks[1].text.starts_with("Gamma")
        );
    }

    #[test]
    fn hard_splits_only_oversized_sentences() {
        let long = "word ".repeat(100);
        let chunks = chunk_pages(
            &[PageText {
                number: 1,
                text: &long,
            }],
            settings(100, 0),
        );
        assert!(chunks.len() >= 5);
        assert!(chunks.iter().all(|c| c.text.len() <= 100));
    }

    #[test]
    fn empty_page_produces_no_chunks() {
        assert!(
            chunk_pages(
                &[PageText {
                    number: 1,
                    text: "  \n \n"
                }],
                ChunkSettings::default()
            )
            .is_empty()
        );
    }

    #[test]
    fn deterministic() {
        let text = "1 A Heading\nSome text. More text.\n\nNext paragraph.";
        let a = chunk_pages(&[PageText { number: 1, text }], settings(80, 20));
        let b = chunk_pages(&[PageText { number: 1, text }], settings(80, 20));
        assert_eq!(a, b);
    }
}
