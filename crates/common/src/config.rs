use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AppConfig {
    pub environment: Environment,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub auth: AuthConfig,
    pub github: GithubConfig,
    pub google: GoogleConfig,
    pub container: ContainerConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl Default for Environment {
    fn default() -> Self {
        Self::Development
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub proxy_port: u16,
    pub builder_port: u16,
    pub frontend_url: String,
    /// Base domain for proxy (e.g., "nodeflare.tech" -> {slug}.nodeflare.tech)
    pub proxy_base_domain: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            proxy_port: 8081,
            builder_port: 8082,
            frontend_url: "http://localhost:3000".to_string(),
            proxy_base_domain: "nodeflare.tech".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    /// Connection acquire timeout in seconds
    pub acquire_timeout_secs: u64,
    /// Idle connection timeout in seconds
    pub idle_timeout_secs: u64,
    /// Maximum lifetime of a connection in seconds (prevents stale connections)
    pub max_lifetime_secs: u64,
    /// Whether to test connections before acquiring from pool
    pub test_before_acquire: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgres://postgres:postgres@localhost:5432/mcp_cloud".to_string(),
            // Neon serverless optimized settings:
            // - Neon compute scales to zero after 5 min (300s) of inactivity
            // - When compute sleeps, all connections are closed server-side
            // - We need to close connections before Neon does
            max_connections: 20,
            // CRITICAL: Set to 0 for Neon serverless
            // With min_connections > 0, app maintains idle connections that become
            // invalid when Neon compute sleeps, causing "stale connection" errors
            min_connections: 0,
            // Neon cold start can take 500ms-few seconds
            acquire_timeout_secs: 10,
            // Close idle connections well before Neon's 5 min sleep threshold
            idle_timeout_secs: 60,
            // Recycle connections every 10 min to prevent long-lived connection issues
            max_lifetime_secs: 600,
            // Test connections before use (catches stale connections from Neon sleep)
            test_before_acquire: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisConfig {
    pub url: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub refresh_token_expiration_days: i64,
    /// Maximum absolute session lifetime in days.
    /// After this time, users must re-authenticate regardless of activity.
    /// This prevents sessions from being extended indefinitely.
    #[serde(default = "default_absolute_session_lifetime_days")]
    pub absolute_session_lifetime_days: i64,
}

fn default_absolute_session_lifetime_days() -> i64 {
    30 // 30 days maximum session lifetime
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt_secret: String::new(), // Must be set via environment variable
            jwt_expiration_hours: 1, // 1 hour - short-lived for security
            refresh_token_expiration_days: 14, // 14 days of inactivity before logout
            absolute_session_lifetime_days: 30, // Max 30 days total session lifetime
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub app_id: String,
    pub app_private_key: String,
    pub webhook_secret: String,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: String::new(),
            app_id: String::new(),
            app_private_key: String::new(),
            webhook_secret: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl Default for GoogleConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContainerConfig {
    /// Base registry path, e.g. `ghcr.io/your-org`
    pub registry_base: String,
    /// PAT / token for authenticating to the container registry
    pub registry_token: String,
    /// Directory where Unix Domain Sockets for MCP containers are created (e.g. `/var/run/mcp`)
    pub socket_dir: String,
    /// Host directory for persistent container data (e.g. `/var/lib/mcp-data`)
    pub data_dir: String,
    /// URL of the Builder service's internal HTTP API (used by API to exec/stats)
    pub builder_internal_url: String,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            registry_base: String::new(),
            registry_token: String::new(),
            socket_dir: "/var/run/mcp".to_string(),
            data_dir: "/var/lib/mcp-data".to_string(),
            builder_internal_url: "http://localhost:8083".to_string(),
        }
    }
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let environment = env::var("ENVIRONMENT")
            .unwrap_or_else(|_| "development".to_string())
            .parse()
            .unwrap_or(Environment::Development);

        Ok(Self {
            environment,
            server: ServerConfig {
                host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
                port: env::var("PORT")
                    .unwrap_or_else(|_| "8080".to_string())
                    .parse()?,
                proxy_port: env::var("PROXY_PORT")
                    .unwrap_or_else(|_| "8081".to_string())
                    .parse()?,
                builder_port: env::var("BUILDER_PORT")
                    .unwrap_or_else(|_| "8082".to_string())
                    .parse()?,
                frontend_url: env::var("FRONTEND_URL")
                    .unwrap_or_else(|_| "http://localhost:3000".to_string()),
                proxy_base_domain: env::var("PROXY_BASE_DOMAIN")
                    .unwrap_or_else(|_| "mcp.cloud".to_string()),
            },
            database: DatabaseConfig {
                url: env::var("DATABASE_URL")
                    .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/mcp_cloud".to_string()),
                // Scalability: Default increased to 100 for production workloads
                max_connections: env::var("DATABASE_MAX_CONNECTIONS")
                    .unwrap_or_else(|_| "100".to_string())
                    .parse()?,
                min_connections: env::var("DATABASE_MIN_CONNECTIONS")
                    .unwrap_or_else(|_| "10".to_string())
                    .parse()?,
                acquire_timeout_secs: env::var("DATABASE_ACQUIRE_TIMEOUT_SECS")
                    .unwrap_or_else(|_| "30".to_string())
                    .parse()?,
                // Reduced from 600s to 300s for faster connection recycling
                idle_timeout_secs: env::var("DATABASE_IDLE_TIMEOUT_SECS")
                    .unwrap_or_else(|_| "300".to_string())
                    .parse()?,
                max_lifetime_secs: env::var("DATABASE_MAX_LIFETIME_SECS")
                    .unwrap_or_else(|_| "1800".to_string())
                    .parse()?,
                test_before_acquire: env::var("DATABASE_TEST_BEFORE_ACQUIRE")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(true),
            },
            redis: RedisConfig {
                url: env::var("REDIS_URL")
                    .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            },
            auth: AuthConfig {
                jwt_secret: {
                    let secret = env::var("JWT_SECRET")
                        .map_err(|_| anyhow::anyhow!("JWT_SECRET environment variable is required"))?;
                    // Minimum 32 characters for security (256 bits)
                    if secret.len() < 32 {
                        return Err(anyhow::anyhow!(
                            "JWT_SECRET must be at least 32 characters long for security. Current length: {}",
                            secret.len()
                        ));
                    }

                    // SECURITY: Check for weak/default secrets
                    let weak_patterns = [
                        "secret",
                        "password",
                        "changeme",
                        "default",
                        "123456",
                        "your-secret",
                        "jwt-secret",
                        "supersecret",
                    ];
                    let lower_secret = secret.to_lowercase();
                    for pattern in &weak_patterns {
                        if lower_secret.contains(pattern) {
                            return Err(anyhow::anyhow!(
                                "JWT_SECRET appears to contain a weak/default pattern '{}'. Use a cryptographically random value.",
                                pattern
                            ));
                        }
                    }

                    // Check for low entropy (repeated characters)
                    let unique_chars: std::collections::HashSet<char> = secret.chars().collect();
                    if unique_chars.len() < 10 {
                        return Err(anyhow::anyhow!(
                            "JWT_SECRET has low entropy (only {} unique characters). Use a cryptographically random value with high entropy.",
                            unique_chars.len()
                        ));
                    }

                    secret
                },
                jwt_expiration_hours: env::var("JWT_EXPIRATION_HOURS")
                    .unwrap_or_else(|_| "1".to_string())
                    .parse()?,
                refresh_token_expiration_days: env::var("REFRESH_TOKEN_EXPIRATION_DAYS")
                    .unwrap_or_else(|_| "14".to_string())
                    .parse()?,
                absolute_session_lifetime_days: env::var("ABSOLUTE_SESSION_LIFETIME_DAYS")
                    .unwrap_or_else(|_| "30".to_string())
                    .parse()?,
            },
            github: GithubConfig {
                client_id: env::var("GITHUB_CLIENT_ID").unwrap_or_default(),
                client_secret: env::var("GITHUB_CLIENT_SECRET").unwrap_or_default(),
                redirect_uri: env::var("GITHUB_REDIRECT_URI").unwrap_or_default(),
                app_id: env::var("GITHUB_APP_ID").unwrap_or_default(),
                app_private_key: env::var("GITHUB_APP_PRIVATE_KEY").unwrap_or_default(),
                webhook_secret: env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default(),
            },
            google: GoogleConfig {
                client_id: env::var("GOOGLE_CLIENT_ID").unwrap_or_default(),
                client_secret: env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default(),
                redirect_uri: env::var("GOOGLE_REDIRECT_URI").unwrap_or_default(),
            },
            container: ContainerConfig {
                registry_base: env::var("CONTAINER_REGISTRY_BASE").unwrap_or_default(),
                registry_token: env::var("CONTAINER_REGISTRY_TOKEN").unwrap_or_default(),
                socket_dir: env::var("CONTAINER_SOCKET_DIR")
                    .unwrap_or_else(|_| "/var/run/mcp".to_string()),
                data_dir: env::var("CONTAINER_DATA_DIR")
                    .unwrap_or_else(|_| "/var/lib/mcp-data".to_string()),
                builder_internal_url: env::var("BUILDER_INTERNAL_URL")
                    .unwrap_or_else(|_| "http://localhost:8083".to_string()),
            },
        })
    }

    pub fn is_production(&self) -> bool {
        self.environment == Environment::Production
    }
}

impl std::str::FromStr for Environment {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "development" | "dev" => Ok(Self::Development),
            "staging" | "stg" => Ok(Self::Staging),
            "production" | "prod" => Ok(Self::Production),
            _ => Err(format!("Unknown environment: {}", s)),
        }
    }
}
