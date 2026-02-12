use chrono::Utc;
use once_cell::sync::Lazy;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MEMORY_MARKER: &str = "【本地会话记忆】";

#[derive(Clone)]
pub struct ThreadMemoryService {
    client: Client,
    endpoint: String,
    username: String,
    password: String,
    max_recent_messages: usize,
    max_context_chars: usize,
    inject_context: bool,
}

impl ThreadMemoryService {
    pub fn from_env() -> Option<Self> {
        let base_url = get_env_or_dotenv("CC_SWITCH_NEO4J_HTTP_URL")
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())?;

        let username = get_env_or_dotenv("CC_SWITCH_NEO4J_USER")?
            .trim()
            .to_string();
        let password = get_env_or_dotenv("CC_SWITCH_NEO4J_PASSWORD")?
            .trim()
            .to_string();

        if username.is_empty() || password.is_empty() {
            return None;
        }

        let database = get_env_or_dotenv("CC_SWITCH_NEO4J_DATABASE")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "neo4j".to_string());

        let endpoint = format!("{base_url}/db/{database}/tx/commit");

        let timeout_secs = parse_env_u64("CC_SWITCH_NEO4J_TIMEOUT_SECS", 4);
        let max_recent_messages = parse_env_usize("CC_SWITCH_NEO4J_CONTEXT_MESSAGES", 8);
        let max_context_chars = parse_env_usize("CC_SWITCH_NEO4J_CONTEXT_CHARS", 2400);
        let inject_context = parse_env_bool("CC_SWITCH_NEO4J_INJECT_CONTEXT", true);

        let client = match Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                log::warn!("初始化 Neo4j HTTP 客户端失败，将禁用线程记忆: {e}");
                return None;
            }
        };

        log::info!("已启用 Neo4j 线程记忆: endpoint={endpoint}");

        Some(Self {
            client,
            endpoint,
            username,
            password,
            max_recent_messages,
            max_context_chars,
            inject_context,
        })
    }

    pub fn inject_context_enabled(&self) -> bool {
        self.inject_context
    }

    pub fn inject_context_for_codex(&self, endpoint: &str, body: &mut Value, context: &str) {
        if context.trim().is_empty() {
            return;
        }

        let context_block = format!(
            "{MEMORY_MARKER}\n{}\n\n【使用要求】仅在相关时引用这些历史信息；若与用户当前指令冲突，以当前指令为准。",
            truncate_to_chars(context, self.max_context_chars),
        );

        if endpoint.contains("/chat/completions") {
            inject_into_chat_completions(body, &context_block);
            return;
        }

        if endpoint.contains("/responses") {
            inject_into_responses(body, &context_block);
        }
    }

    pub async fn build_context(
        &self,
        app_type: &str,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        let thread_id = local_thread_id(app_type, session_id);

        let summary_rows = self
            .execute_statement(
                "MATCH (t:Thread {id: $thread_id})
                 OPTIONAL MATCH (t)-[:HAS_SUMMARY]->(s:Summary)
                 WITH s ORDER BY s.updated_at DESC
                 LIMIT 1
                 RETURN s.content AS summary",
                json!({ "thread_id": thread_id }),
            )
            .await?;

        let summary = summary_rows
            .first()
            .and_then(|row| row.get("summary"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());

        let message_rows = self
            .execute_statement(
                "MATCH (t:Thread {id: $thread_id})-[:HAS_MESSAGE]->(m:Message)
                 RETURN m.role AS role, m.content AS content, m.ts AS ts
                 ORDER BY m.ts DESC
                 LIMIT $limit",
                json!({
                    "thread_id": local_thread_id(app_type, session_id),
                    "limit": self.max_recent_messages as i64
                }),
            )
            .await?;

        let mut messages = Vec::new();
        for row in message_rows.into_iter().rev() {
            let role = row
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let content = row
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if !content.is_empty() {
                messages.push((role, content.to_string()));
            }
        }

        if summary.is_none() && messages.is_empty() {
            return Ok(None);
        }

        let mut lines = Vec::new();
        if let Some(summary) = summary {
            lines.push(format!("历史摘要: {}", truncate_to_chars(&summary, 800)));
        }
        if !messages.is_empty() {
            lines.push("最近对话:".to_string());
            for (role, content) in messages {
                let role_label = if role == "assistant" {
                    "助手"
                } else if role == "user" {
                    "用户"
                } else {
                    "系统"
                };
                lines.push(format!(
                    "- {}: {}",
                    role_label,
                    truncate_to_chars(&content, 320)
                ));
            }
        }

        let context = truncate_to_chars(&lines.join("\n"), self.max_context_chars);
        if context.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(context))
        }
    }

    pub async fn persist_exchange(
        &self,
        app_type: &str,
        session_id: &str,
        provider_id: &str,
        request_text: Option<&str>,
        response_text: Option<&str>,
    ) -> Result<(), String> {
        let request_text = request_text.map(str::trim).filter(|v| !v.is_empty());
        let response_text = response_text.map(str::trim).filter(|v| !v.is_empty());
        if request_text.is_none() && response_text.is_none() {
            return Ok(());
        }

        let now_ms = Utc::now().timestamp_millis();
        let thread_id = local_thread_id(app_type, session_id);

        self.execute_statement(
            "MERGE (t:Thread {id: $thread_id})
             ON CREATE SET t.app_type = $app_type, t.session_id = $session_id, t.created_at = $ts
             SET t.updated_at = $ts, t.last_provider_id = $provider_id",
            json!({
                "thread_id": thread_id,
                "app_type": app_type,
                "session_id": session_id,
                "provider_id": provider_id,
                "ts": now_ms
            }),
        )
        .await?;

        if let Some(user_text) = request_text {
            self.insert_message(&thread_id, app_type, provider_id, "user", user_text, now_ms)
                .await?;
        }

        if let Some(assistant_text) = response_text {
            self.insert_message(
                &thread_id,
                app_type,
                provider_id,
                "assistant",
                assistant_text,
                now_ms + 1,
            )
            .await?;
        }

        let mut summary_parts = Vec::new();
        if let Some(user_text) = request_text {
            summary_parts.push(format!("用户: {}", truncate_to_chars(user_text, 280)));
        }
        if let Some(assistant_text) = response_text {
            summary_parts.push(format!("助手: {}", truncate_to_chars(assistant_text, 520)));
        }

        if !summary_parts.is_empty() {
            self.execute_statement(
                "MATCH (t:Thread {id: $thread_id})
                 MERGE (s:Summary {thread_id: $thread_id})
                 ON CREATE SET s.id = $summary_id, s.app_type = $app_type
                 SET s.content = $content, s.updated_at = $ts
                 MERGE (t)-[:HAS_SUMMARY]->(s)",
                json!({
                    "thread_id": thread_id,
                    "summary_id": format!("summary-{thread_id}"),
                    "app_type": app_type,
                    "content": truncate_to_chars(&summary_parts.join("\n"), 960),
                    "ts": now_ms
                }),
            )
            .await?;
        }

        Ok(())
    }

    pub fn extract_user_text_from_request(app_type: &str, body: &Value) -> Option<String> {
        let _ = app_type;

        if let Some(messages) = body.get("messages").and_then(Value::as_array) {
            for item in messages.iter().rev() {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("");
                if role != "user" {
                    continue;
                }
                let content = item.get("content").map(extract_text).unwrap_or_default();
                let content = content.trim();
                if !content.is_empty() {
                    return Some(content.to_string());
                }
            }
        }

        if let Some(input) = body.get("input") {
            let text = extract_text(input);
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }

        if let Some(prompt) = body.get("prompt").and_then(Value::as_str) {
            let prompt = prompt.trim();
            if !prompt.is_empty() {
                return Some(prompt.to_string());
            }
        }

        None
    }

    pub fn extract_assistant_text_from_response(app_type: &str, body: &Value) -> Option<String> {
        let _ = app_type;

        if let Some(choices) = body.get("choices").and_then(Value::as_array) {
            for choice in choices {
                if let Some(content) = choice
                    .get("message")
                    .and_then(|msg| msg.get("content"))
                    .map(extract_text)
                {
                    let content = content.trim();
                    if !content.is_empty() {
                        return Some(content.to_string());
                    }
                }
            }
        }

        if let Some(output) = body.get("output").and_then(Value::as_array) {
            let mut chunks = Vec::new();
            for item in output {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("");
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
                if role == "assistant" || item_type == "message" {
                    let content = item.get("content").map(extract_text).unwrap_or_default();
                    let content = content.trim();
                    if !content.is_empty() {
                        chunks.push(content.to_string());
                    }
                }
            }
            if !chunks.is_empty() {
                return Some(chunks.join("\n"));
            }
        }

        if let Some(content) = body.get("content").map(extract_text) {
            let content = content.trim();
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }

        None
    }

    pub fn extract_assistant_text_from_sse_events(events: &[Value]) -> Option<String> {
        let mut chunks = Vec::new();

        for event in events {
            if let Some(delta) = event
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("delta"))
                .and_then(|delta| delta.get("content"))
                .map(extract_text)
            {
                let delta = delta.trim();
                if !delta.is_empty() {
                    chunks.push(delta.to_string());
                    continue;
                }
            }

            if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                let delta = delta.trim();
                if !delta.is_empty() {
                    chunks.push(delta.to_string());
                    continue;
                }
            }

            if let Some(item_type) = event.get("type").and_then(Value::as_str) {
                if item_type.ends_with(".delta") {
                    if let Some(delta) = event
                        .get("delta")
                        .map(extract_text)
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                    {
                        chunks.push(delta);
                        continue;
                    }
                }
            }

            if let Some(content) = event
                .get("message")
                .and_then(|message| message.get("content"))
                .map(extract_text)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
            {
                chunks.push(content);
            }
        }

        if chunks.is_empty() {
            None
        } else {
            Some(chunks.join(""))
        }
    }

    async fn insert_message(
        &self,
        thread_id: &str,
        app_type: &str,
        provider_id: &str,
        role: &str,
        content: &str,
        ts: i64,
    ) -> Result<(), String> {
        self.execute_statement(
            "MATCH (t:Thread {id: $thread_id})
             CREATE (m:Message {
               id: $message_id,
               role: $role,
               content: $content,
               ts: $ts,
               app_type: $app_type,
               provider_id: $provider_id
             })
             MERGE (t)-[:HAS_MESSAGE]->(m)",
            json!({
                "thread_id": thread_id,
                "message_id": uuid::Uuid::new_v4().to_string(),
                "role": role,
                "content": truncate_to_chars(content, 4000),
                "ts": ts,
                "app_type": app_type,
                "provider_id": provider_id
            }),
        )
        .await?;
        Ok(())
    }

    async fn execute_statement(
        &self,
        statement: &str,
        parameters: Value,
    ) -> Result<Vec<std::collections::HashMap<String, Value>>, String> {
        let payload = json!({
            "statements": [{
                "statement": statement,
                "parameters": parameters
            }]
        });

        let response = self
            .client
            .post(&self.endpoint)
            .basic_auth(&self.username, Some(&self.password))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("调用 Neo4j 失败: {e}"))?;

        let status = response.status();
        let raw: Value = response
            .json()
            .await
            .map_err(|e| format!("解析 Neo4j 响应失败: {e}"))?;

        if !status.is_success() {
            return Err(format!("Neo4j 请求失败: status={status}, body={raw}"));
        }

        if let Some(errors) = raw.get("errors").and_then(Value::as_array) {
            if let Some(first) = errors.first() {
                let message = first
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("未知错误");
                return Err(format!("Neo4j 查询失败: {message}"));
            }
        }

        Ok(extract_rows_as_maps(&raw))
    }
}

fn inject_into_chat_completions(body: &mut Value, context_block: &str) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    let already_exists = messages.iter().any(|item| {
        item.get("role").and_then(Value::as_str) == Some("system")
            && item
                .get("content")
                .map(extract_text)
                .map(|text| text.contains(MEMORY_MARKER))
                .unwrap_or(false)
    });

    if already_exists {
        return;
    }

    messages.insert(
        0,
        json!({
            "role": "system",
            "content": context_block
        }),
    );
}

fn inject_into_responses(body: &mut Value, context_block: &str) {
    let existing = body
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if existing.contains(MEMORY_MARKER) {
        return;
    }

    let merged = if existing.trim().is_empty() {
        context_block.to_string()
    } else {
        format!("{context_block}\n\n{existing}")
    };

    if let Some(obj) = body.as_object_mut() {
        obj.insert("instructions".to_string(), Value::String(merged));
    }
}

fn extract_rows_as_maps(raw: &Value) -> Vec<std::collections::HashMap<String, Value>> {
    let mut rows = Vec::new();

    let Some(first_result) = raw
        .get("results")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
    else {
        return rows;
    };

    let columns = first_result
        .get("columns")
        .and_then(Value::as_array)
        .map(|cols| {
            cols.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let data_rows = first_result
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for item in data_rows {
        let mut row_map = std::collections::HashMap::new();
        let row_values = item
            .get("row")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, key) in columns.iter().enumerate() {
            row_map.insert(
                key.clone(),
                row_values.get(index).cloned().unwrap_or(Value::Null),
            );
        }
        rows.push(row_map);
    }

    rows
}

fn local_thread_id(app_type: &str, session_id: &str) -> String {
    format!("{app_type}:{session_id}")
}

fn truncate_to_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = String::new();
    for ch in input.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn parse_env_u64(key: &str, default: u64) -> u64 {
    get_env_or_dotenv(key)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn parse_env_usize(key: &str, default: usize) -> usize {
    get_env_or_dotenv(key)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn parse_env_bool(key: &str, default: bool) -> bool {
    if let Some(v) = get_env_or_dotenv(key) {
        let v = v.trim().to_ascii_lowercase();
        return matches!(v.as_str(), "1" | "true" | "yes" | "on");
    }
    default
}

fn extract_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.to_string(),
        Value::Array(items) => items
            .iter()
            .map(extract_text)
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Value::Object(map) => {
            for key in ["text", "output_text", "input_text", "content", "delta"] {
                if let Some(v) = map.get(key) {
                    let text = extract_text(v);
                    if !text.trim().is_empty() {
                        return text;
                    }
                }
            }

            map.values()
                .map(extract_text)
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
        other => other.to_string(),
    }
}

fn get_env_or_dotenv(key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    DOTENV_VALUES
        .get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

static DOTENV_VALUES: Lazy<HashMap<String, String>> = Lazy::new(load_dotenv_values);

fn load_dotenv_values() -> HashMap<String, String> {
    if let Some(path) = find_dotenv_file() {
        match fs::read_to_string(&path) {
            Ok(content) => {
                let parsed = parse_env_like_file(&content);
                if !parsed.is_empty() {
                    log::info!("已加载 .env 配置: {}", path.display());
                }
                parsed
            }
            Err(e) => {
                log::warn!("读取 .env 文件失败，忽略本地配置: {} ({e})", path.display());
                HashMap::new()
            }
        }
    } else {
        HashMap::new()
    }
}

fn find_dotenv_file() -> Option<PathBuf> {
    let mut checked = std::collections::HashSet::new();

    if let Some(path) = std::env::var("CC_SWITCH_ENV_FILE")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.exists())
    {
        return Some(path);
    }

    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = find_dotenv_in_ancestors(&cwd, &mut checked) {
            return Some(found);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if let Some(found) = find_dotenv_in_ancestors(parent, &mut checked) {
                return Some(found);
            }
        }
    }

    None
}

fn find_dotenv_in_ancestors(
    start: &Path,
    checked: &mut std::collections::HashSet<PathBuf>,
) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let candidate = ancestor.join(".env");
        if checked.insert(candidate.clone()) && candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn parse_env_like_file(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            let value = strip_quotes(value.trim());
            map.insert(key.to_string(), value.to_string());
        }
    }

    map
}

fn strip_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}
