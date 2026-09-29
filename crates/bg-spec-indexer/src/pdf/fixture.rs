//! Deterministic minimal PDF writer used to produce synthetic regression fixtures.
//!
//! Generates single-font (Helvetica, WinAnsi) text-only PDFs. Not a general PDF writer:
//! it exists so fixtures are reproducible and contain no third-party copyrighted text.

/// One page of text lines. An empty page simulates a scanned (image-only) page.
#[derive(Debug, Clone, Default)]
pub struct FixturePage {
    pub lines: Vec<String>,
}

impl FixturePage {
    pub fn new<I, S>(lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
        }
    }

    pub fn blank() -> Self {
        Self::default()
    }
}

fn escape_pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

fn content_stream(page: &FixturePage) -> String {
    if page.lines.is_empty() {
        return String::new();
    }
    let mut s = String::from("BT\n/F1 10 Tf\n13 TL\n50 800 Td\n");
    for line in &page.lines {
        s.push('(');
        s.push_str(&escape_pdf_text(line));
        s.push_str(") Tj T*\n");
    }
    s.push_str("ET\n");
    s
}

/// Renders a deterministic PDF document (byte-identical for identical input).
pub fn render_pdf(title: &str, pages: &[FixturePage]) -> Vec<u8> {
    // Object layout: 1 catalog, 2 pages, 3 font, 4 info, then (page, content) pairs.
    let n = pages.len();
    let page_obj = |i: usize| 5 + 2 * i;
    let mut objects: Vec<String> = Vec::with_capacity(4 + 2 * n);
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_obj(i))).collect();
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        n
    ));
    objects.push(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    );
    objects.push(format!(
        "<< /Title ({}) /Producer (bg-spec fixture writer) >>",
        escape_pdf_text(title)
    ));
    for (i, page) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            page_obj(i) + 1
        ));
        let stream = content_stream(page);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        ));
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref_at = out.len();
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for off in offsets {
        xref.push_str(&format!("{off:010} 00000 n \n"));
    }
    out.extend_from_slice(xref.as_bytes());
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_at
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_output() {
        let pages = [FixturePage::new(["Hello (world)"]), FixturePage::blank()];
        assert_eq!(render_pdf("t", &pages), render_pdf("t", &pages));
        let pdf = String::from_utf8_lossy(&render_pdf("t", &pages)).into_owned();
        assert!(pdf.contains("Hello \\(world\\)"));
        assert!(pdf.contains("/Count 2"));
    }
}
