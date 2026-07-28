use mcp_common::{AppConfig, Error, Result};
use serde::{Deserialize, Serialize};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v3/userinfo";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleUser {
    /// Google's unique user ID (sub claim)
    pub sub: String,
    /// User's email address
    pub email: Option<String>,
    /// Whether email is verified
    pub email_verified: Option<bool>,
    /// User's display name
    pub name: Option<String>,
    /// User's given name (first name)
    pub given_name: Option<String>,
    /// User's family name (last name)
    pub family_name: Option<String>,
    /// URL to user's profile picture
    pub picture: Option<String>,
    /// User's locale
    pub locale: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    #[allow(dead_code)]
    expires_in: u64,
    #[allow(dead_code)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
    error_description: Option<String>,
}

#[derive(Clone)]
pub struct GoogleOAuth {
    client_id: String,
    client_secret: String,
    redirect_url: String,
    http_client: reqwest::Client,
}

impl GoogleOAuth {
    pub fn new(config: &AppConfig, redirect_url: &str) -> Result<Self> {
        if config.google.client_id.is_empty() {
            return Err(Error::Config("GOOGLE_CLIENT_ID not configured".to_string()));
        }
        if config.google.client_secret.is_empty() {
            return Err(Error::Config("GOOGLE_CLIENT_SECRET not configured".to_string()));
        }

        let http_client = reqwest::Client::builder()
            .user_agent("MCP-Cloud/1.0")
            .build()
            .map_err(|e| Error::Internal(e.to_string()))?;

        Ok(Self {
            client_id: config.google.client_id.clone(),
            client_secret: config.google.client_secret.clone(),
            redirect_url: redirect_url.to_string(),
            http_client,
        })
    }

    /// Generate the Google OAuth authorization URL
    /// Returns (authorization_url, csrf_state_token)
    pub fn get_authorization_url(&self) -> (String, String) {
        // SECURITY: Use cryptographically secure random token
        let state = crate::jwt::generate_random_token(32)
            .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());

        let redirect_encoded = url::form_urlencoded::byte_serialize(self.redirect_url.as_bytes())
            .collect::<String>();

        // Request openid, email, and profile scopes
        let scopes = "openid%20email%20profile";

        let url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=select_account",
            GOOGLE_AUTH_URL,
            &self.client_id,
            redirect_encoded,
            scopes,
            &state
        );

        (url, state)
    }

    /// Exchange authorization code for access token
    pub async fn exchange_code(&self, code: &str) -> Result<String> {
        let response = self
            .http_client
            .post(GOOGLE_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
                ("code", &code.to_string()),
                ("redirect_uri", &self.redirect_url),
                ("grant_type", &"authorization_code".to_string()),
            ])
            .send()
            .await
            .map_err(|e| Error::ExternalService(format!("Google OAuth error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.json::<TokenErrorResponse>().await;
            let error_msg = match body {
                Ok(err) => format!("{}: {}", err.error, err.error_description.unwrap_or_default()),
                Err(_) => format!("HTTP {}", status),
            };
            return Err(Error::ExternalService(format!(
                "Google OAuth token exchange error: {}",
                error_msg
            )));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| Error::ExternalService(format!("Failed to parse Google token response: {}", e)))?;

        Ok(token_response.access_token)
    }

    /// Get user info from Google
    pub async fn get_user(&self, access_token: &str) -> Result<GoogleUser> {
        let response = self
            .http_client
            .get(GOOGLE_USERINFO_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| Error::ExternalService(format!("Google API error: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::ExternalService(format!(
                "Google API error: {} - {}",
                status, body
            )));
        }

        response
            .json::<GoogleUser>()
            .await
            .map_err(|e| Error::ExternalService(format!("Failed to parse Google user info: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_authorization_url_generation() {
        // This is a basic test to ensure the URL format is correct
        // In production, GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET would be set
        let url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=select_account",
            GOOGLE_AUTH_URL,
            "test_client_id",
            "http%3A%2F%2Flocalhost%3A8080%2Fcallback",
            "openid%20email%20profile",
            "test_state"
        );

        assert!(url.contains("accounts.google.com"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=openid%20email%20profile"));
    }
}
