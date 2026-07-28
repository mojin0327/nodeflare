use chrono::{DateTime, Utc};
use mcp_common::{McpMethod, ScopeChecker};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApiKey {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub server_id: Option<Uuid>,
    pub name: String,
    pub key_prefix: String,
    /// SECURITY: key_hash should never be serialized to API responses
    #[serde(skip_serializing)]
    pub key_hash: String,
    pub scopes: serde_json::Value,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ApiKey {
    pub fn scopes(&self) -> Vec<String> {
        serde_json::from_value(self.scopes.clone()).unwrap_or_default()
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        let scopes = self.scopes();
        scopes.iter().any(|s| s == scope || s == "*")
    }

    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            expires_at < Utc::now()
        } else {
            false
        }
    }

    /// Create a scope checker for this API key
    pub fn scope_checker(&self) -> ScopeChecker {
        ScopeChecker::new(&self.scopes())
    }

    /// Check if this API key is allowed to perform an MCP method
    pub fn is_method_allowed(&self, method: McpMethod, target: Option<&str>) -> bool {
        self.scope_checker().is_allowed(method, target)
    }

    /// Check if this API key can call a specific tool
    pub fn can_call_tool(&self, tool_name: &str) -> bool {
        self.is_method_allowed(McpMethod::ToolsCall, Some(tool_name))
    }

    /// Check if this API key can list tools
    pub fn can_list_tools(&self) -> bool {
        self.is_method_allowed(McpMethod::ToolsList, None)
    }

    /// Check if this API key can read a specific resource
    pub fn can_read_resource(&self, resource_uri: &str) -> bool {
        self.is_method_allowed(McpMethod::ResourcesRead, Some(resource_uri))
    }

    /// Check if this API key can list resources
    pub fn can_list_resources(&self) -> bool {
        self.is_method_allowed(McpMethod::ResourcesList, None)
    }

    /// Check if this API key can get a specific prompt
    pub fn can_get_prompt(&self, prompt_name: &str) -> bool {
        self.is_method_allowed(McpMethod::PromptsGet, Some(prompt_name))
    }

    /// Check if this API key can list prompts
    pub fn can_list_prompts(&self) -> bool {
        self.is_method_allowed(McpMethod::PromptsList, None)
    }
}

#[derive(Debug, Clone)]
pub struct CreateApiKey {
    pub workspace_id: Uuid,
    pub server_id: Option<Uuid>,
    pub name: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}
