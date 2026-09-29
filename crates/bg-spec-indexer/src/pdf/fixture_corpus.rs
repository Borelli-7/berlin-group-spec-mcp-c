//! Content of the synthetic fixture PDFs shipped in `corpus/pdf`.
//!
//! All text is ORIGINAL SYNTHETIC TEST DATA written for this project. It paraphrases the
//! general shape of an account-information API so the pipeline can be exercised end to
//! end; it is not Berlin Group text and must not be used as a specification.

use super::fixture::{FixturePage, render_pdf};

const WRAP: usize = 92;
const DISCLAIMER: &str = "SYNTHETIC TEST FIXTURE. This document is original test data created for the bg-spec \
MCP server. It is not a Berlin Group publication and has no normative value.";

/// A fixture PDF: corpus-relative path, title and pages.
pub struct FixturePdf {
    pub path: &'static str,
    pub title: &'static str,
    pub pages: Vec<FixturePage>,
}

impl FixturePdf {
    pub fn render(&self) -> Vec<u8> {
        render_pdf(self.title, &self.pages)
    }
}

/// Builds a page from paragraphs; headings are single lines, paragraphs are word-wrapped
/// and separated by blank lines.
fn page(paragraphs: &[&str]) -> FixturePage {
    let mut lines = Vec::new();
    for (i, p) in paragraphs.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        let mut line = String::new();
        for word in p.split_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > WRAP {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    FixturePage { lines }
}

fn v2_implementation_guidelines() -> FixturePdf {
    FixturePdf {
        path: "pdf/v2/implementation-guidelines.pdf",
        title: "Open Finance Framework v2 - Implementation Guidelines (synthetic fixture)",
        pages: vec![
            page(&[
                "Open Finance Framework Version 2",
                "Implementation Guidelines - Account Information (synthetic fixture)",
                DISCLAIMER,
            ]),
            page(&[
                "1 Introduction",
                "This document describes the account information interface of the synthetic Open Finance \
                 Framework version 2. Version 2 extends version 1.3 and keeps the version 1.3 interface \
                 available for existing TPPs.",
                "2 Conventions",
                "The key words MUST, MUST NOT, SHALL, SHALL NOT, SHOULD and MAY are to be interpreted as \
                 normative requirement levels. Statements without these key words are informative.",
            ]),
            page(&[
                "4.2 Read Transaction List",
                "Call GET /accounts/{accountId}/transactions reads the transaction list of an addressed account.",
                "The query parameter bookingStatus is mandatory. The ASPSP SHALL support the values booked, \
                 pending and both. The ASPSP MAY support the values information and all. If the value \
                 information is used, the ASPSP SHALL return standing order information instead of transactions.",
                "The query parameter dateFrom is mandatory unless deltaList is used. The ASPSP SHALL reject a \
                 request without dateFrom with HTTP status 400 and error code FORMAT_ERROR.",
                "The ASPSP SHALL support the pagination of transaction lists. The TPP MAY limit the page size \
                 with the query parameter pageSize. The ASPSP SHALL return a link next when more entries exist.",
                "The ASPSP SHALL return HTTP status 429 with error code ACCESS_EXCEEDED when the access \
                 frequency limit of the consent is exceeded.",
                "Each returned transaction MUST contain transactionAmount and bookingDate. The bookingDate is \
                 returned as date-time in version 2.",
            ]),
            FixturePage::blank(),
            page(&[
                "4.3 Read Transaction Details",
                "Call GET /accounts/{accountId}/transactions/{transactionId} reads the details of one \
                 transaction. The ASPSP SHALL return HTTP status 404 if the transactionId is unknown.",
                "4.4 Read Balance",
                "Call GET /accounts/{accountId}/balances reads the balances of an account. The ASPSP SHALL \
                 return at least one balance of type closingBooked or interimAvailable.",
            ]),
            page(&[
                "5 Error Codes",
                "FORMAT_ERROR (400): the request does not match the syntax of the interface.",
                "PERIOD_INVALID (400): the requested period is outside the supported range.",
                "CONSENT_INVALID (401): the consent was created but is not valid for the addressed service.",
                "ACCESS_EXCEEDED (429): the access frequency limit of the consent is exceeded.",
            ]),
        ],
    }
}

fn v2_operational_rules() -> FixturePdf {
    FixturePdf {
        path: "pdf/v2/operational-rules.pdf",
        title: "Open Finance Framework v2 - Operational Rules (synthetic fixture)",
        pages: vec![
            page(&[
                "Open Finance Framework Version 2",
                "Operational Rules (synthetic fixture)",
                DISCLAIMER,
            ]),
            page(&[
                "3 Transaction History Access",
                "The ASPSP SHALL make at least 90 days of transaction history available through the \
                 transaction list endpoint.",
                "Without active PSU involvement the TPP MUST NOT read the transaction list of an account more \
                 than four times per calendar day. The limit is counted per account and per consent.",
            ]),
        ],
    }
}

fn v13_implementation_guidelines() -> FixturePdf {
    FixturePdf {
        path: "pdf/v1/implementation-guidelines-v1.3.pdf",
        title: "NextGenPSD2 v1.3 - Implementation Guidelines (synthetic fixture)",
        pages: vec![
            page(&[
                "NextGenPSD2 Framework Version 1.3",
                "Implementation Guidelines (synthetic fixture)",
                DISCLAIMER,
            ]),
            page(&[
                "6.3 Read Transaction List",
                "Call GET /v1/accounts/{account-id}/transactions reads the transaction list of an account.",
                "The query parameter bookingStatus is mandatory. The ASPSP SHALL support the values booked, \
                 pending and both.",
                "The query parameter dateFrom is conditional. It is mandatory unless deltaList is used.",
                "The ASPSP SHALL return HTTP status 400, 401, 403 or 404 on errors.",
            ]),
        ],
    }
}

/// All fixture PDFs of the example corpus.
pub fn fixture_pdfs() -> Vec<FixturePdf> {
    vec![
        v2_implementation_guidelines(),
        v2_operational_rules(),
        v13_implementation_guidelines(),
    ]
}
