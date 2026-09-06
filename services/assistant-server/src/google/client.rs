//! Concrete Google API client and provider implementation.
//!
//! Implements `GmailProvider` and `CalendarProvider` using standard HTTPS
//! calls to Google's REST endpoints, authenticating with server-held,
//! AES-256-GCM encrypted OAuth credentials. Secrets and raw tokens never
//! leave the server boundary or appear in log streams.

use async_trait::async_trait;
use reqwest::{Client, StatusCode as HttpStatusCode};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use assistant_protocol::{
    AccountSummary, CalendarEvent, CreateEventRequest as CreateCalendarEvent, EmailDetail,
    EmailSummary,
};
use assistant_tools::{CalendarProvider, GmailProvider, ToolError, providers::UpdateCalendarEvent};

use crate::crypto;

const GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const CALENDAR_EVENTS_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";
const USERINFO_EMAIL_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email";
const USERINFO_PROFILE_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.profile";

// Milestone 6. Every one is `.readonly`, and every Classroom scope is the
// `.me` variant: this application reads the signed-in student's own academic
// information and has no teacher or administrator capability. See ADR-0032.
const CLASSROOM_COURSES_SCOPE: &str = "https://www.googleapis.com/auth/classroom.courses.readonly";
const CLASSROOM_COURSEWORK_SCOPE: &str =
    "https://www.googleapis.com/auth/classroom.coursework.me.readonly";
const CLASSROOM_ANNOUNCEMENTS_SCOPE: &str =
    "https://www.googleapis.com/auth/classroom.announcements.readonly";
// `drive.readonly` is a Google *restricted* scope: a published application
// using it must pass OAuth verification and a security assessment. It is
// requested anyway because the alternative, `drive.file`, only ever grants
// access to files the user has individually picked, which cannot answer
// "search my Drive" at all. See ADR-0033.
const DRIVE_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";

/// Every scope this application asks for, in one place.
///
/// Consent is all-or-nothing per connection rather than incremental: an
/// account connected before Milestone 6 holds only the Gmail and Calendar
/// scopes, and Classroom or Drive calls on it will fail with a 403 until the
/// user reconnects it. `AccountSummary::scopes` carries what was actually
/// granted so the UI can say which account needs reconnecting instead of
/// letting the feature fail silently.
pub const REQUESTED_SCOPES: &[&str] = &[
    USERINFO_EMAIL_SCOPE,
    USERINFO_PROFILE_SCOPE,
    GMAIL_READONLY_SCOPE,
    CALENDAR_EVENTS_SCOPE,
    CLASSROOM_COURSES_SCOPE,
    CLASSROOM_COURSEWORK_SCOPE,
    CLASSROOM_ANNOUNCEMENTS_SCOPE,
    DRIVE_READONLY_SCOPE,
];

/// Scopes a feature needs, for telling the user which account to reconnect.
pub const CLASSROOM_SCOPES: &[&str] = &[
    CLASSROOM_COURSES_SCOPE,
    CLASSROOM_COURSEWORK_SCOPE,
    CLASSROOM_ANNOUNCEMENTS_SCOPE,
];

pub const DRIVE_SCOPES: &[&str] = &[DRIVE_READONLY_SCOPE];

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredGoogleTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

#[derive(Clone)]
pub struct GoogleClient {
    // `pub(super)` and no wider: the Classroom and Drive providers are sibling
    // modules inside `google` and need the pool, the HTTP client and a fresh
    // access token. Nothing outside this module can reach them.
    pub(super) pool: PgPool,
    pub(super) http: Client,
    client_id: Option<String>,
    client_secret: Option<String>,
    encryption_key: [u8; 32],
}

impl GoogleClient {
    pub fn new(
        pool: PgPool,
        http: Client,
        client_id: Option<String>,
        client_secret: Option<String>,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            pool,
            http,
            client_id,
            client_secret,
            encryption_key,
        }
    }

    /// Generates the Google OAuth authorization URL for a specific user.
    pub fn generate_auth_url(
        &self,
        user_id: Uuid,
        redirect_uri: &str,
    ) -> Result<String, ToolError> {
        let client_id = self
            .client_id
            .as_deref()
            .ok_or_else(|| ToolError::Failed("GOOGLE_CLIENT_ID is not configured".into()))?;

        // Encrypt state payload with AES-256-GCM to prevent CSRF and tampering
        let state_payload = serde_json::json!({
            "user_id": user_id,
            "nonce": Uuid::new_v4(),
            "created_at": OffsetDateTime::now_utc().unix_timestamp(),
        });
        let state_bytes =
            serde_json::to_vec(&state_payload).map_err(|e| ToolError::Failed(e.to_string()))?;
        let encrypted_state = crypto::encrypt(&state_bytes, &self.encryption_key)
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        let state_param = encode_hex(&encrypted_state);

        let scopes = REQUESTED_SCOPES.join(" ");

        let encoded_scopes = url_encode(&scopes);
        let encoded_redirect = url_encode(redirect_uri);

        let url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth?\
            client_id={client_id}&\
            redirect_uri={encoded_redirect}&\
            response_type=code&\
            scope={encoded_scopes}&\
            access_type=offline&\
            prompt=consent&\
            state={state_param}"
        );

        Ok(url)
    }

    /// Verifies the OAuth state parameter and returns the authorized user_id.
    pub fn verify_oauth_state(&self, state: &str) -> Result<Uuid, ToolError> {
        let raw_bytes = decode_hex(state)
            .ok_or_else(|| ToolError::Failed("Invalid OAuth state parameter".into()))?;
        let decrypted = crypto::decrypt(&raw_bytes, &self.encryption_key)
            .map_err(|_| ToolError::Failed("Failed to decrypt OAuth state".into()))?;
        let val: serde_json::Value = serde_json::from_slice(&decrypted)
            .map_err(|_| ToolError::Failed("Malformed OAuth state".into()))?;

        let created_at = val.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);
        let now = OffsetDateTime::now_utc().unix_timestamp();
        // State expires after 15 minutes (900 seconds)
        if now - created_at > 900 {
            return Err(ToolError::Failed("OAuth state expired".into()));
        }

        let user_id_str = val
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Failed("Missing user_id in state".into()))?;

        Uuid::parse_str(user_id_str)
            .map_err(|_| ToolError::Failed("Invalid user_id in state".into()))
    }

    /// Exchanges an OAuth authorization code for tokens and registers the connected account.
    pub async fn exchange_code(
        &self,
        user_id: Uuid,
        code: &str,
        redirect_uri: &str,
    ) -> Result<AccountSummary, ToolError> {
        let client_id = self
            .client_id
            .as_deref()
            .ok_or_else(|| ToolError::Failed("GOOGLE_CLIENT_ID is not configured".into()))?;
        let client_secret = self
            .client_secret
            .as_deref()
            .ok_or_else(|| ToolError::Failed("GOOGLE_CLIENT_SECRET is not configured".into()))?;

        let body_str = format!(
            "code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
            url_encode(code),
            url_encode(client_id),
            url_encode(client_secret),
            url_encode(redirect_uri)
        );

        let token_resp = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body_str)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if !token_resp.status().is_success() {
            let status = token_resp.status();
            let err_text = token_resp.text().await.unwrap_or_default();
            tracing::error!(status = %status, "Google token exchange failed");
            return Err(ToolError::Failed(format!(
                "Google token exchange failed with {status}: {err_text}"
            )));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: i64,
            scope: Option<String>,
        }

        let token_data: TokenResponse = token_resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        // Fetch user profile
        let userinfo_resp = self
            .http
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .bearer_auth(&token_data.access_token)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        #[derive(Deserialize)]
        struct UserInfo {
            id: String,
            email: String,
            name: Option<String>,
        }

        let user_info: UserInfo = userinfo_resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let expires_at = OffsetDateTime::now_utc().unix_timestamp() + token_data.expires_in;
        let stored_tokens = StoredGoogleTokens {
            access_token: token_data.access_token,
            refresh_token: token_data.refresh_token,
            expires_at,
        };

        let token_json =
            serde_json::to_vec(&stored_tokens).map_err(|e| ToolError::Failed(e.to_string()))?;
        let encrypted_credentials = crypto::encrypt(&token_json, &self.encryption_key)
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let scopes: Vec<String> = token_data
            .scope
            .unwrap_or_default()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        // Upsert into connected_accounts
        let row = sqlx::query(
            r#"
            INSERT INTO connected_accounts
                (user_id, provider, provider_account_id, email, display_name, scopes, encrypted_credentials, status)
            VALUES
                ($1, 'google', $2, $3, $4, $5, $6, 'active')
            ON CONFLICT (user_id, provider, provider_account_id)
            DO UPDATE SET
                email = EXCLUDED.email,
                display_name = EXCLUDED.display_name,
                scopes = EXCLUDED.scopes,
                encrypted_credentials = EXCLUDED.encrypted_credentials,
                status = 'active',
                updated_at = NOW()
            RETURNING id, user_id, provider, provider_account_id, email, display_name, scopes, status, created_at, updated_at
            "#
        )
        .bind(user_id)
        .bind(&user_info.id)
        .bind(&user_info.email)
        .bind(&user_info.name)
        .bind(&scopes)
        .bind(&encrypted_credentials)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?;

        Ok(AccountSummary {
            id: row.get("id"),
            user_id: row.get("user_id"),
            provider: row.get("provider"),
            provider_account_id: row.get("provider_account_id"),
            email: row.get("email"),
            display_name: row.get("display_name"),
            scopes: row.get("scopes"),
            status: row.get("status"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
    }

    /// Lists all connected accounts owned by the user.
    pub async fn list_accounts(&self, user_id: Uuid) -> Result<Vec<AccountSummary>, ToolError> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, provider, provider_account_id, email, display_name, scopes, status, created_at, updated_at
            FROM connected_accounts
            WHERE user_id = $1 AND provider = 'google'
            ORDER BY created_at ASC
            "#
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?;

        let accounts = rows
            .into_iter()
            .map(|r| AccountSummary {
                id: r.get("id"),
                user_id: r.get("user_id"),
                provider: r.get("provider"),
                provider_account_id: r.get("provider_account_id"),
                email: r.get("email"),
                display_name: r.get("display_name"),
                scopes: r.get("scopes"),
                status: r.get("status"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect();

        Ok(accounts)
    }

    /// Disconnects a connected account, revoking local access.
    pub async fn disconnect_account(
        &self,
        user_id: Uuid,
        account_id: Uuid,
    ) -> Result<(), ToolError> {
        let res = sqlx::query(
            r#"
            UPDATE connected_accounts
            SET status = 'disconnected', updated_at = NOW()
            WHERE id = $1 AND user_id = $2
            "#,
        )
        .bind(account_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(ToolError::NotFound(format!(
                "Account {account_id} not found"
            )));
        }

        Ok(())
    }

    /// Retrieves an unexpired access token, transparently refreshing if expired.
    pub(super) async fn get_access_token(
        &self,
        user_id: Uuid,
        account_id: Uuid,
    ) -> Result<String, ToolError> {
        let row = sqlx::query(
            r#"
            SELECT id, encrypted_credentials, status
            FROM connected_accounts
            WHERE id = $1 AND user_id = $2 AND provider = 'google'
            "#,
        )
        .bind(account_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?;

        let Some(row) = row else {
            return Err(ToolError::NotFound(format!(
                "Account {account_id} not found"
            )));
        };

        let status: String = row.get("status");
        if status != "active" {
            return Err(ToolError::Failed(format!(
                "Account {account_id} is in status '{status}'"
            )));
        }

        let encrypted: Vec<u8> = row.get("encrypted_credentials");
        let decrypted = crypto::decrypt(&encrypted, &self.encryption_key)
            .map_err(|e| ToolError::Failed(format!("Failed to decrypt credentials: {e}")))?;

        let mut tokens: StoredGoogleTokens = serde_json::from_slice(&decrypted)
            .map_err(|e| ToolError::Failed(format!("Corrupted credentials: {e}")))?;

        let now = OffsetDateTime::now_utc().unix_timestamp();
        // If token expires in less than 60 seconds, refresh it
        if now + 60 >= tokens.expires_at {
            let Some(ref refresh_token) = tokens.refresh_token else {
                let _ = sqlx::query(
                    "UPDATE connected_accounts SET status = 'disconnected', updated_at = NOW() WHERE id = $1"
                )
                .bind(account_id)
                .execute(&self.pool)
                .await;
                return Err(ToolError::Failed(
                    "Token expired and no refresh token available".into(),
                ));
            };

            let client_id = self
                .client_id
                .as_deref()
                .ok_or_else(|| ToolError::Failed("GOOGLE_CLIENT_ID not configured".into()))?;
            let client_secret = self
                .client_secret
                .as_deref()
                .ok_or_else(|| ToolError::Failed("GOOGLE_CLIENT_SECRET not configured".into()))?;

            let body_str = format!(
                "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
                url_encode(client_id),
                url_encode(client_secret),
                url_encode(refresh_token)
            );

            let resp = self
                .http
                .post("https://oauth2.googleapis.com/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body_str)
                .send()
                .await
                .map_err(|e| ToolError::Failed(e.to_string()))?;

            if !resp.status().is_success() {
                let _ = sqlx::query(
                    "UPDATE connected_accounts SET status = 'error', updated_at = NOW() WHERE id = $1"
                )
                .bind(account_id)
                .execute(&self.pool)
                .await;
                return Err(ToolError::Failed(
                    "Failed to refresh Google token; re-authentication required".into(),
                ));
            }

            #[derive(Deserialize)]
            struct RefreshResp {
                access_token: String,
                expires_in: i64,
            }

            let refresh_data: RefreshResp = resp
                .json()
                .await
                .map_err(|e| ToolError::Failed(e.to_string()))?;

            tokens.access_token = refresh_data.access_token;
            tokens.expires_at = now + refresh_data.expires_in;

            let updated_bytes =
                serde_json::to_vec(&tokens).map_err(|e| ToolError::Failed(e.to_string()))?;
            let re_encrypted = crypto::encrypt(&updated_bytes, &self.encryption_key)
                .map_err(|e| ToolError::Failed(e.to_string()))?;

            let _ = sqlx::query(
                "UPDATE connected_accounts SET encrypted_credentials = $1, updated_at = NOW() WHERE id = $2 AND user_id = $3"
            )
            .bind(re_encrypted)
            .bind(account_id)
            .bind(user_id)
            .execute(&self.pool)
            .await;
        }

        Ok(tokens.access_token)
    }
}

// ---------------------------------------------------------------------------
// GmailProvider Implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl GmailProvider for GoogleClient {
    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> Result<Vec<EmailSummary>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;
        let max_results = limit.clamp(1, 50);

        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages?q={}&maxResults={}",
            url_encode(query),
            max_results
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ToolError::Failed(format!(
                "Gmail API error {status}: {body}"
            )));
        }

        #[derive(Deserialize)]
        struct ListResponse {
            messages: Option<Vec<MessageRef>>,
        }
        #[derive(Deserialize)]
        struct MessageRef {
            id: String,
        }

        let list_data: ListResponse = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let Some(refs) = list_data.messages else {
            return Ok(Vec::new());
        };

        let mut summaries = Vec::new();
        for mref in refs {
            let meta_url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From&metadataHeaders=To&metadataHeaders=Date",
                mref.id
            );

            let meta_resp = match self.http.get(&meta_url).bearer_auth(&token).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => continue,
            };

            #[derive(Deserialize)]
            struct MessageMeta {
                id: String,
                #[serde(rename = "threadId")]
                thread_id: String,
                #[serde(rename = "labelIds")]
                label_ids: Option<Vec<String>>,
                snippet: Option<String>,
                #[serde(rename = "internalDate")]
                internal_date: Option<String>,
                payload: Option<PayloadMeta>,
            }
            #[derive(Deserialize)]
            struct PayloadMeta {
                headers: Option<Vec<Header>>,
            }
            #[derive(Deserialize)]
            struct Header {
                name: String,
                value: String,
            }

            if let Ok(meta) = meta_resp.json::<MessageMeta>().await {
                let mut sender = "Unknown".to_string();
                let mut recipient_list = Vec::new();
                let mut subject = "(No Subject)".to_string();

                if let Some(headers) = meta.payload.and_then(|p| p.headers) {
                    for h in headers {
                        match h.name.to_lowercase().as_str() {
                            "from" => sender = h.value,
                            "to" => {
                                recipient_list = h
                                    .value
                                    .split(',')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                            }
                            "subject" => subject = h.value,
                            _ => {}
                        }
                    }
                }

                let is_unread = meta
                    .label_ids
                    .as_ref()
                    .map(|labels| labels.iter().any(|l| l == "UNREAD"))
                    .unwrap_or(false);

                let date = meta
                    .internal_date
                    .and_then(|d| d.parse::<i64>().ok())
                    .and_then(|ms| OffsetDateTime::from_unix_timestamp(ms / 1000).ok());

                summaries.push(EmailSummary {
                    id: meta.id,
                    account_id,
                    thread_id: meta.thread_id,
                    from: sender,
                    to: recipient_list,
                    subject,
                    date,
                    snippet: meta.snippet.unwrap_or_default(),
                    is_unread,
                });
            }
        }

        Ok(summaries)
    }

    async fn read(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        message_id: &str,
    ) -> Result<EmailDetail, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}?format=full"
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(ToolError::Failed(format!("Gmail read error {status}")));
        }

        let full: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let id = full["id"].as_str().unwrap_or(message_id).to_string();
        let thread_id = full["threadId"].as_str().unwrap_or_default().to_string();

        let mut sender = "Unknown".to_string();
        let mut recipient_list = Vec::new();
        let mut subject = "(No Subject)".to_string();

        if let Some(headers) = full["payload"]["headers"].as_array() {
            for h in headers {
                let name = h["name"].as_str().unwrap_or_default().to_lowercase();
                let value = h["value"].as_str().unwrap_or_default().to_string();
                match name.as_str() {
                    "from" => sender = value,
                    "to" => {
                        recipient_list = value
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    "subject" => subject = value,
                    _ => {}
                }
            }
        }

        let body_text = extract_body_from_json(&full["payload"]);

        let is_unread = full["labelIds"]
            .as_array()
            .map(|arr| arr.iter().any(|l| l.as_str() == Some("UNREAD")))
            .unwrap_or(false);

        let date = full["internalDate"]
            .as_str()
            .and_then(|d| d.parse::<i64>().ok())
            .and_then(|ms| OffsetDateTime::from_unix_timestamp(ms / 1000).ok());

        Ok(EmailDetail {
            id,
            account_id,
            thread_id,
            from: sender,
            to: recipient_list,
            subject,
            date,
            body_text,
            is_unread,
        })
    }
}

fn extract_body_from_json(payload: &serde_json::Value) -> String {
    if let Some(data) = payload["body"]["data"].as_str() {
        let decoded = decode_base64(data);
        if let Ok(text) = String::from_utf8(decoded) {
            return text;
        }
    }

    if let Some(parts) = payload["parts"].as_array() {
        // Look for text/plain first
        for p in parts {
            if p["mimeType"].as_str() == Some("text/plain") {
                let res = extract_body_from_json(p);
                if !res.is_empty() {
                    return res;
                }
            }
        }
        // Fallback to text/html or any part
        for p in parts {
            let res = extract_body_from_json(p);
            if !res.is_empty() {
                return res;
            }
        }
    }

    String::new()
}

// ---------------------------------------------------------------------------
// CalendarProvider Implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl CalendarProvider for GoogleClient {
    async fn list(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<Vec<CalendarEvent>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let time_min_str = start
            .format(&Rfc3339)
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        let time_max_str = end
            .format(&Rfc3339)
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events?\
            timeMin={}&\
            timeMax={}&\
            singleEvents=true&\
            orderBy=startTime",
            url_encode(&time_min_str),
            url_encode(&time_max_str),
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ToolError::Failed(format!(
                "Calendar list error {status}: {body}"
            )));
        }

        #[derive(Deserialize)]
        struct EventsResp {
            items: Option<Vec<GoogleEventItem>>,
        }

        let events_resp: EventsResp = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let items = events_resp.items.unwrap_or_default();
        let events = items
            .into_iter()
            .map(|item| map_google_event(account_id, item))
            .collect();

        Ok(events)
    }

    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        query: &str,
    ) -> Result<Vec<CalendarEvent>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events?\
            q={}&\
            maxResults=50&\
            singleEvents=true&\
            orderBy=startTime",
            url_encode(query)
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(ToolError::Failed(format!("Calendar search error {status}")));
        }

        #[derive(Deserialize)]
        struct EventsResp {
            items: Option<Vec<GoogleEventItem>>,
        }

        let events_resp: EventsResp = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let items = events_resp.items.unwrap_or_default();
        let events = items
            .into_iter()
            .map(|item| map_google_event(account_id, item))
            .collect();

        Ok(events)
    }

    async fn create(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        event: CreateCalendarEvent,
    ) -> Result<CalendarEvent, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let start_str = event
            .start_time
            .format(&Rfc3339)
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        let end_str = event
            .end_time
            .format(&Rfc3339)
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let body = serde_json::json!({
            "summary": event.title,
            "description": event.description,
            "location": event.location,
            "start": { "dateTime": start_str },
            "end": { "dateTime": end_str },
        });

        let resp = self
            .http
            .post("https://www.googleapis.com/calendar/v3/calendars/primary/events")
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(ToolError::Failed(format!(
                "Calendar create error {status}: {err_text}"
            )));
        }

        let item: GoogleEventItem = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        Ok(map_google_event(account_id, item))
    }

    async fn update(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        event_id: &str,
        event: UpdateCalendarEvent,
    ) -> Result<CalendarEvent, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let mut body = serde_json::Map::new();
        if let Some(ref title) = event.title {
            body.insert("summary".into(), serde_json::Value::String(title.clone()));
        }
        if let Some(ref desc) = event.description {
            body.insert(
                "description".into(),
                serde_json::Value::String(desc.clone()),
            );
        }
        if let Some(ref loc) = event.location {
            body.insert("location".into(), serde_json::Value::String(loc.clone()));
        }
        if let Some(ref st) = event.start_time {
            let s = st
                .format(&Rfc3339)
                .map_err(|e| ToolError::Failed(e.to_string()))?;
            body.insert("start".into(), serde_json::json!({ "dateTime": s }));
        }
        if let Some(ref et) = event.end_time {
            let s = et
                .format(&Rfc3339)
                .map_err(|e| ToolError::Failed(e.to_string()))?;
            body.insert("end".into(), serde_json::json!({ "dateTime": s }));
        }

        let url =
            format!("https://www.googleapis.com/calendar/v3/calendars/primary/events/{event_id}");

        let resp = self
            .http
            .patch(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(ToolError::Failed(format!(
                "Calendar update error {status}: {err_text}"
            )));
        }

        let item: GoogleEventItem = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        Ok(map_google_event(account_id, item))
    }

    async fn delete(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        event_id: &str,
    ) -> Result<(), ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let url =
            format!("https://www.googleapis.com/calendar/v3/calendars/primary/events/{event_id}");

        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if resp.status() == HttpStatusCode::NO_CONTENT || resp.status() == HttpStatusCode::OK {
            Ok(())
        } else if resp.status() == HttpStatusCode::NOT_FOUND {
            Err(ToolError::NotFound(format!("Event {event_id} not found")))
        } else {
            let status = resp.status();
            Err(ToolError::Failed(format!("Calendar delete error {status}")))
        }
    }
}

#[derive(Deserialize)]
struct GoogleEventItem {
    id: String,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start: Option<TimePoint>,
    end: Option<TimePoint>,
}

#[derive(Deserialize)]
struct TimePoint {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    date: Option<String>,
}

fn map_google_event(account_id: Uuid, item: GoogleEventItem) -> CalendarEvent {
    let title = item.summary.unwrap_or_else(|| "(Untitled Event)".into());
    let now = OffsetDateTime::now_utc();

    let all_day = item
        .start
        .as_ref()
        .map(|s| s.date.is_some())
        .unwrap_or(false);

    let start_time = item
        .start
        .as_ref()
        .and_then(|s| {
            s.date_time
                .as_deref()
                .and_then(|dt| OffsetDateTime::parse(dt, &Rfc3339).ok())
                .or_else(|| {
                    s.date.as_deref().and_then(|d| {
                        OffsetDateTime::parse(&format!("{d}T00:00:00Z"), &Rfc3339).ok()
                    })
                })
        })
        .unwrap_or(now);

    let end_time = item
        .end
        .as_ref()
        .and_then(|s| {
            s.date_time
                .as_deref()
                .and_then(|dt| OffsetDateTime::parse(dt, &Rfc3339).ok())
                .or_else(|| {
                    s.date.as_deref().and_then(|d| {
                        OffsetDateTime::parse(&format!("{d}T23:59:59Z"), &Rfc3339).ok()
                    })
                })
        })
        .unwrap_or_else(|| start_time + time::Duration::hours(1));

    CalendarEvent {
        id: item.id,
        account_id,
        title,
        start_time,
        end_time,
        description: item.description,
        location: item.location,
        all_day,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        let byte = u8::from_str_radix(&s[i..i + 2], 16).ok()?;
        bytes.push(byte);
    }
    Some(bytes)
}

fn decode_base64(input: &str) -> Vec<u8> {
    let mut clean = Vec::new();
    for c in input.chars() {
        match c {
            'A'..='Z' => clean.push(c as u8 - b'A'),
            'a'..='z' => clean.push(c as u8 - b'a' + 26),
            '0'..='9' => clean.push(c as u8 - b'0' + 52),
            '+' | '-' => clean.push(62),
            '/' | '_' => clean.push(63),
            '=' | ' ' | '\n' | '\r' => continue,
            _ => continue,
        }
    }
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
        if chunk.len() >= 2 {
            out.push((chunk[0] << 2) | (chunk[1] >> 4));
        }
        if chunk.len() >= 3 {
            out.push(((chunk[1] & 0x0F) << 4) | (chunk[2] >> 2));
        }
        if chunk.len() >= 4 {
            out.push(((chunk[2] & 0x03) << 6) | chunk[3]);
        }
    }
    out
}
