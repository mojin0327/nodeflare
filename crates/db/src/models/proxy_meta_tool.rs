use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ProxyMetaTool {
    pub handler_type: String,
    pub name: String,
    pub description: String,
    pub input_schema: JsonValue,
    pub is_enabled: bool,
    pub sort_order: i32,
}
