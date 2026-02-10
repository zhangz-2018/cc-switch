use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose;
use base64::Engine as _;
use chrono::Utc;
use reqwest::Client;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::get_home_dir;
use crate::error::AppError;
use crate::provider::{Provider, UsageData, UsageResult};

pub const ANTIGRAVITY_ACCESS_TOKEN_KEY: &str = "ANTIGRAVITY_ACCESS_TOKEN";
pub const ANTIGRAVITY_REFRESH_TOKEN_KEY: &str = "ANTIGRAVITY_REFRESH_TOKEN";
pub const ANTIGRAVITY_EMAIL_KEY: &str = "ANTIGRAVITY_EMAIL";
pub const ANTIGRAVITY_EXPIRES_AT_KEY: &str = "ANTIGRAVITY_EXPIRES_AT";
pub const ANTIGRAVITY_PROJECT_ID_KEY: &str = "ANTIGRAVITY_PROJECT_ID";

const CLOUD_CODE_BASE_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";
const QUOTA_API_URL: &str =
    "https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:fetchAvailableModels";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const FALLBACK_PROJECT_ID: &str = "bamboo-precept-lgxtn";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityImportedSession {
    pub email: String,
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(rename = "refreshToken")]
    pub refresh_token: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
    #[serde(rename = "projectId", skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityModelQuota {
    pub name: String,
    #[serde(rename = "remainingPercent")]
    pub remaining_percent: i32,
    #[serde(rename = "usedPercent")]
    pub used_percent: i32,
    #[serde(rename = "resetTime", skip_serializing_if = "Option::is_none")]
    pub reset_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityQuotaResponse {
    #[serde(rename = "projectId")]
    pub project_id: String,
    #[serde(rename = "subscriptionTier", skip_serializing_if = "Option::is_none")]
    pub subscription_tier: Option<String>,
    pub models: Vec<AntigravityModelQuota>,
    #[serde(rename = "fetchedAt")]
    pub fetched_at: i64,
}

#[derive(Debug, Clone)]
struct TokenBundle {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoadProjectResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project_id: Option<String>,
    #[serde(rename = "paidTier")]
    paid_tier: Option<TierInfo>,
    #[serde(rename = "currentTier")]
    current_tier: Option<TierInfo>,
}

#[derive(Debug, Deserialize)]
struct TierInfo {
    id: Option<String>,
}

pub fn import_current_session_from_local_db() -> Result<AntigravityImportedSession, AppError> {
    let db_path = get_antigravity_db_path();
    if !db_path.exists() {
        return Err(AppError::localized(
            "antigravity.db.not_found",
            format!("未找到 Antigravity 数据库: {}", db_path.display()),
            format!("Antigravity database not found: {}", db_path.display()),
        ));
    }

    let conn = Connection::open(&db_path).map_err(|e| {
        AppError::localized(
            "antigravity.db.open_failed",
            format!("打开 Antigravity 数据库失败: {e}"),
            format!("Failed to open Antigravity database: {e}"),
        )
    })?;

    let mut token = extract_token_bundle_new_format(&conn)
        .or_else(|| extract_token_bundle_old_format(&conn))
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.token.not_found",
                "未在 Antigravity 数据库中找到可用的 OAuth Token",
                "OAuth token not found in Antigravity database",
            )
        })?;

    if token.email.as_deref().unwrap_or("").trim().is_empty() {
        token.email = fetch_user_email_sync(&token.access_token).ok();
    }

    let email = token.email.clone().ok_or_else(|| {
        AppError::localized(
            "antigravity.email.not_found",
            "未能解析账号邮箱，请先在 Antigravity 客户端完成登录",
            "Failed to resolve account email, please login in Antigravity client first",
        )
    })?;

    let project_id = fetch_project_id_and_tier_sync(&token.access_token, Some(&email)).0;

    Ok(AntigravityImportedSession {
        email,
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: token.expires_at,
        project_id,
    })
}

pub fn apply_account_from_provider(provider: &Provider) -> Result<(), AppError> {
    let env_map = extract_env_map_from_provider(provider)?;

    let access_token = env_map
        .get(ANTIGRAVITY_ACCESS_TOKEN_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.provider.missing_access_token",
                "缺少 ANTIGRAVITY_ACCESS_TOKEN，无法切换 Antigravity 官方账号",
                "Missing ANTIGRAVITY_ACCESS_TOKEN, cannot switch Antigravity official account",
            )
        })?;

    let refresh_token = env_map
        .get(ANTIGRAVITY_REFRESH_TOKEN_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.provider.missing_refresh_token",
                "缺少 ANTIGRAVITY_REFRESH_TOKEN，无法切换 Antigravity 官方账号",
                "Missing ANTIGRAVITY_REFRESH_TOKEN, cannot switch Antigravity official account",
            )
        })?;

    let email = env_map
        .get(ANTIGRAVITY_EMAIL_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.provider.missing_email",
                "缺少 ANTIGRAVITY_EMAIL，无法切换 Antigravity 官方账号",
                "Missing ANTIGRAVITY_EMAIL, cannot switch Antigravity official account",
            )
        })?;

    let expires_at = env_map
        .get(ANTIGRAVITY_EXPIRES_AT_KEY)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or_else(|| Utc::now().timestamp() + 3600);

    let db_path = get_antigravity_db_path();
    if !db_path.exists() {
        return Err(AppError::localized(
            "antigravity.db.not_found",
            format!("未找到 Antigravity 数据库: {}", db_path.display()),
            format!("Antigravity database not found: {}", db_path.display()),
        ));
    }

    inject_token_to_antigravity_db(&db_path, &access_token, &refresh_token, expires_at, &email)?;

    restart_antigravity_best_effort();

    Ok(())
}

pub async fn query_usage_from_provider(provider: &Provider) -> Result<UsageResult, AppError> {
    let env_map = extract_env_map_from_provider(provider)?;

    let access_token = env_map
        .get(ANTIGRAVITY_ACCESS_TOKEN_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.provider.missing_access_token",
                "缺少 ANTIGRAVITY_ACCESS_TOKEN，无法查询 Antigravity 余量",
                "Missing ANTIGRAVITY_ACCESS_TOKEN, cannot query Antigravity quota",
            )
        })?;

    let email = if let Some(v) = env_map
        .get(ANTIGRAVITY_EMAIL_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty())
    {
        v
    } else {
        fetch_user_email_async(&access_token).await?
    };

    let cached_project_id = env_map
        .get(ANTIGRAVITY_PROJECT_ID_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty());

    let quota = fetch_quota(&access_token, &email, cached_project_id.as_deref()).await?;

    let usage_data: Vec<UsageData> = quota
        .models
        .iter()
        .map(|m| UsageData {
            plan_name: Some(m.name.clone()),
            extra: m
                .reset_time
                .as_ref()
                .map(|v| format!("重置时间: {v}"))
                .or_else(|| {
                    quota
                        .subscription_tier
                        .as_ref()
                        .map(|v| format!("订阅: {v}"))
                }),
            is_valid: Some(true),
            invalid_message: None,
            total: Some(100.0),
            used: Some(f64::from(m.used_percent)),
            remaining: Some(f64::from(m.remaining_percent)),
            unit: Some("%".to_string()),
        })
        .collect();

    if usage_data.is_empty() {
        return Ok(UsageResult {
            success: false,
            data: None,
            error: Some("未获取到可展示的模型余量数据".to_string()),
        });
    }

    Ok(UsageResult {
        success: true,
        data: Some(usage_data),
        error: None,
    })
}

pub async fn fetch_quota_from_provider(
    provider: &Provider,
) -> Result<AntigravityQuotaResponse, AppError> {
    let env_map = extract_env_map_from_provider(provider)?;

    let access_token = env_map
        .get(ANTIGRAVITY_ACCESS_TOKEN_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.provider.missing_access_token",
                "缺少 ANTIGRAVITY_ACCESS_TOKEN，无法查询 Antigravity 余量",
                "Missing ANTIGRAVITY_ACCESS_TOKEN, cannot query Antigravity quota",
            )
        })?;

    let email = if let Some(v) = env_map
        .get(ANTIGRAVITY_EMAIL_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty())
    {
        v
    } else {
        fetch_user_email_async(&access_token).await?
    };

    let cached_project_id = env_map
        .get(ANTIGRAVITY_PROJECT_ID_KEY)
        .cloned()
        .filter(|v| !v.trim().is_empty());

    fetch_quota(&access_token, &email, cached_project_id.as_deref()).await
}

async fn fetch_quota(
    access_token: &str,
    email: &str,
    cached_project_id: Option<&str>,
) -> Result<AntigravityQuotaResponse, AppError> {
    let (project_id, tier) = if let Some(pid) = cached_project_id {
        (pid.to_string(), None)
    } else {
        let (project_id, tier) = fetch_project_id_and_tier(access_token, Some(email)).await;
        (
            project_id.unwrap_or_else(|| FALLBACK_PROJECT_ID.to_string()),
            tier,
        )
    };

    let client = Client::new();
    let resp = client
        .post(QUOTA_API_URL)
        .bearer_auth(access_token)
        .header("User-Agent", "cc-switch/antigravity")
        .header("Content-Type", "application/json")
        .json(&json!({ "project": project_id }))
        .send()
        .await
        .map_err(|e| {
            AppError::localized(
                "antigravity.quota.request_failed",
                format!("请求 Antigravity 余量接口失败: {e}"),
                format!("Failed to request Antigravity quota API: {e}"),
            )
        })?;

    let status = resp.status();
    let payload: Value = resp.json().await.map_err(|e| {
        AppError::localized(
            "antigravity.quota.parse_failed",
            format!("解析 Antigravity 余量响应失败: {e}"),
            format!("Failed to parse Antigravity quota response: {e}"),
        )
    })?;

    if !status.is_success() {
        let reason = payload
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str)
            .or_else(|| payload.get("message").and_then(Value::as_str))
            .unwrap_or("未知错误");

        return Err(AppError::localized(
            "antigravity.quota.request_failed",
            format!("Antigravity 余量查询失败 ({}): {reason}", status.as_u16()),
            format!(
                "Antigravity quota request failed ({}): {reason}",
                status.as_u16()
            ),
        ));
    }

    let mut models: Vec<AntigravityModelQuota> = Vec::new();

    if let Some(obj) = payload.get("models").and_then(Value::as_object) {
        for (name, info) in obj {
            let quota = info.get("quotaInfo").or_else(|| info.get("quota_info"));
            let Some(quota) = quota else {
                continue;
            };

            let remaining_fraction = quota
                .get("remainingFraction")
                .or_else(|| quota.get("remaining_fraction"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let remaining_percent = (remaining_fraction * 100.0).round().clamp(0.0, 100.0) as i32;
            let used_percent = (100 - remaining_percent).clamp(0, 100);
            let reset_time = quota
                .get("resetTime")
                .or_else(|| quota.get("reset_time"))
                .and_then(Value::as_str)
                .map(str::to_string);

            if name.contains("gemini") || name.contains("claude") {
                models.push(AntigravityModelQuota {
                    name: name.to_string(),
                    remaining_percent,
                    used_percent,
                    reset_time,
                });
            }
        }
    }

    models.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(AntigravityQuotaResponse {
        project_id,
        subscription_tier: tier,
        models,
        fetched_at: Utc::now().timestamp(),
    })
}

async fn fetch_project_id_and_tier(
    access_token: &str,
    email: Option<&str>,
) -> (Option<String>, Option<String>) {
    let client = Client::new();

    let resp = client
        .post(format!("{CLOUD_CODE_BASE_URL}/v1internal:loadCodeAssist"))
        .bearer_auth(access_token)
        .header("User-Agent", "cc-switch/antigravity")
        .header("Content-Type", "application/json")
        .json(&json!({ "metadata": { "ideType": "ANTIGRAVITY" } }))
        .send()
        .await;

    let Ok(resp) = resp else {
        return (None, None);
    };

    if !resp.status().is_success() {
        if let Some(email) = email {
            log::warn!("loadCodeAssist failed for {email}: {}", resp.status());
        }
        return (None, None);
    }

    let Ok(body) = resp.json::<LoadProjectResponse>().await else {
        return (None, None);
    };

    let tier = body
        .paid_tier
        .and_then(|v| v.id)
        .or_else(|| body.current_tier.and_then(|v| v.id));

    (body.project_id, tier)
}

fn fetch_project_id_and_tier_sync(
    access_token: &str,
    email: Option<&str>,
) -> (Option<String>, Option<String>) {
    let runtime = tokio::runtime::Runtime::new();
    let Ok(runtime) = runtime else {
        return (None, None);
    };

    runtime.block_on(fetch_project_id_and_tier(access_token, email))
}

async fn fetch_user_email_async(access_token: &str) -> Result<String, AppError> {
    let resp = Client::new()
        .get(USERINFO_URL)
        .bearer_auth(access_token)
        .header("User-Agent", "cc-switch/antigravity")
        .send()
        .await
        .map_err(|e| {
            AppError::localized(
                "antigravity.userinfo.request_failed",
                format!("查询用户信息失败: {e}"),
                format!("Failed to fetch user info: {e}"),
            )
        })?;

    if !resp.status().is_success() {
        return Err(AppError::localized(
            "antigravity.userinfo.request_failed",
            format!("查询用户信息失败: HTTP {}", resp.status().as_u16()),
            format!("Failed to fetch user info: HTTP {}", resp.status().as_u16()),
        ));
    }

    let payload: Value = resp.json().await.map_err(|e| {
        AppError::localized(
            "antigravity.userinfo.parse_failed",
            format!("解析用户信息失败: {e}"),
            format!("Failed to parse user info: {e}"),
        )
    })?;

    payload
        .get("email")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.userinfo.email_missing",
                "用户信息中缺少 email 字段",
                "Email field missing in user info",
            )
        })
}

fn fetch_user_email_sync(access_token: &str) -> Result<String, AppError> {
    let runtime = tokio::runtime::Runtime::new().map_err(|e| {
        AppError::localized(
            "antigravity.runtime.init_failed",
            format!("初始化异步运行时失败: {e}"),
            format!("Failed to initialize async runtime: {e}"),
        )
    })?;

    runtime.block_on(fetch_user_email_async(access_token))
}

fn extract_env_map_from_provider(provider: &Provider) -> Result<HashMap<String, String>, AppError> {
    let env_obj = provider
        .settings_config
        .get("env")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AppError::localized(
                "antigravity.provider.invalid_env",
                "Gemini 配置格式错误：缺少 env 配置",
                "Invalid Gemini settings: missing env object",
            )
        })?;

    let mut env_map = HashMap::new();
    for (k, v) in env_obj {
        if let Some(s) = v.as_str() {
            env_map.insert(k.clone(), s.to_string());
        }
    }
    Ok(env_map)
}

fn get_antigravity_db_path() -> PathBuf {
    if let Ok(raw_path) = std::env::var("ANTIGRAVITY_STATE_DB_PATH") {
        let trimmed = raw_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if cfg!(target_os = "macos") {
        return get_home_dir()
            .join("Library/Application Support/Antigravity/User/globalStorage/state.vscdb");
    }

    if cfg!(target_os = "windows") {
        if let Ok(app_data) = std::env::var("APPDATA") {
            return PathBuf::from(app_data).join("Antigravity\\User\\globalStorage\\state.vscdb");
        }
    }

    if cfg!(target_os = "linux") {
        return get_home_dir().join(".config/Antigravity/User/globalStorage/state.vscdb");
    }

    get_home_dir().join(".config/Antigravity/User/globalStorage/state.vscdb")
}

pub fn is_antigravity_provider(provider: &Provider) -> bool {
    if provider
        .meta
        .as_ref()
        .and_then(|m| m.partner_promotion_key.as_deref())
        .map(|k| k.eq_ignore_ascii_case("antigravity"))
        .unwrap_or(false)
    {
        return true;
    }

    let name_lower = provider.name.to_ascii_lowercase();
    if name_lower.contains("antigravity") {
        return true;
    }

    if let Some(url) = provider.website_url.as_deref() {
        if url.to_ascii_lowercase().contains("antigravity") {
            return true;
        }
    }

    provider
        .settings_config
        .pointer("/env/GOOGLE_GEMINI_BASE_URL")
        .and_then(Value::as_str)
        .map(|v| v.to_ascii_lowercase().contains("antigravity"))
        .unwrap_or(false)
}

pub fn has_official_credentials(provider: &Provider) -> bool {
    extract_env_map_from_provider(provider)
        .map(|env| {
            env.get(ANTIGRAVITY_ACCESS_TOKEN_KEY)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
                && env
                    .get(ANTIGRAVITY_REFRESH_TOKEN_KEY)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false)
                && env
                    .get(ANTIGRAVITY_EMAIL_KEY)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn extract_token_bundle_new_format(conn: &Connection) -> Option<TokenBundle> {
    let value: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            ["antigravityUnifiedStateSync.oauthToken"],
            |row| row.get(0),
        )
        .ok()?;

    let outer = general_purpose::STANDARD.decode(value).ok()?;
    let inner = find_length_delimited_field(&outer, 1)?;
    let inner2 = find_length_delimited_field(&inner, 2)?;
    let oauth_info_b64 = find_length_delimited_field(&inner2, 1)?;
    let oauth_info_b64 = String::from_utf8(oauth_info_b64).ok()?;
    let oauth_info = general_purpose::STANDARD.decode(oauth_info_b64).ok()?;

    parse_oauth_info_message(&oauth_info, None)
}

fn extract_token_bundle_old_format(conn: &Connection) -> Option<TokenBundle> {
    let value: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            ["jetskiStateSync.agentManagerInitState"],
            |row| row.get(0),
        )
        .ok()?;

    let blob = general_purpose::STANDARD.decode(value).ok()?;
    let email = find_length_delimited_field(&blob, 2).and_then(|v| String::from_utf8(v).ok());
    let oauth_field = find_length_delimited_field(&blob, 6)?;

    parse_oauth_info_message(&oauth_field, email)
}

fn parse_oauth_info_message(data: &[u8], email: Option<String>) -> Option<TokenBundle> {
    let access_token = find_length_delimited_field(data, 1)
        .and_then(|v| String::from_utf8(v).ok())
        .filter(|v| !v.trim().is_empty())?;

    let refresh_token = find_length_delimited_field(data, 3)
        .and_then(|v| String::from_utf8(v).ok())
        .filter(|v| !v.trim().is_empty())?;

    let expires_at = find_length_delimited_field(data, 4)
        .and_then(|msg| find_varint_field(&msg, 1))
        .map(|v| v as i64)
        .unwrap_or_else(|| Utc::now().timestamp() + 3600);

    Some(TokenBundle {
        access_token,
        refresh_token,
        expires_at,
        email,
    })
}

fn inject_token_to_antigravity_db(
    db_path: &Path,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
    email: &str,
) -> Result<(), AppError> {
    let conn = Connection::open(db_path).map_err(|e| {
        AppError::localized(
            "antigravity.db.open_failed",
            format!("打开 Antigravity 数据库失败: {e}"),
            format!("Failed to open Antigravity database: {e}"),
        )
    })?;

    inject_new_format(&conn, access_token, refresh_token, expires_at)?;
    let _ = inject_old_format_if_exists(&conn, access_token, refresh_token, expires_at, email);

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        ["antigravityOnboarding", "true"],
    )
    .map_err(|e| {
        AppError::localized(
            "antigravity.db.write_failed",
            format!("写入 Antigravity onboarding 标记失败: {e}"),
            format!("Failed to write Antigravity onboarding marker: {e}"),
        )
    })?;

    Ok(())
}

fn inject_new_format(
    conn: &Connection,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
) -> Result<(), AppError> {
    let oauth_info = create_oauth_info(access_token, refresh_token, expires_at);
    let oauth_info_b64 = general_purpose::STANDARD.encode(oauth_info);

    let inner2 = encode_string_field(1, &oauth_info_b64);
    let inner = [
        encode_string_field(1, "oauthTokenInfoSentinelKey"),
        encode_len_delimited_field(2, &inner2),
    ]
    .concat();
    let outer = encode_len_delimited_field(1, &inner);
    let outer_b64 = general_purpose::STANDARD.encode(outer);

    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        ["antigravityUnifiedStateSync.oauthToken", &outer_b64],
    )
    .map_err(|e| {
        AppError::localized(
            "antigravity.db.write_failed",
            format!("写入 antigravityUnifiedStateSync.oauthToken 失败: {e}"),
            format!("Failed to write antigravityUnifiedStateSync.oauthToken: {e}"),
        )
    })?;

    Ok(())
}

fn inject_old_format_if_exists(
    conn: &Connection,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
    email: &str,
) -> Result<(), AppError> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            ["jetskiStateSync.agentManagerInitState"],
            |row| row.get(0),
        )
        .ok();

    let Some(existing) = existing else {
        return Ok(());
    };

    let blob = general_purpose::STANDARD.decode(existing).map_err(|e| {
        AppError::localized(
            "antigravity.db.parse_failed",
            format!("解析旧格式 Token 数据失败: {e}"),
            format!("Failed to decode old token blob: {e}"),
        )
    })?;

    let cleaned = remove_field(&remove_field(&remove_field(&blob, 1)?, 2)?, 6)?;
    let new_payload = [
        cleaned,
        create_email_field(email),
        create_oauth_field(access_token, refresh_token, expires_at),
    ]
    .concat();

    let encoded = general_purpose::STANDARD.encode(new_payload);

    conn.execute(
        "UPDATE ItemTable SET value = ?1 WHERE key = ?2",
        [&encoded, "jetskiStateSync.agentManagerInitState"],
    )
    .map_err(|e| {
        AppError::localized(
            "antigravity.db.write_failed",
            format!("写入 jetskiStateSync.agentManagerInitState 失败: {e}"),
            format!("Failed to write jetskiStateSync.agentManagerInitState: {e}"),
        )
    })?;

    Ok(())
}

fn restart_antigravity_best_effort() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-f", "Antigravity"])
            .output();
        let _ = std::process::Command::new("open")
            .arg("antigravity://")
            .output();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "taskkill /IM antigravity.exe /F"])
            .output();
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start antigravity://"])
            .output();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-f", "antigravity"])
            .output();
        let _ = std::process::Command::new("xdg-open")
            .arg("antigravity://")
            .output();
    }
}

fn read_varint(data: &[u8], mut offset: usize) -> Result<(u64, usize), AppError> {
    let mut result = 0_u64;
    let mut shift = 0_u32;

    while offset < data.len() {
        let byte = data[offset];
        result |= u64::from(byte & 0x7f) << shift;
        offset += 1;

        if byte & 0x80 == 0 {
            return Ok((result, offset));
        }

        shift += 7;
        if shift > 63 {
            break;
        }
    }

    Err(AppError::localized(
        "antigravity.protobuf.varint_invalid",
        "解析 Protobuf Varint 失败",
        "Failed to parse protobuf varint",
    ))
}

fn skip_field(data: &[u8], offset: usize, wire_type: u8) -> Result<usize, AppError> {
    match wire_type {
        0 => {
            let (_, next) = read_varint(data, offset)?;
            Ok(next)
        }
        1 => Ok(offset.saturating_add(8)),
        2 => {
            let (len, start) = read_varint(data, offset)?;
            Ok(start.saturating_add(len as usize))
        }
        5 => Ok(offset.saturating_add(4)),
        _ => Err(AppError::localized(
            "antigravity.protobuf.wire_type_invalid",
            format!("不支持的 Protobuf wire type: {wire_type}"),
            format!("Unsupported protobuf wire type: {wire_type}"),
        )),
    }
}

fn find_length_delimited_field(data: &[u8], target_field: u32) -> Option<Vec<u8>> {
    let mut offset = 0_usize;

    while offset < data.len() {
        let (tag, next) = read_varint(data, offset).ok()?;
        let wire_type = (tag & 0x7) as u8;
        let field_num = (tag >> 3) as u32;

        if wire_type == 2 && field_num == target_field {
            let (len, start) = read_varint(data, next).ok()?;
            let end = start.checked_add(len as usize)?;
            if end <= data.len() {
                return Some(data[start..end].to_vec());
            }
            return None;
        }

        offset = skip_field(data, next, wire_type).ok()?;
    }

    None
}

fn find_varint_field(data: &[u8], target_field: u32) -> Option<u64> {
    let mut offset = 0_usize;

    while offset < data.len() {
        let (tag, next) = read_varint(data, offset).ok()?;
        let wire_type = (tag & 0x7) as u8;
        let field_num = (tag >> 3) as u32;

        if wire_type == 0 && field_num == target_field {
            return read_varint(data, next).ok().map(|v| v.0);
        }

        offset = skip_field(data, next, wire_type).ok()?;
    }

    None
}

fn remove_field(data: &[u8], target_field: u32) -> Result<Vec<u8>, AppError> {
    let mut result = Vec::new();
    let mut offset = 0_usize;

    while offset < data.len() {
        let start = offset;
        let (tag, next) = read_varint(data, offset)?;
        let wire_type = (tag & 0x7) as u8;
        let field_num = (tag >> 3) as u32;
        let end = skip_field(data, next, wire_type)?;

        if field_num != target_field {
            if end <= data.len() {
                result.extend_from_slice(&data[start..end]);
            }
        }

        offset = end;
    }

    Ok(result)
}

fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 0x80 {
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

fn encode_len_delimited_field(field_num: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = encode_varint(u64::from((field_num << 3) | 2));
    out.extend(encode_varint(bytes.len() as u64));
    out.extend(bytes);
    out
}

fn encode_string_field(field_num: u32, value: &str) -> Vec<u8> {
    encode_len_delimited_field(field_num, value.as_bytes())
}

fn create_email_field(email: &str) -> Vec<u8> {
    encode_string_field(2, email)
}

fn create_oauth_info(access_token: &str, refresh_token: &str, expires_at: i64) -> Vec<u8> {
    let field1 = encode_string_field(1, access_token);
    let field2 = encode_string_field(2, "Bearer");
    let field3 = encode_string_field(3, refresh_token);

    let mut timestamp = encode_varint(1 << 3);
    timestamp.extend(encode_varint(expires_at as u64));
    let field4 = encode_len_delimited_field(4, &timestamp);

    [field1, field2, field3, field4].concat()
}

fn create_oauth_field(access_token: &str, refresh_token: &str, expires_at: i64) -> Vec<u8> {
    let oauth_info = create_oauth_info(access_token, refresh_token, expires_at);
    encode_len_delimited_field(6, &oauth_info)
}
