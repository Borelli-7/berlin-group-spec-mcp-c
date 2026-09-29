use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
    Unchanged,
}

/// Which side of the HTTP exchange a change affects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSide {
    Endpoint,
    Request,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeArea {
    Endpoint,
    Path,
    OperationId,
    Deprecated,
    Tags,
    Parameter,
    RequestBody,
    Response,
    ResponseHeader,
    MediaType,
    Security,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaChangeKind {
    SchemaAdded,
    SchemaRemoved,
    PropertyAdded,
    PropertyRemoved,
    TypeChanged,
    FormatChanged,
    NullableChanged,
    /// Property became required.
    RequiredAdded,
    /// Property is no longer required.
    RequiredRemoved,
    EnumValuesAdded,
    EnumValuesRemoved,
    /// Enum constraint introduced or dropped entirely.
    EnumConstraintChanged,
    RefChanged,
    CompositionChanged,
    ConstraintChanged,
    AdditionalPropertiesChanged,
}

/// One structural schema difference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchemaChange {
    /// Location inside the compared schema, e.g. `/properties/bookingStatus/enum`.
    pub pointer: String,
    pub kind: SchemaChangeKind,
    pub change: ChangeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    /// Component schema in which the change was detected (if within a named schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_name: Option<String>,
}

/// One structural difference between two operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompatibilityChange {
    pub area: ChangeArea,
    pub side: ChangeSide,
    pub change: ChangeType,
    /// Subject of the change, e.g. `query:bookingStatus`, `response:200`, `application/json`.
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schema_changes: Vec<SchemaChange>,
}
