use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use url::Url;
use uuid::Uuid;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::Provider;
use crate::store::AppState;

const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_OAUTH_AUTHORIZE_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_OAUTH_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const CODEX_OAUTH_SCOPE: &str = "openid profile email offline_access";
const CODEX_OAUTH_ORIGINATOR: &str = "codex_vscode";
const CODEX_OAUTH_CALLBACK_PORT: u16 = 1455;
const CODEX_OAUTH_PORT_IN_USE_CODE: &str = "CODEX_OAUTH_PORT_IN_USE";
const CODEX_OAUTH_DEFAULT_EXPIRES_IN: i64 = 5 * 60;
const CODEX_OAUTH_DEFAULT_INTERVAL: i64 = 2;
const CODEX_OAUTH_SESSION_TTL_SECONDS: i64 = 5 * 60;

static CODEX_OAUTH_SESSIONS: Lazy<Mutex<HashMap<String, CodexPkceOauthSession>>> =
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
struct CodexPkceOauthSession {
    started_at: i64,
    expires_at: i64,
    state_token: String,
    code_verifier: String,
    redirect_uri: String,
    auth_url: String,
    auth_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexOAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

#[tauri::command]
pub async fn codex_oauth_init_device_flow() -> Result<CodexOauthDeviceFlowResponse, String> {
    start_browser_oauth_session()
}

#[tauri::command]
pub async fn codex_oauth_poll_token(device_code: String) -> Result<CodexOauthPollResponse, String> {
    match poll_browser_oauth_session(&device_code).await {
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

fn start_browser_oauth_session() -> Result<CodexOauthDeviceFlowResponse, String> {
    cleanup_expired_oauth_sessions();

    if let Some((session_id, session)) = get_active_oauth_session() {
        return Ok(CodexOauthDeviceFlowResponse {
            device_code: session_id,
            user_code: String::new(),
            verification_uri: session.auth_url.clone(),
            verification_uri_complete: Some(session.auth_url),
            expires_in: (session.expires_at - Utc::now().timestamp()).max(0),
            interval: CODEX_OAUTH_DEFAULT_INTERVAL,
        });
    }

    let listener = bind_oauth_callback_listener()?;

    let session_id = Uuid::new_v4().to_string();
    let started_at = Utc::now().timestamp();
    let expires_at = started_at + CODEX_OAUTH_SESSION_TTL_SECONDS;
    let code_verifier = generate_base64url_token();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state_token = generate_base64url_token();
    let redirect_uri = format!(
        "http://localhost:{}/auth/callback",
        CODEX_OAUTH_CALLBACK_PORT
    );
    let auth_url = build_auth_url(&redirect_uri, &code_challenge, &state_token)?;

    {
        let mut sessions = CODEX_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        sessions.insert(
            session_id.clone(),
            CodexPkceOauthSession {
                started_at,
                expires_at,
                state_token: state_token.clone(),
                code_verifier,
                redirect_uri,
                auth_url: auth_url.clone(),
                auth_code: None,
            },
        );
    }

    start_callback_server(listener, session_id.clone(), state_token, expires_at);

    Ok(CodexOauthDeviceFlowResponse {
        device_code: session_id,
        user_code: String::new(),
        verification_uri: auth_url.clone(),
        verification_uri_complete: Some(auth_url),
        expires_in: CODEX_OAUTH_DEFAULT_EXPIRES_IN,
        interval: CODEX_OAUTH_DEFAULT_INTERVAL,
    })
}

async fn poll_browser_oauth_session(session_id: &str) -> Result<CodexOauthPollResponse, String> {
    cleanup_expired_oauth_sessions();

    let session = {
        let sessions = CODEX_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        let Some(session) = sessions.get(session_id) else {
            return Ok(CodexOauthPollResponse {
                status: "error".to_string(),
                auth_json: None,
                email: None,
                error: Some("oauth_session_not_found".to_string()),
                error_description: Some("OAuth 会话不存在或已过期，请重新登录".to_string()),
            });
        };
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

    let Some(code) = session.auth_code.clone() else {
        return Ok(CodexOauthPollResponse {
            status: "pending".to_string(),
            auth_json: None,
            email: None,
            error: Some("authorization_pending".to_string()),
            error_description: Some("等待浏览器完成授权".to_string()),
        });
    };

    let token_response =
        match exchange_code_for_token(&code, &session.code_verifier, &session.redirect_uri).await {
            Ok(tokens) => tokens,
            Err(err) => {
                remove_oauth_session(session_id);
                return Ok(CodexOauthPollResponse {
                    status: "error".to_string(),
                    auth_json: None,
                    email: None,
                    error: Some("oauth_token_exchange_failed".to_string()),
                    error_description: Some(err),
                });
            }
        };

    let auth_json = build_auth_json_from_tokens(&token_response);
    let id_token_claims = token_response
        .id_token
        .as_deref()
        .and_then(decode_jwt_payload);
    let email_from_claims = extract_email_from_claims(id_token_claims.as_ref());
    let email = if email_from_claims.is_some() {
        email_from_claims
    } else {
        fetch_user_email(&Client::new(), &token_response.access_token)
            .await
            .ok()
            .flatten()
    };

    remove_oauth_session(session_id);
    Ok(CodexOauthPollResponse {
        status: "success".to_string(),
        auth_json: Some(auth_json),
        email,
        error: None,
        error_description: None,
    })
}

fn get_active_oauth_session() -> Option<(String, CodexPkceOauthSession)> {
    let sessions = CODEX_OAUTH_SESSIONS.lock().ok()?;
    sessions.iter().find_map(|(id, session)| {
        if Utc::now().timestamp() <= session.expires_at {
            Some((id.clone(), session.clone()))
        } else {
            None
        }
    })
}

fn generate_base64url_token() -> String {
    let bytes: [u8; 32] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

fn build_auth_url(redirect_uri: &str, code_challenge: &str, state: &str) -> Result<String, String> {
    let mut url = Url::parse(CODEX_OAUTH_AUTHORIZE_ENDPOINT)
        .map_err(|e| format!("构建 OAuth 授权链接失败: {e}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", CODEX_OAUTH_CLIENT_ID);
        pairs.append_pair("redirect_uri", redirect_uri);
        pairs.append_pair("scope", CODEX_OAUTH_SCOPE);
        pairs.append_pair("code_challenge", code_challenge);
        pairs.append_pair("code_challenge_method", "S256");
        pairs.append_pair("id_token_add_organizations", "true");
        pairs.append_pair("codex_cli_simplified_flow", "true");
        pairs.append_pair("state", state);
        pairs.append_pair("originator", CODEX_OAUTH_ORIGINATOR);
    }
    Ok(url.to_string())
}

fn bind_oauth_callback_listener() -> Result<TcpListener, String> {
    let listener = TcpListener::bind(("127.0.0.1", CODEX_OAUTH_CALLBACK_PORT)).map_err(|e| {
        if e.kind() == ErrorKind::AddrInUse {
            format!(
                "{}:{}（请关闭占用 1455 端口的进程后重试）",
                CODEX_OAUTH_PORT_IN_USE_CODE, CODEX_OAUTH_CALLBACK_PORT
            )
        } else {
            format!(
                "绑定 OAuth 回调端口失败 ({}): {e}",
                CODEX_OAUTH_CALLBACK_PORT
            )
        }
    })?;

    listener
        .set_nonblocking(true)
        .map_err(|e| format!("设置 OAuth 回调监听失败: {e}"))?;

    Ok(listener)
}

fn start_callback_server(
    listener: TcpListener,
    session_id: String,
    expected_state: String,
    expires_at: i64,
) {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now()
            + Duration::from_secs(CODEX_OAUTH_SESSION_TTL_SECONDS.max(1) as u64);

        loop {
            let should_stop = {
                let sessions = match CODEX_OAUTH_SESSIONS.lock() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                match sessions.get(&session_id) {
                    Some(session) => {
                        session.state_token != expected_state
                            || session.auth_code.is_some()
                            || Utc::now().timestamp() > session.expires_at
                    }
                    None => true,
                }
            };

            if should_stop
                || Utc::now().timestamp() > expires_at
                || std::time::Instant::now() > deadline
            {
                break;
            }

            match listener.accept() {
                Ok((stream, _)) => {
                    handle_callback_request(stream, &session_id, &expected_state);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
    });
}

fn handle_callback_request(mut stream: TcpStream, session_id: &str, expected_state: &str) {
    let Some(path) = read_http_request_path(&mut stream) else {
        write_http_response(
            &mut stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            "Bad Request",
        );
        return;
    };

    if path.starts_with("/auth/callback") {
        let query = path.split_once('?').map(|(_, q)| q).unwrap_or_default();
        let params = parse_query_params(query);
        let state = params.get("state").cloned().unwrap_or_default();
        let code = params.get("code").cloned().unwrap_or_default();

        if state != expected_state {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "text/html; charset=utf-8",
                &oauth_result_html("授权失败", "登录状态已失效，请返回应用重新发起登录。"),
            );
            return;
        }

        if code.trim().is_empty() {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "text/html; charset=utf-8",
                &oauth_result_html("授权失败", "未收到授权码，请关闭页面后重试。"),
            );
            return;
        }

        let stored = {
            let mut sessions = match CODEX_OAUTH_SESSIONS.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            if let Some(session) = sessions.get_mut(session_id) {
                if session.state_token == expected_state {
                    session.auth_code = Some(code);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        if stored {
            write_http_response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                &oauth_result_html("授权成功", "你可以关闭此页面并返回 CC Switch。"),
            );
        } else {
            write_http_response(
                &mut stream,
                "409 Conflict",
                "text/html; charset=utf-8",
                &oauth_result_html("授权失败", "登录会话不存在或已过期，请重新发起登录。"),
            );
        }
        return;
    }

    if path.starts_with("/cancel") {
        write_http_response(
            &mut stream,
            "200 OK",
            "text/plain; charset=utf-8",
            "Login cancelled",
        );
        remove_oauth_session(session_id);
        return;
    }

    write_http_response(
        &mut stream,
        "404 Not Found",
        "text/plain; charset=utf-8",
        "Not Found",
    );
}

fn read_http_request_path(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = [0u8; 8192];
    let mut collected: Vec<u8> = Vec::with_capacity(1024);

    for _ in 0..8 {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&buffer[..n]);
        if collected.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if n < buffer.len() {
            break;
        }
    }

    let request = String::from_utf8_lossy(&collected);
    let first_line = request.lines().next()?.trim();
    let mut parts = first_line.split_whitespace();
    let _method = parts.next()?;
    let path = parts.next()?.trim();
    Some(path.to_string())
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect::<HashMap<String, String>>()
}

fn write_http_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn oauth_result_html(title: &str, message: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{title}</title>
  <style>
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #f6f8fb; }}
    .card {{ width: min(520px, 90vw); background: #fff; border: 1px solid #e6e9ef; border-radius: 14px; padding: 28px; box-shadow: 0 8px 28px rgba(15,23,42,.08); }}
    h1 {{ margin: 0 0 12px 0; font-size: 22px; color: #0f172a; }}
    p {{ margin: 0; color: #334155; line-height: 1.65; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>{title}</h1>
    <p>{message}</p>
  </div>
</body>
</html>"#
    )
}

async fn exchange_code_for_token(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<CodexOAuthTokenResponse, String> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", CODEX_OAUTH_CLIENT_ID),
        ("code_verifier", code_verifier),
    ];

    let response = Client::new()
        .post(CODEX_OAUTH_TOKEN_ENDPOINT)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("请求 OAuth Token 失败: {e}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取 OAuth Token 响应失败: {e}"))?;

    if !status.is_success() {
        let detail = if body.len() > 300 {
            format!("{}...", &body[..300])
        } else {
            body
        };
        return Err(format!(
            "OAuth Token 交换失败 ({}): {}",
            status.as_u16(),
            detail
        ));
    }

    let payload: CodexOAuthTokenResponse =
        serde_json::from_str(&body).map_err(|e| format!("解析 OAuth Token 响应失败: {e}"))?;

    if payload.access_token.trim().is_empty() {
        return Err("OAuth Token 响应缺少 access_token".to_string());
    }

    Ok(payload)
}

fn build_auth_json_from_tokens(payload: &CodexOAuthTokenResponse) -> Value {
    let account_id = payload
        .id_token
        .as_deref()
        .and_then(decode_jwt_payload)
        .as_ref()
        .and_then(|claims| extract_account_id_from_claims(Some(claims)));

    let mut tokens_obj = serde_json::Map::new();
    tokens_obj.insert("access_token".to_string(), json!(payload.access_token));
    if let Some(refresh_token) = payload
        .refresh_token
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        tokens_obj.insert("refresh_token".to_string(), json!(refresh_token));
    }
    if let Some(id_token) = payload.id_token.as_ref().filter(|s| !s.trim().is_empty()) {
        tokens_obj.insert("id_token".to_string(), json!(id_token));
    }
    if let Some(account_id) = account_id.as_ref().filter(|s| !s.trim().is_empty()) {
        tokens_obj.insert("account_id".to_string(), json!(account_id));
    }

    let mut auth_obj = serde_json::Map::new();
    auth_obj.insert("auth_mode".to_string(), json!("chatgpt"));
    auth_obj.insert("last_refresh".to_string(), json!(Utc::now().to_rfc3339()));
    auth_obj.insert("tokens".to_string(), Value::Object(tokens_obj));
    auth_obj.insert("access_token".to_string(), json!(payload.access_token));
    if let Some(refresh_token) = payload
        .refresh_token
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        auth_obj.insert("refresh_token".to_string(), json!(refresh_token));
    }
    if let Some(id_token) = payload.id_token.as_ref().filter(|s| !s.trim().is_empty()) {
        auth_obj.insert("id_token".to_string(), json!(id_token));
    }
    if let Some(account_id) = account_id {
        auth_obj.insert("chatgpt_account_id".to_string(), json!(account_id));
    }

    Value::Object(auth_obj)
}

fn cleanup_expired_oauth_sessions() {
    let now = Utc::now().timestamp();
    if let Ok(mut sessions) = CODEX_OAUTH_SESSIONS.lock() {
        sessions.retain(|_, session| {
            let not_expired = now <= session.expires_at;
            let not_stuck = now - session.started_at <= CODEX_OAUTH_SESSION_TTL_SECONDS + 60;
            not_expired && not_stuck
        });
    }
}

fn remove_oauth_session(session_id: &str) {
    if let Ok(mut sessions) = CODEX_OAUTH_SESSIONS.lock() {
        sessions.remove(session_id);
    }
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
    use super::{build_auth_url, parse_query_params};

    #[test]
    fn parse_query_params_decodes_values() {
        let params = parse_query_params("code=abc%2B123&state=xyz%20test");
        assert_eq!(params.get("code").map(String::as_str), Some("abc+123"));
        assert_eq!(params.get("state").map(String::as_str), Some("xyz test"));
    }

    #[test]
    fn build_auth_url_contains_pkce_and_callback() {
        let url = build_auth_url(
            "http://localhost:1455/auth/callback",
            "challenge-value",
            "state-token",
        )
        .expect("build url");
        assert!(url.contains("code_challenge=challenge-value"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=state-token"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
    }
}
