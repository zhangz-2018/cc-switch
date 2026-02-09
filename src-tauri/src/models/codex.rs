use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccount {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub plan: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub is_current: bool,
}


