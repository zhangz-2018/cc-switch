use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::models::codex::CodexAccount;
use rusqlite::{params, OptionalExtension};

impl Database {
    /// 添加 Codex 账号
    pub fn add_codex_account(&self, account: &CodexAccount) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO codex_accounts (
                id, name, email, access_token, refresh_token, expires_at, plan,
                created_at, updated_at, is_current
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                account.id,
                account.name,
                account.email,
                account.access_token,
                account.refresh_token,
                account.expires_at,
                account.plan,
                account.created_at,
                account.updated_at,
                account.is_current,
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 获取所有 Codex 账号
    pub fn list_codex_accounts(&self) -> Result<Vec<CodexAccount>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, email, access_token, refresh_token, expires_at, plan,
                created_at, updated_at, is_current FROM codex_accounts ORDER BY created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let accounts = stmt
            .query_map([], |row| {
                Ok(CodexAccount {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    email: row.get(2)?,
                    access_token: row.get(3)?,
                    refresh_token: row.get(4)?,
                    expires_at: row.get(5)?,
                    plan: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    is_current: row.get(9)?,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(accounts)
    }

    /// 获取单个 Codex 账号
    pub fn get_codex_account(&self, id: &str) -> Result<Option<CodexAccount>, AppError> {
        let conn = lock_conn!(self.conn);
        let account = conn
            .query_row(
                "SELECT id, name, email, access_token, refresh_token, expires_at, plan,
                created_at, updated_at, is_current FROM codex_accounts WHERE id = ?1",
                params![id],
                |row| {
                    Ok(CodexAccount {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        email: row.get(2)?,
                        access_token: row.get(3)?,
                        refresh_token: row.get(4)?,
                        expires_at: row.get(5)?,
                        plan: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                        is_current: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(account)
    }

    /// 设置当前激活账号
    pub fn set_current_codex_account(&self, id: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        // 先将所有账号设为非当前
        conn.execute("UPDATE codex_accounts SET is_current = 0", [])
            .map_err(|e| AppError::Database(e.to_string()))?;

        // 设置指定账号为当前
        conn.execute(
            "UPDATE codex_accounts SET is_current = 1 WHERE id = ?1",
            params![id],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    /// 获取当前激活的 Codex 账号
    pub fn get_current_codex_account(&self) -> Result<Option<CodexAccount>, AppError> {
        let conn = lock_conn!(self.conn);
        let account = conn
            .query_row(
                "SELECT id, name, email, access_token, refresh_token, expires_at, plan,
                created_at, updated_at, is_current FROM codex_accounts WHERE is_current = 1 LIMIT 1",
                [],
                |row| {
                    Ok(CodexAccount {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        email: row.get(2)?,
                        access_token: row.get(3)?,
                        refresh_token: row.get(4)?,
                        expires_at: row.get(5)?,
                        plan: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                        is_current: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(account)
    }

    /// 删除 Codex 账号
    pub fn delete_codex_account(&self, id: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute("DELETE FROM codex_accounts WHERE id = ?1", params![id])
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
