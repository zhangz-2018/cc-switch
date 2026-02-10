use std::path::PathBuf;

use crate::codex_config::get_codex_config_dir;

/// 清理 Codex CLI 的本地认证缓存（不删除 auth.json）
pub fn clear_codex_auth_cache() -> Result<(), String> {
    let dir = get_codex_config_dir();
    let mut paths = Vec::new();

    // 常见缓存文件
    paths.push(dir.join(".codex-global-state.json"));
    paths.push(dir.join("models_cache.json"));

    // 常见缓存目录
    paths.push(dir.join("sqlite"));
    paths.push(dir.join("tmp"));

    for path in paths {
        if !path.exists() {
            continue;
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| format!("删除目录失败 {}: {e}", path.display()))?;
        } else {
            std::fs::remove_file(&path)
                .map_err(|e| format!("删除文件失败 {}: {e}", path.display()))?;
        }
    }

    Ok(())
}

/// 提供给需要调试时使用的路径输出
#[allow(dead_code)]
fn _debug_paths() -> Vec<PathBuf> {
    let dir = get_codex_config_dir();
    vec![
        dir.join(".codex-global-state.json"),
        dir.join("models_cache.json"),
        dir.join("sqlite"),
        dir.join("tmp"),
    ]
}
