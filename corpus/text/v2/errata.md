# openFinance API Framework v2 - Project Errata and Clarifications

PROJECT NOTE. Curated by the implementation team; not a Berlin Group publication and without
normative value. Each entry records a discrepancy observed between the official files of this
corpus and the reading adopted by the project. Page numbers are physical PDF pages (as indexed),
not the page numbers printed in the document footer.

## E-01 ASPSP-SCA-Approach value SIGNATURE missing from the OpenAPI enum

Protocol Functions and Security Measures 2.3, section 8.4.2 (page 95), lists the values
EMBEDDED, DECOUPLED, REDIRECT, ASPSP-CHANNEL and SIGNATURE for the ASPSP-SCA-Approach
response header. Consent API 2.2 uses "ASPSP-SCA-Approach: SIGNATURE" in an example (page 32).
The header definitions in the Signing Basket 2.3, Consent 2.1 and PIS 2.3 OpenAPI files list
SIGNATURE in the description, but their enum only contains EMBEDDED, DECOUPLED, REDIRECT and
ASPSP-CHANNEL.

Clarification: the PDF value list prevails. SIGNATURE is a valid ASPSP-SCA-Approach value;
clients generated from the OpenAPI enum must tolerate it.

## E-02 SCA Approach Type codes in the Data Dictionary differ from header values

Data Dictionary 2.3.1, section 3.39 SCA Approach Type (page 213), and the schema
SCAApproachType of the Data Dictionary OpenAPI 2.3.1 define the codes Decoupled, Embedded,
OAauth2, Redirect and Online-Channel. The header values of E-01 use ASPSP-CHANNEL instead of
Online-Channel, subsume OAuth under REDIRECT and add SIGNATURE. "OAauth2" is spelled as
published.

Clarification: SCAApproachType codes are used in data structures only. Do not map them 1:1 onto
the ASPSP-SCA-Approach header; Online-Channel corresponds to ASPSP-CHANNEL.

## E-03 pageSize query parameter not declared in the AIS OpenAPI

XS2A API Implementation Guidelines 2.4, section 4.4.4 Read Transaction List (page 96), defines
the optional query parameter pageSize ("Optional if supported by API provider"; rejected if not
supported or if higher than maxPageSize). Data Dictionary 2.3.1 (page 99) lists "pageSize" as a
value of supportedQueryParametersTransactions and defines maxPageSize. The AIS OpenAPI 2.3
operation GET /v2/accounts/{account-id}/transactions does not declare pageSize. NextGenPSD2
1.3.16 has no pageSize parameter at all.

Clarification: implement pageSize as described in the Implementation Guidelines when supported
and advertise it together with maxPageSize. The OpenAPI contract alone is incomplete here.

## E-04 Document version skew between PDFs and OpenAPI files

The XS2A API Implementation Guidelines are version 2.4 (change log section 7.3, page 127) while
the AIS, PIS and PIIS OpenAPI files declare info.version 2.3. The Consent API PDF is version 2.2
(change log section 5.2, page 76) while the Consent OpenAPI declares info.version 2.1, although
it already contains 2.2 content such as the "userParameters" access right (renamed from
"psuParameters" in 2.2). The PIIS OpenAPI is dated 2026-01-07; the other v2
OpenAPI files are dated 2026-02-04.

Clarification: info.version is not a reliable alignment indicator. Compare contracts by content.

## E-05 Consent validity attribute renamed from validUntil to validTo

NextGenPSD2 1.3.16 (Implementation Guidelines section 6.3.1, page 155, and OpenAPI POST
/v1/consents) requires validUntil and combinedServiceIndicator. openFinance Consent API 2.2,
section 3.4.1 (page 25), and the Consent OpenAPI POST /v2/consents/account-access require
validTo and consentType instead; combinedServiceIndicator no longer exists.

Clarification: migration from v1 to v2 must rename validUntil to validTo and derive consentType;
semantics (inclusive date, "9999-12-31" for maximum validity) are unchanged.

## E-06 Two different funds-confirmation consent endpoints under /v2

The NextGenPSD2 extended service "Confirmation of Funds Consent" 2.0 (corpus folder openapi/v1)
publishes /v2/consents/confirmation-of-funds. openFinance Consent API 2.2, section 3.5.1
(pages 38-39), defines /v2/consents/funds-confirmations and states that the earlier endpoint
was adapted under the new name for the "access" and "consentType" data model.

Clarification: both paths start with /v2 but belong to different frameworks. Only
/v2/consents/funds-confirmations belongs to openFinance v2.

## E-07 Counting of AIS accesses without PSU involvement

NextGenPSD2 1.3.16 limits frequencyPerDay to at most 4 unless agreed bilaterally (page 155) and
rejects excess accesses with HTTP 429 ACCESS_EXCEEDED (page 309). openFinance keeps the same
frequencyPerDay rule (Consent API 2.2, pages 25-26) but the Data Dictionary 2.3.1 (page 100)
lets the ASPSP declare the counting method in countingOfAisRequests: "none", "timeslot"
(aggregated accesses in a time interval count once) or "byEndpointCall" (4 accesses per day per
endpoint).

Clarification: the counting unit is not fixed by the framework. The project counts according
to the value it publishes in countingOfAisRequests and returns 429 ACCESS_EXCEEDED when exceeded.

## E-08 TPP-prefixed names replaced by Client-prefixed names

Protocol Functions and Security Measures 2.3 renames TPP-Redirect-URI to Client-Redirect-URI
(change log, page 204); the Consent API 2.1 change log (page 75) applies the same rename to
TPP-SCA-Preference and TPP-Explicit-Authorisation-Preferred. The v2 PIS OpenAPI uses
Client-Redirect-URI while NextGenPSD2 1.3.16 uses TPP-Redirect-URI. The push notification
target changed from POST /TPP-notification-URI (NextGenPSD2 lean push 1.0.0) to
POST /Client-Notification-URL (openFinance Resource Status Notification 2.3, section 12.5,
page 196).

Clarification: v2 endpoints accept only the Client-prefixed names; any v1 compatibility layer
must translate the headers explicitly.

## E-09 Multilevel SCA transaction status spelled PACT once in the Implementation Guidelines

XS2A API Implementation Guidelines 2.4, section 2.3.1 Status Information for PIS, introduces the
status for partially authorised payments as "PACT" (PartiallyAcceptedTechnicalCorrect, page 18)
but uses "PATC" for the same status two pages later (page 20). The Data Dictionary 2.3.1
Transaction Status code list (page 223) and the TransactionStatus enums of the v2 AIS, PIS,
Consent, Signing Basket, Push and Data Dictionary OpenAPI files only contain PATC.

Clarification: PATC is the only valid code. PACT is a typo in the Implementation Guidelines.

## E-10 Change logs name the SCA preference and negative redirect headers differently

The change log of Protocol Functions and Security Measures 2.3 (pages 204-205) records the
renames TPP-SCA-Preference to Client-SCA-Preference and TPP-Nok-Redirect-URI to Client-Nok-URI;
the Consent API 2.2 change log (page 75) also names Client-SCA-Preference. The normative header
tables of Protocol Functions section 8.4.1 (pages 93-94) and the v2 OpenAPI files use
Client-SCA-Approach-Preference and Client-Nok-Redirect-URI.

Clarification: implement the header names of section 8.4.1 and the OpenAPI files
(Client-SCA-Approach-Preference, Client-Nok-Redirect-URI). The change log names are not used.
