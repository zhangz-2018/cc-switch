use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;
use uuid::Uuid;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::Provider;
use crate::store::AppState;

const CODEX_DEVICE_AUTH_URL: &str = "https://auth.openai.com/codex/device";
const CODEX_OAUTH_DEFAULT_EXPIRES_IN: i64 = 900;
const CODEX_OAUTH_DEFAULT_INTERVAL: i64 = 5;
const CODEX_OAUTH_SESSION_TTL_SECONDS: i64 = 20 * 60;
const CODEX_OAUTH_PARSE_MAX_WAIT_MS: u64 = 4000;
const CODEX_OAUTH_PARSE_STEP_MS: u64 = 120;

static ANSI_ESCAPE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\x1b\[[0-9;]*m").expect("valid ansi regex"));
static DEVICE_CODE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b([A-Z0-9]{4}-[A-Z0-9]{4})\b").expect("valid device code regex"));
static VERIFICATION_URL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https://auth\.openai\.com/\S+").expect("valid verification url regex")
});
static CODEX_OAUTH_SESSIONS: Lazy<Mutex<HashMap<String, CodexCliOauthSession>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOauthDeviceFlowResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    pub interval: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexOauthPollResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexQuotaWindow {
    pub used_percent: i64,
    pub limit_window_seconds: i64,
    pub reset_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexQuotaUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<CodexQuotaWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly: Option<CodexQuotaWindow>,
    pub fetched_at: i64,
}

#[derive(Debug, Clone)]
enum CodexCliProcessStatus {
    Running,
    Exited(i32),
}

#[derive(Debug, Clone)]
struct CodexCliOauthSession {
    started_at: i64,
    expires_at: i64,
    log_path: PathBuf,
    verification_uri: Option<String>,
    user_code: Option<String>,
    auth_mtime_before: Option<i64>,
    process_status: CodexCliProcessStatus,
}

#[derive(Debug)]
struct ParsedCodexAuth {
    auth_json: Value,
    access_token: String,
    id_token: Option<String>,
    file_mtime: Option<i64>,
}

#[tauri::command]
pub async fn codex_oauth_init_device_flow() -> Result<CodexOauthDeviceFlowResponse, String> {
    start_codex_cli_oauth_session()
}

#[tauri::command]
pub async fn codex_oauth_poll_token(device_code: String) -> Result<CodexOauthPollResponse, String> {
    match poll_codex_cli_oauth_session(&device_code).await {
        Ok(resp) => Ok(resp),
        Err(err) => Ok(CodexOauthPollResponse {
            status: "error".to_string(),
            auth_json: None,
            email: None,
            error: Some("oauth_poll_failed".to_string()),
            error_description: Some(err),
        }),
    }
}

fn start_codex_cli_oauth_session() -> Result<CodexOauthDeviceFlowResponse, String> {
    ensure_codex_cli_available()?;
    cleanup_expired_oauth_sessions();

    let session_id = Uuid::new_v4().to_string();
    let started_at = Utc::now().timestamp();
    let expires_at = started_at + CODEX_OAUTH_SESSION_TTL_SECONDS;
    let log_path = get_codex_oauth_log_dir()?.join(format!("codex-login-{session_id}.log"));

    let stdout_file =
        File::create(&log_path).map_err(|e| format!("创建 OAuth 日志文件失败: {e}"))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("准备 OAuth 日志输出失败: {e}"))?;

    let mut child = Command::new("codex")
        .arg("login")
        .arg("--device-auth")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("TERM", "dumb")
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|e| format!("启动 Codex 登录进程失败: {e}"))?;

    let auth_mtime_before = file_mtime_unix(&crate::codex_config::get_codex_auth_path());

    {
        let mut sessions = CODEX_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        sessions.insert(
            session_id.clone(),
            CodexCliOauthSession {
                started_at,
                expires_at,
                log_path: log_path.clone(),
                verification_uri: None,
                user_code: None,
                auth_mtime_before,
                process_status: CodexCliProcessStatus::Running,
            },
        );
    }

    let session_id_for_wait = session_id.clone();
    std::thread::spawn(move || {
        let exit_code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        if let Ok(mut sessions) = CODEX_OAUTH_SESSIONS.lock() {
            if let Some(session) = sessions.get_mut(&session_id_for_wait) {
                session.process_status = CodexCliProcessStatus::Exited(exit_code);
            }
        }
    });

    let deadline = std::time::Instant::now() + Duration::from_millis(CODEX_OAUTH_PARSE_MAX_WAIT_MS);
    let mut verification_uri: Option<String> = None;
    let mut user_code: Option<String> = None;

    while std::time::Instant::now() < deadline {
        if let Ok(log_text) = fs::read_to_string(&log_path) {
            let (parsed_uri, parsed_code) = parse_device_flow_from_log(&log_text);
            if verification_uri.is_none() {
                verification_uri = parsed_uri;
            }
            if user_code.is_none() {
                user_code = parsed_code;
            }
            if verification_uri.is_some() && user_code.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(CODEX_OAUTH_PARSE_STEP_MS));
    }

    {
        let mut sessions = CODEX_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        if let Some(session) = sessions.get_mut(&session_id) {
            if session.verification_uri.is_none() {
                session.verification_uri = verification_uri.clone();
            }
            if session.user_code.is_none() {
                session.user_code = user_code.clone();
            }
        }
    }

    let process_exit_code = {
        let sessions = CODEX_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        sessions
            .get(&session_id)
            .and_then(|s| match s.process_status {
                CodexCliProcessStatus::Exited(code) => Some(code),
                CodexCliProcessStatus::Running => None,
            })
    };

    if verification_uri.is_none() && user_code.is_none() {
        if let Some(code) = process_exit_code {
            let details = read_log_tail(&log_path, 16).unwrap_or_else(|| "未知错误".to_string());
            return Err(format!("启动 Codex OAuth 失败 (exit {code}): {details}"));
        }
    }

    Ok(CodexOauthDeviceFlowResponse {
        device_code: session_id,
        user_code: user_code.unwrap_or_default(),
        verification_uri: verification_uri
            .clone()
            .unwrap_or_else(|| CODEX_DEVICE_AUTH_URL.to_string()),
        verification_uri_complete: verification_uri,
        expires_in: CODEX_OAUTH_DEFAULT_EXPIRES_IN,
        interval: CODEX_OAUTH_DEFAULT_INTERVAL,
    })
}

async fn poll_codex_cli_oauth_session(session_id: &str) -> Result<CodexOauthPollResponse, String> {
    cleanup_expired_oauth_sessions();

    let mut session = {
        let mut sessions = CODEX_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        let Some(session) = sessions.get_mut(session_id) else {
            return Ok(CodexOauthPollResponse {
                status: "error".to_string(),
                auth_json: None,
                email: None,
                error: Some("oauth_session_not_found".to_string()),
                error_description: Some("OAuth 会话不存在或已过期，请重新登录".to_string()),
            });
        };
        refresh_session_device_flow_from_log(session);
        session.clone()
    };

    if Utc::now().timestamp() > session.expires_at {
        remove_oauth_session(session_id);
        return Ok(CodexOauthPollResponse {
            status: "error".to_string(),
            auth_json: None,
            email: None,
            error: Some("oauth_session_expired".to_string()),
            error_description: Some("OAuth 登录已超时，请重试".to_string()),
        });
    }

    if let Some(parsed_auth) = try_parse_codex_auth_file()? {
        let auth_file_updated = match (parsed_auth.file_mtime, session.auth_mtime_before) {
            (Some(after), Some(before)) => after > before,
            (Some(_), None) => true,
            _ => false,
        };
        let can_accept_existing_token =
            matches!(session.process_status, CodexCliProcessStatus::Exited(0));

        if auth_file_updated || can_accept_existing_token {
            let mut auth_json = parsed_auth.auth_json;
            if let Some(auth_obj) = auth_json.as_object_mut() {
                auth_obj.insert("auth_mode".to_string(), json!("chatgpt"));
                if auth_obj.get("last_refresh").is_none() {
                    auth_obj.insert("last_refresh".to_string(), json!(Utc::now().to_rfc3339()));
                }
            }

            let email_from_claims = parsed_auth
                .id_token
                .as_deref()
                .and_then(decode_jwt_payload)
                .as_ref()
                .and_then(|claims| extract_email_from_claims(Some(claims)));

            let email = if email_from_claims.is_some() {
                email_from_claims
            } else {
                fetch_user_email(&Client::new(), &parsed_auth.access_token)
                    .await
                    .ok()
                    .flatten()
            };

            remove_oauth_session(session_id);
            return Ok(CodexOauthPollResponse {
                status: "success".to_string(),
                auth_json: Some(auth_json),
                email,
                error: None,
                error_description: None,
            });
        }
    }

    session = {
        let sessions = CODEX_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        let Some(current) = sessions.get(session_id) else {
            return Ok(CodexOauthPollResponse {
                status: "error".to_string(),
                auth_json: None,
                email: None,
                error: Some("oauth_session_not_found".to_string()),
                error_description: Some("OAuth 会话不存在或已过期，请重新登录".to_string()),
            });
        };
        current.clone()
    };

    match session.process_status {
        CodexCliProcessStatus::Running | CodexCliProcessStatus::Exited(0) => {
            Ok(CodexOauthPollResponse {
                status: "pending".to_string(),
                auth_json: None,
                email: None,
                error: Some("authorization_pending".to_string()),
                error_description: Some("等待浏览器完成授权".to_string()),
            })
        }
        CodexCliProcessStatus::Exited(code) => {
            remove_oauth_session(session_id);
            let details = read_log_tail(&session.log_path, 20)
                .unwrap_or_else(|| "Codex 登录进程异常退出".to_string());
            Ok(CodexOauthPollResponse {
                status: "error".to_string(),
                auth_json: None,
                email: None,
                error: Some("oauth_cli_failed".to_string()),
                error_description: Some(format!("Codex 登录失败 (exit {code}): {details}")),
            })
        }
    }
}

fn ensure_codex_cli_available() -> Result<(), String> {
    let status = Command::new("codex")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("未检测到 codex 命令，请先安装 Codex CLI: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("检测到 codex 命令但无法执行，请检查安装或 PATH 配置".to_string())
    }
}

fn get_codex_oauth_log_dir() -> Result<PathBuf, String> {
    let log_dir = crate::config::get_app_config_dir().join("oauth-logs");
    fs::create_dir_all(&log_dir).map_err(|e| format!("创建 OAuth 日志目录失败: {e}"))?;
    Ok(log_dir)
}

fn cleanup_expired_oauth_sessions() {
    let now = Utc::now().timestamp();
    if let Ok(mut sessions) = CODEX_OAUTH_SESSIONS.lock() {
        let expired_ids: Vec<String> = sessions
            .iter()
            .filter_map(|(id, session)| {
                let expired_by_time = now > session.expires_at;
                let exited_too_long =
                    matches!(session.process_status, CodexCliProcessStatus::Exited(_))
                        && now - session.started_at > 5 * 60;
                if expired_by_time || exited_too_long {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        for id in expired_ids {
            if let Some(session) = sessions.remove(&id) {
                let _ = fs::remove_file(&session.log_path);
            }
        }
    }
}

fn remove_oauth_session(session_id: &str) {
    if let Ok(mut sessions) = CODEX_OAUTH_SESSIONS.lock() {
        if let Some(session) = sessions.remove(session_id) {
            let _ = fs::remove_file(&session.log_path);
        }
    }
}

fn refresh_session_device_flow_from_log(session: &mut CodexCliOauthSession) {
    if session.verification_uri.is_some() && session.user_code.is_some() {
        return;
    }

    if let Ok(log_text) = fs::read_to_string(&session.log_path) {
        let (verification_uri, user_code) = parse_device_flow_from_log(&log_text);
        if session.verification_uri.is_none() {
            session.verification_uri = verification_uri;
        }
        if session.user_code.is_none() {
            session.user_code = user_code;
        }
    }
}

fn parse_device_flow_from_log(raw_text: &str) -> (Option<String>, Option<String>) {
    let plain = ANSI_ESCAPE_RE.replace_all(raw_text, "");
    let verification_uri = VERIFICATION_URL_RE
        .find(&plain)
        .map(|m| m.as_str().trim().to_string());
    let user_code = DEVICE_CODE_RE
        .captures(&plain)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string());
    (verification_uri, user_code)
}

fn read_log_tail(path: &Path, max_lines: usize) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    let tail = lines[start..].join("\n");
    let sanitized = ANSI_ESCAPE_RE.replace_all(&tail, "").to_string();
    if sanitized.trim().is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn try_parse_codex_auth_file() -> Result<Option<ParsedCodexAuth>, String> {
    let auth_path = crate::codex_config::get_codex_auth_path();
    if !auth_path.exists() {
        return Ok(None);
    }

    let content =
        fs::read_to_string(&auth_path).map_err(|e| format!("读取 auth.json 失败: {e}"))?;
    let auth_json: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 auth.json 失败: {e}"))?;

    let access_token = auth_json
        .get("tokens")
        .and_then(Value::as_object)
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .or_else(|| auth_json.get("access_token").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    let Some(access_token) = access_token else {
        return Ok(None);
    };

    let id_token = auth_json
        .get("tokens")
        .and_then(Value::as_object)
        .and_then(|tokens| tokens.get("id_token"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    Ok(Some(ParsedCodexAuth {
        auth_json,
        access_token,
        id_token,
        file_mtime: file_mtime_unix(&auth_path),
    }))
}

fn file_mtime_unix(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_secs()).ok()
}

#[tauri::command]
pub async fn codex_get_quota(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<CodexQuotaUsage, String> {
    let provider = state
        .db
        .get_provider_by_id(&provider_id, AppType::Codex.as_str())
        .map_err(|e: AppError| e.to_string())?
        .ok_or_else(|| format!("未找到 Codex 供应商: {provider_id}"))?;

    let (token, account_id, base_url) = extract_token_and_context(&provider)?;
    let (normalized_base_url, use_wham_path) = normalize_usage_base_url(base_url.as_deref());
    let usage_url = if use_wham_path {
        format!("{}/wham/usage", normalized_base_url)
    } else {
        format!("{}/api/codex/usage", normalized_base_url)
    };

    let mut req = Client::new()
        .get(&usage_url)
        .bearer_auth(token)
        .header("User-Agent", "cc-switch")
        .header("Accept", "application/json");

    if let Some(account_id) = account_id {
        req = req.header("ChatGPT-Account-Id", account_id);
    }

    let res = req
        .send()
        .await
        .map_err(|e| format!("查询 Codex 用量失败: {e}"))?;

    let status = res.status();
    let body: Value = res
        .json()
        .await
        .map_err(|e| format!("解析 Codex 用量响应失败: {e}"))?;

    if !status.is_success() {
        let reason = body
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| body.get("error").and_then(Value::as_str))
            .unwrap_or("未知错误");
        return Err(format!(
            "查询 Codex 用量失败 ({}): {reason}",
            status.as_u16()
        ));
    }

    Ok(parse_quota_payload(&body))
}

fn extract_token_and_context(
    provider: &Provider,
) -> Result<(String, Option<String>, Option<String>), String> {
    let settings = provider
        .settings_config
        .as_object()
        .ok_or("Codex 配置格式错误：settingsConfig 必须为对象")?;
    let auth = settings
        .get("auth")
        .and_then(Value::as_object)
        .ok_or("Codex 配置缺少 auth 字段")?;

    let token = auth
        .get("tokens")
        .and_then(Value::as_object)
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .or_else(|| auth.get("access_token").and_then(Value::as_str))
        .or_else(|| auth.get("OPENAI_API_KEY").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .ok_or("未找到可用的 token，请先登录或填写 API Key")?
        .to_string();

    let mut account_id = auth
        .get("tokens")
        .and_then(Value::as_object)
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            auth.get("chatgpt_account_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    if account_id.is_none() {
        if let Some(id_token) = auth
            .get("tokens")
            .and_then(Value::as_object)
            .and_then(|tokens| tokens.get("id_token"))
            .and_then(Value::as_str)
        {
            account_id = extract_account_id_from_claims(decode_jwt_payload(id_token).as_ref());
        }
    }

    let base_url = settings
        .get("config")
        .and_then(Value::as_str)
        .and_then(extract_base_url_from_toml);

    Ok((token, account_id, base_url))
}

fn normalize_usage_base_url(base_url: Option<&str>) -> (String, bool) {
    let raw = base_url.unwrap_or("").trim().trim_end_matches('/');
    if raw.is_empty() {
        return ("https://chatgpt.com/backend-api".to_string(), true);
    }

    let lower = raw.to_ascii_lowercase();
    if lower.contains("/backend-api") {
        return (raw.to_string(), true);
    }

    let is_chatgpt_host =
        lower.starts_with("https://chatgpt.com") || lower.starts_with("https://chat.openai.com");
    if is_chatgpt_host {
        return (format!("{raw}/backend-api"), true);
    }

    (raw.to_string(), false)
}

fn extract_base_url_from_toml(config_toml: &str) -> Option<String> {
    let re = Regex::new(r#"(?m)^\s*base_url\s*=\s*[\"']([^\"']+)[\"']\s*$"#).ok()?;
    re.captures(config_toml)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
}

fn parse_quota_payload(payload: &Value) -> CodexQuotaUsage {
    let now = Utc::now().timestamp();

    let mut result = CodexQuotaUsage {
        plan_type: payload
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                payload
                    .get("planType")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
        five_hour: None,
        weekly: None,
        fetched_at: now,
    };

    let rate_limit = payload
        .get("rate_limit")
        .or_else(|| payload.get("rateLimit"))
        .or_else(|| payload.get("rate_limits"))
        .or_else(|| payload.get("rateLimits"))
        .unwrap_or(payload);

    let primary = rate_limit
        .get("primary_window")
        .or_else(|| rate_limit.get("primaryWindow"))
        .or_else(|| rate_limit.get("primary"))
        .or_else(|| payload.get("primary_window"))
        .or_else(|| payload.get("primary"));

    let secondary = rate_limit
        .get("secondary_window")
        .or_else(|| rate_limit.get("secondaryWindow"))
        .or_else(|| rate_limit.get("secondary"))
        .or_else(|| payload.get("secondary_window"))
        .or_else(|| payload.get("secondary"));

    let mut windows: Vec<CodexQuotaWindow> = Vec::new();
    if let Some(primary) = primary.and_then(parse_window) {
        windows.push(primary);
    }
    if let Some(secondary) = secondary.and_then(parse_window) {
        windows.push(secondary);
    }

    windows.sort_by_key(|w| w.limit_window_seconds);

    match windows.len() {
        0 => {}
        1 => {
            let only = windows.remove(0);
            if only.limit_window_seconds <= 8 * 3600 {
                result.five_hour = Some(only);
            } else {
                result.weekly = Some(only);
            }
        }
        _ => {
            result.five_hour = windows.first().cloned();
            result.weekly = windows.last().cloned();
        }
    }

    result
}

fn parse_window(value: &Value) -> Option<CodexQuotaWindow> {
    if value.is_null() {
        return None;
    }

    let used_percent = get_i64(value, &["used_percent", "usedPercent"])?;

    let mut limit_window_seconds = get_i64(
        value,
        &[
            "limit_window_seconds",
            "limitWindowSeconds",
            "window_duration_seconds",
        ],
    );

    if limit_window_seconds.is_none() {
        limit_window_seconds =
            get_i64(value, &["window_duration_mins", "windowDurationMins"]).map(|mins| mins * 60);
    }

    let limit_window_seconds = limit_window_seconds?;

    let reset_at =
        get_i64(value, &["reset_at", "resetAt", "resets_at", "resetsAt"]).or_else(|| {
            get_i64(value, &["reset_after_seconds", "resetAfterSeconds"])
                .map(|after| Utc::now().timestamp() + after)
        })?;

    Some(CodexQuotaWindow {
        used_percent,
        limit_window_seconds,
        reset_at,
    })
}

fn get_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(v) = value.get(*key) {
            if let Some(i) = v.as_i64() {
                return Some(i);
            }
            if let Some(u) = v.as_u64() {
                return i64::try_from(u).ok();
            }
            if let Some(s) = v.as_str() {
                if let Ok(parsed) = s.parse::<i64>() {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

fn decode_jwt_payload(jwt: &str) -> Option<Value> {
    let mut parts = jwt.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;

    let decoded = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| URL_SAFE.decode(payload_b64))
        .ok()?;

    serde_json::from_slice::<Value>(&decoded).ok()
}

fn extract_email_from_claims(claims: Option<&Value>) -> Option<String> {
    let claims = claims?;
    claims
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            claims
                .get("https://api.openai.com/profile")
                .and_then(Value::as_object)
                .and_then(|profile| profile.get("email"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn extract_account_id_from_claims(claims: Option<&Value>) -> Option<String> {
    let claims = claims?;
    claims
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
        .and_then(|auth| {
            auth.get("chatgpt_account_id")
                .or_else(|| auth.get("account_id"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn fetch_user_email(client: &Client, access_token: &str) -> Result<Option<String>, String> {
    let res = client
        .get("https://auth0.openai.com/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("请求 userinfo 失败: {e}"))?;

    if !res.status().is_success() {
        return Ok(None);
    }

    let payload: HashMap<String, Value> = res
        .json()
        .await
        .map_err(|e| format!("解析 userinfo 响应失败: {e}"))?;

    Ok(payload
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::parse_device_flow_from_log;

    #[test]
    fn parse_device_flow_from_log_extracts_url_and_code() {
        let sample = format!(
            "\nWelcome to Codex [v0.98.0]\n\
             Follow these steps to sign in with ChatGPT using device code authorization:\n\
             \n\
             1. Open this link in your browser and sign in to your account\n\
             \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m\n\
             \n\
             2. Enter this one-time code\n\
             \u{1b}[94mABCD-1234\u{1b}[0m\n"
        );

        let (url, code) = parse_device_flow_from_log(&sample);
        assert_eq!(url.as_deref(), Some("https://auth.openai.com/codex/device"));
        assert_eq!(code.as_deref(), Some("ABCD-1234"));
    }

    #[test]
    fn parse_device_flow_from_log_handles_missing_values() {
        let sample = "Codex login started";
        let (url, code) = parse_device_flow_from_log(sample);
        assert!(url.is_none());
        assert!(code.is_none());
    }
}
