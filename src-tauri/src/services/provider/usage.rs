//! Usage script execution
//!
//! Handles executing and formatting usage query results.

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::{UsageData, UsageResult, UsageScript};
use crate::services::antigravity;
use crate::settings;
use crate::store::AppState;
use crate::usage_script;
use super::gemini_auth::is_google_official_gemini;

/// Execute usage script and format result (private helper method)
pub(crate) async fn execute_and_format_usage_result(
    script_code: &str,
    api_key: &str,
    base_url: &str,
    timeout: u64,
    access_token: Option<&str>,
    user_id: Option<&str>,
    template_type: Option<&str>,
) -> Result<UsageResult, AppError> {
    match usage_script::execute_usage_script(
        script_code,
        api_key,
        base_url,
        timeout,
        access_token,
        user_id,
        template_type,
    )
    .await
    {
        Ok(data) => {
            let usage_list: Vec<UsageData> = if data.is_array() {
                serde_json::from_value(data).map_err(|e| {
                    AppError::localized(
                        "usage_script.data_format_error",
                        format!("数据格式错误: {e}"),
                        format!("Data format error: {e}"),
                    )
                })?
            } else {
                let single: UsageData = serde_json::from_value(data).map_err(|e| {
                    AppError::localized(
                        "usage_script.data_format_error",
                        format!("数据格式错误: {e}"),
                        format!("Data format error: {e}"),
                    )
                })?;
                vec![single]
            };

            Ok(UsageResult {
                success: true,
                data: Some(usage_list),
                error: None,
            })
        }
        Err(err) => {
            let lang = settings::get_settings()
                .language
                .unwrap_or_else(|| "zh".to_string());

            let msg = match err {
                AppError::Localized { zh, en, .. } => {
                    if lang == "en" {
                        en
                    } else {
                        zh
                    }
                }
                other => other.to_string(),
            };

            Ok(UsageResult {
                success: false,
                data: None,
                error: Some(msg),
            })
        }
    }
}

/// Extract API key from provider configuration
fn extract_api_key_from_provider(provider: &crate::provider::Provider) -> Option<String> {
    if let Some(env) = provider.settings_config.get("env") {
        // Try multiple possible API key fields
        env.get("GEMINI_API_KEY")
            .or_else(|| env.get("GOOGLE_API_KEY"))
            .or_else(|| env.get("ANTHROPIC_AUTH_TOKEN"))
            .or_else(|| env.get("ANTHROPIC_API_KEY"))
            .or_else(|| env.get("OPENROUTER_API_KEY"))
            .or_else(|| env.get("API_KEY"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    }
}

/// Extract base URL from provider configuration
fn extract_base_url_from_provider(provider: &crate::provider::Provider) -> Option<String> {
    if let Some(env) = provider.settings_config.get("env") {
        // Try multiple possible base URL fields
        env.get("ANTHROPIC_BASE_URL")
            .or_else(|| env.get("GOOGLE_GEMINI_BASE_URL"))
            .or_else(|| env.get("GEMINI_BASE_URL"))
            .or_else(|| env.get("BASE_URL"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim_end_matches('/').to_string())
    } else {
        None
    }
}

/// Query provider usage (using saved script configuration)
pub async fn query_usage(
    state: &AppState,
    app_type: AppType,
    provider_id: &str,
) -> Result<UsageResult, AppError> {
    let (
        provider_snapshot,
        script_config,
        api_key_from_provider,
        base_url_from_provider,
        should_use_antigravity_quota,
        should_use_google_oauth_quota,
    ) = {
        let providers = state.db.get_all_providers(app_type.as_str())?;
        let provider = providers.get(provider_id).ok_or_else(|| {
            AppError::localized(
                "provider.not_found",
                format!("供应商不存在: {provider_id}"),
                format!("Provider not found: {provider_id}"),
            )
        })?;

        let usage_script = provider
            .meta
            .as_ref()
            .and_then(|m| m.usage_script.as_ref())
            .cloned();

        let should_use_antigravity_quota =
            app_type == AppType::Gemini && antigravity::is_antigravity_provider(provider);
        let should_use_google_oauth_quota = app_type == AppType::Gemini
            && is_google_official_gemini(provider)
            && antigravity::has_google_oauth_access_token(provider);

        (
            provider.clone(),
            usage_script,
            extract_api_key_from_provider(provider),
            extract_base_url_from_provider(provider),
            should_use_antigravity_quota,
            should_use_google_oauth_quota,
        )
    };

    // 优先使用已启用的脚本配置（保持现有行为）
    if let Some(usage_script) = script_config.as_ref().filter(|s| s.enabled) {
        let api_key = usage_script
            .api_key
            .clone()
            .filter(|k| !k.is_empty())
            .or(api_key_from_provider)
            .unwrap_or_default();

        let base_url = usage_script
            .base_url
            .clone()
            .filter(|u| !u.is_empty())
            .or(base_url_from_provider)
            .unwrap_or_default();

        return execute_and_format_usage_result(
            &usage_script.code,
            &api_key,
            &base_url,
            usage_script.timeout.unwrap_or(10),
            usage_script.access_token.as_deref(),
            usage_script.user_id.as_deref(),
            usage_script.template_type.as_deref(),
        )
        .await;
    }

    // Gemini 官方账号：无脚本时自动走多模型余量接口（Antigravity / Google OAuth）
    if should_use_antigravity_quota || should_use_google_oauth_quota {
        return antigravity::query_usage_from_provider(&provider_snapshot).await;
    }

    // 其他供应商保持原有错误提示
    if script_config.is_none() {
        return Err(AppError::localized(
            "provider.usage.script.missing",
            "未配置用量查询脚本",
            "Usage script is not configured",
        ));
    }

    Err(AppError::localized(
        "provider.usage.disabled",
        "用量查询未启用",
        "Usage query is disabled",
    ))
}

/// Test usage script (using temporary script content, not saved)
#[allow(clippy::too_many_arguments)]
pub async fn test_usage_script(
    _state: &AppState,
    _app_type: AppType,
    _provider_id: &str,
    script_code: &str,
    timeout: u64,
    api_key: Option<&str>,
    base_url: Option<&str>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    template_type: Option<&str>,
) -> Result<UsageResult, AppError> {
    // Use provided credential parameters directly for testing
    execute_and_format_usage_result(
        script_code,
        api_key.unwrap_or(""),
        base_url.unwrap_or(""),
        timeout,
        access_token,
        user_id,
        template_type,
    )
    .await
}

/// Validate UsageScript configuration (boundary checks)
pub(crate) fn validate_usage_script(script: &UsageScript) -> Result<(), AppError> {
    // Validate auto query interval (0-1440 minutes, max 24 hours)
    if let Some(interval) = script.auto_query_interval {
        if interval > 1440 {
            return Err(AppError::localized(
                "usage_script.interval_too_large",
                format!("自动查询间隔不能超过 1440 分钟（24小时），当前值: {interval}"),
                format!(
                    "Auto query interval cannot exceed 1440 minutes (24 hours), current: {interval}"
                ),
            ));
        }
    }

    Ok(())
}
