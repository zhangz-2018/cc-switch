use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::Utc;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

const GOOGLE_OAUTH_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GOOGLE_OAUTH_CLIENT_SECRET: &str = "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";
const GOOGLE_OAUTH_AUTHORIZE_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_OAUTH_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_OAUTH_USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const GOOGLE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
const GOOGLE_OAUTH_CALLBACK_PORT: u16 = 1456;
const GOOGLE_OAUTH_DEFAULT_EXPIRES_IN: i64 = 5 * 60;
const GOOGLE_OAUTH_DEFAULT_INTERVAL: i64 = 2;
const GOOGLE_OAUTH_SESSION_TTL_SECONDS: i64 = 5 * 60;
const GOOGLE_OAUTH_PORT_IN_USE_CODE: &str = "GEMINI_OAUTH_PORT_IN_USE";

static GOOGLE_OAUTH_SESSIONS: Lazy<Mutex<HashMap<String, GeminiOauthSession>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiOauthInitResponse {
    pub device_code: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    pub interval: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiOauthPollResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

#[derive(Debug, Clone)]
struct GeminiOauthSession {
    started_at: i64,
    expires_at: i64,
    state_token: String,
    redirect_uri: String,
    auth_url: String,
    auth_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfoResponse {
    #[serde(default)]
    email: Option<String>,
}

#[tauri::command]
pub async fn gemini_oauth_init_login() -> Result<GeminiOauthInitResponse, String> {
    start_google_oauth_session()
}

#[tauri::command]
pub async fn gemini_oauth_poll_token(
    device_code: String,
) -> Result<GeminiOauthPollResponse, String> {
    match poll_google_oauth_session(&device_code).await {
        Ok(resp) => Ok(resp),
        Err(err) => Ok(GeminiOauthPollResponse {
            status: "error".to_string(),
            email: None,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            error: Some("oauth_poll_failed".to_string()),
            error_description: Some(err),
        }),
    }
}

fn start_google_oauth_session() -> Result<GeminiOauthInitResponse, String> {
    cleanup_expired_oauth_sessions();

    if let Some((session_id, session)) = get_active_oauth_session() {
        return Ok(GeminiOauthInitResponse {
            device_code: session_id,
            verification_uri: session.auth_url.clone(),
            verification_uri_complete: Some(session.auth_url),
            expires_in: (session.expires_at - Utc::now().timestamp()).max(0),
            interval: GOOGLE_OAUTH_DEFAULT_INTERVAL,
        });
    }

    let listener = bind_oauth_callback_listener()?;

    let session_id = Uuid::new_v4().to_string();
    let started_at = Utc::now().timestamp();
    let expires_at = started_at + GOOGLE_OAUTH_SESSION_TTL_SECONDS;
    let state_token = generate_base64url_token();
    let redirect_uri = format!(
        "http://localhost:{}/oauth-callback",
        GOOGLE_OAUTH_CALLBACK_PORT
    );
    let auth_url = build_auth_url(&redirect_uri, &state_token)?;

    {
        let mut sessions = GOOGLE_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        sessions.insert(
            session_id.clone(),
            GeminiOauthSession {
                started_at,
                expires_at,
                state_token: state_token.clone(),
                redirect_uri,
                auth_url: auth_url.clone(),
                auth_code: None,
            },
        );
    }

    start_callback_server(listener, session_id.clone(), state_token, expires_at);

    Ok(GeminiOauthInitResponse {
        device_code: session_id,
        verification_uri: auth_url.clone(),
        verification_uri_complete: Some(auth_url),
        expires_in: GOOGLE_OAUTH_DEFAULT_EXPIRES_IN,
        interval: GOOGLE_OAUTH_DEFAULT_INTERVAL,
    })
}

async fn poll_google_oauth_session(session_id: &str) -> Result<GeminiOauthPollResponse, String> {
    cleanup_expired_oauth_sessions();

    let session = {
        let sessions = GOOGLE_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth 会话状态锁异常，请重试".to_string())?;
        let Some(session) = sessions.get(session_id) else {
            return Ok(GeminiOauthPollResponse {
                status: "error".to_string(),
                email: None,
                access_token: None,
                refresh_token: None,
                expires_at: None,
                error: Some("oauth_session_not_found".to_string()),
                error_description: Some("OAuth 会话不存在或已过期，请重新登录".to_string()),
            });
        };
        session.clone()
    };

    if Utc::now().timestamp() > session.expires_at {
        remove_oauth_session(session_id);
        return Ok(GeminiOauthPollResponse {
            status: "error".to_string(),
            email: None,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            error: Some("oauth_session_expired".to_string()),
            error_description: Some("Google 登录已超时，请重试".to_string()),
        });
    }

    let Some(code) = session.auth_code.clone() else {
        return Ok(GeminiOauthPollResponse {
            status: "pending".to_string(),
            email: None,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            error: Some("authorization_pending".to_string()),
            error_description: Some("等待浏览器完成 Google 授权".to_string()),
        });
    };

    let token_response = match exchange_code_for_token(&code, &session.redirect_uri).await {
        Ok(tokens) => tokens,
        Err(err) => {
            remove_oauth_session(session_id);
            return Ok(GeminiOauthPollResponse {
                status: "error".to_string(),
                email: None,
                access_token: None,
                refresh_token: None,
                expires_at: None,
                error: Some("oauth_token_exchange_failed".to_string()),
                error_description: Some(err),
            });
        }
    };

    let refresh_token = token_response
        .refresh_token
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string);

    if refresh_token.is_none() {
        remove_oauth_session(session_id);
        return Ok(GeminiOauthPollResponse {
            status: "error".to_string(),
            email: None,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            error: Some("missing_refresh_token".to_string()),
            error_description: Some(
                "Google 未返回 refresh_token，请在 Google 授权管理中移除该应用后重试".to_string(),
            ),
        });
    }

    let access_token = token_response.access_token.trim().to_string();
    let expires_at = Utc::now().timestamp() + token_response.expires_in.unwrap_or(3600);
    let email = fetch_user_email(&Client::new(), &access_token)
        .await
        .ok()
        .flatten();

    remove_oauth_session(session_id);
    Ok(GeminiOauthPollResponse {
        status: "success".to_string(),
        email,
        access_token: Some(access_token),
        refresh_token,
        expires_at: Some(expires_at),
        error: None,
        error_description: None,
    })
}

fn get_active_oauth_session() -> Option<(String, GeminiOauthSession)> {
    let sessions = GOOGLE_OAUTH_SESSIONS.lock().ok()?;
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

fn build_auth_url(redirect_uri: &str, state: &str) -> Result<String, String> {
    let mut url = Url::parse(GOOGLE_OAUTH_AUTHORIZE_ENDPOINT)
        .map_err(|e| format!("构建 Google 授权链接失败: {e}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("client_id", GOOGLE_OAUTH_CLIENT_ID);
        pairs.append_pair("redirect_uri", redirect_uri);
        pairs.append_pair("response_type", "code");
        pairs.append_pair("scope", GOOGLE_OAUTH_SCOPE);
        pairs.append_pair("access_type", "offline");
        pairs.append_pair("prompt", "consent");
        pairs.append_pair("include_granted_scopes", "true");
        pairs.append_pair("state", state);
    }
    Ok(url.to_string())
}

fn bind_oauth_callback_listener() -> Result<TcpListener, String> {
    let listener = TcpListener::bind(("127.0.0.1", GOOGLE_OAUTH_CALLBACK_PORT)).map_err(|e| {
        if e.kind() == ErrorKind::AddrInUse {
            format!(
                "{}:{}（请关闭占用 1456 端口的进程后重试）",
                GOOGLE_OAUTH_PORT_IN_USE_CODE, GOOGLE_OAUTH_CALLBACK_PORT
            )
        } else {
            format!(
                "绑定 Google OAuth 回调端口失败 ({}): {e}",
                GOOGLE_OAUTH_CALLBACK_PORT
            )
        }
    })?;

    listener
        .set_nonblocking(true)
        .map_err(|e| format!("设置 Google OAuth 回调监听失败: {e}"))?;

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
            + Duration::from_secs(GOOGLE_OAUTH_SESSION_TTL_SECONDS.max(1) as u64);

        loop {
            let should_stop = {
                let sessions = match GOOGLE_OAUTH_SESSIONS.lock() {
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
                Ok((stream, _)) => handle_callback_request(stream, &session_id, &expected_state),
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

    if !path.starts_with("/oauth-callback") {
        write_http_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not Found",
        );
        return;
    }

    let query = path.split_once('?').map(|(_, q)| q).unwrap_or_default();
    let params = parse_query_params(query);
    let state = params.get("state").cloned().unwrap_or_default();
    let code = params.get("code").cloned().unwrap_or_default();

    if state != expected_state {
        write_http_response(
            &mut stream,
            "400 Bad Request",
            "text/html; charset=utf-8",
            &oauth_result_html("授权失败", "登录状态校验失败，请返回应用重新发起登录。"),
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
        let mut sessions = match GOOGLE_OAUTH_SESSIONS.lock() {
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
}

async fn exchange_code_for_token(
    code: &str,
    redirect_uri: &str,
) -> Result<GoogleTokenResponse, String> {
    let params = [
        ("client_id", GOOGLE_OAUTH_CLIENT_ID),
        ("client_secret", GOOGLE_OAUTH_CLIENT_SECRET),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];

    let response = Client::new()
        .post(GOOGLE_OAUTH_TOKEN_ENDPOINT)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("请求 Google OAuth Token 失败: {e}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取 Google OAuth Token 响应失败: {e}"))?;

    if !status.is_success() {
        let detail = if body.len() > 400 {
            format!("{}...", &body[..400])
        } else {
            body
        };
        return Err(format!(
            "Google OAuth Token 交换失败 ({}): {}",
            status.as_u16(),
            detail
        ));
    }

    let payload: GoogleTokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析 Google OAuth Token 响应失败: {e}"))?;

    if payload.access_token.trim().is_empty() {
        return Err("Google OAuth Token 响应缺少 access_token".to_string());
    }

    Ok(payload)
}

async fn fetch_user_email(client: &Client, access_token: &str) -> Result<Option<String>, String> {
    let response = client
        .get(GOOGLE_OAUTH_USERINFO_ENDPOINT)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("查询 Google 用户信息失败: {e}"))?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let payload: GoogleUserInfoResponse = response
        .json()
        .await
        .map_err(|e| format!("解析 Google 用户信息失败: {e}"))?;
    Ok(payload.email.filter(|v| !v.trim().is_empty()))
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
        body.len(),
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

fn cleanup_expired_oauth_sessions() {
    let now = Utc::now().timestamp();
    if let Ok(mut sessions) = GOOGLE_OAUTH_SESSIONS.lock() {
        sessions.retain(|_, session| {
            let not_expired = now <= session.expires_at;
            let not_stuck = now - session.started_at <= GOOGLE_OAUTH_SESSION_TTL_SECONDS + 60;
            not_expired && not_stuck
        });
    }
}

fn remove_oauth_session(session_id: &str) {
    if let Ok(mut sessions) = GOOGLE_OAUTH_SESSIONS.lock() {
        sessions.remove(session_id);
    }
}
