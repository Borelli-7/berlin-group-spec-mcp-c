use bg_spec_core::CoreError;
use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};

/// Tool failure mapping.
///
/// * Caller errors (invalid input, invalid locator, not found) become a tool result with
///   `isError: true` and a JSON body `{error: {code, message}}` so the agent can self-correct.
/// * Internal failures (storage, search, integrity) become JSON-RPC internal errors.
#[derive(Debug)]
pub struct ToolError(pub CoreError);

impl From<CoreError> for ToolError {
    fn from(e: CoreError) -> Self {
        Self(e)
    }
}

impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        let err = self.0;
        if err.is_client_error() {
            tracing::debug!(code = err.code(), error = %err, "tool rejected request");
            let body = serde_json::json!({ "error": { "code": err.code(), "message": err.to_string() } });
            Ok(CallToolResult::error(vec![ContentBlock::text(body.to_string())]).into())
        } else {
            tracing::error!(code = err.code(), error = %err, "tool failed");
            Err(ErrorData::internal_error(
                err.to_string(),
                Some(serde_json::json!({ "code": err.code() })),
            ))
        }
    }
}
