# Open Finance Framework v2 - Errata and Clarifications (synthetic fixture)

SYNTHETIC TEST FIXTURE. Original test data created for the bg-spec MCP server; not a
Berlin Group publication and without normative value.

## E-07 Transaction list access frequency

Clarification: the limit of four transaction list reads per calendar day without PSU
involvement is counted per consent only, not per account.

## E-09 bookingStatus value all

The bookingStatus value all returns booked, pending and information entries in one
response. ASPSPs that do not support information SHALL reject the value all with
HTTP status 400 and error code FORMAT_ERROR.
