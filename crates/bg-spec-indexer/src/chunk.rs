//! Page-aware, structure-preserving chunking: page → section → paragraph → sentence.
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
    /// 0-based position within the page.
    pub ordinal: u32,
    pub text: String,
}

/// Detects numbered headings such as `4.2.1 Read Transaction List` or `Annex B Error Codes`.
/// Returns the normalized section label (`4.2.1 Read Transaction List`).
pub fn detect_heading(line: &str) -> Option<String> {
    let line = line.trim();
    if let Some(md) = line.strip_prefix('#') {
        let title = md.trim_start_matches('#').trim();
        return (!title.is_empty() && md.starts_with([' ', '#'])).then(|| title.to_owned());
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
        return Some(format!("{number} {title}"));
    }
    if matches!(label, "Annex" | "Appendix" | "ANNEX" | "APPENDIX") {
        return Some(line.to_owned());
    }
    None
}

enum Block {
    Heading(String),
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
        } else if let Some(h) = detect_heading(line) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(h));
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
    while rest.len() > max {
        let cut = rest.floor_char_boundary(max);
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
        if p.len() <= max {
            out.push(("\n\n", p.clone()));
            continue;
        }
        let mut sep = "\n\n";
        for s in sentences(p) {
            if s.len() <= max {
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
    let start = text.ceil_char_boundary(text.len().saturating_sub(overlap));
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
    for (sep, unit) in units(paragraphs, max) {
        let needed = if buf.is_empty() {
            unit.len()
        } else {
            buf.len() + sep.len() + unit.len()
        };
        if needed > max && !buf.is_empty() {
            let tail = overlap_tail(&buf, overlap).to_owned();
            chunks.push(std::mem::take(&mut buf));
            if !tail.is_empty() && tail.len() + 1 + unit.len() <= max {
                buf = tail;
            }
        }
        if !buf.is_empty() {
            buf.push_str(if buf.len() + sep.len() + unit.len() <= max {
                sep
            } else {
                " "
            });
        }
        buf.push_str(&unit);
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
    let mut section: Option<String> = None;
    for page in pages {
        let mut ordinal = 0u32;
        let mut group: Vec<String> = Vec::new();
        let mut emit = |section: &Option<String>, group: &mut Vec<String>, ordinal: &mut u32| {
            for text in pack(group, settings) {
                out.push(RawChunk {
                    page: page.number,
                    section: section.clone(),
                    ordinal: *ordinal,
                    text,
                });
                *ordinal += 1;
            }
            group.clear();
        };
        for block in blocks(page.text) {
            match block {
                Block::Heading(h) => {
                    emit(&section, &mut group, &mut ordinal);
                    group.push(h.clone());
                    section = Some(h);
                }
                Block::Paragraph(p) => group.push(p),
            }
        }
        emit(&section, &mut group, &mut ordinal);
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
