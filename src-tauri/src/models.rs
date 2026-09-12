use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ===== Connection Configuration Types =====

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum DbType {
    MySQL,
    Mongo,
    Redis,
}

impl From<DbType> for String {
    fn from(v: DbType) -> String {
        match v {
            DbType::MySQL => "mysql".to_string(),
            DbType::Mongo => "mongo".to_string(),
            DbType::Redis => "redis".to_string(),
        }
    }
}

impl TryFrom<String> for DbType {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "mysql" => Ok(DbType::MySQL),
            "mongo" => Ok(DbType::Mongo),
            "redis" => Ok(DbType::Redis),
            other => Err(format!("unknown db type: {}", other)),
        }
    }
}

impl std::fmt::Display for DbType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbType::MySQL => write!(f, "mysql"),
            DbType::Mongo => write!(f, "mongodb"),
            DbType::Redis => write!(f, "redis"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTunnelConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// SSH key file path (empty string = use password)
    pub key_path: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AuthMethod {
    /// Username + password auth
    Password { username: String, password: String },
    /// No credentials (e.g., Redis without auth)
    None,
    /// MongoDB connection string
    ConnectionString { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConnection {
    pub id: String,
    pub name: String,
    pub kind: DbType,
    pub group: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub auth: AuthMethod,
    pub ssl: bool,
    pub connection_timeout_secs: u32,
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

impl Default for DatabaseConnection {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: String::new(),
            kind: DbType::MySQL,
            group: "Default".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3306,
            database: String::new(),
            auth: AuthMethod::Password {
                username: "root".to_string(),
                password: String::new(),
            },
            ssl: false,
            connection_timeout_secs: 30,
            ssh_tunnel: None,
        }
    }
}

// ===== Query Result Types =====

/// A single row edit submitted for a bulk table update.
///
/// `where_key` locates the target row (typically its primary-key columns
/// with their original values); `set` are the new column values to apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowUpdate {
    /// Column -> original value used to build the WHERE clause.
    pub where_key: HashMap<String, serde_json::Value>,
    /// Column -> new value to write.
    pub set: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<HashMap<String, serde_json::Value>>,
    pub rows_affected: Option<u64>,
    pub execution_time_ms: u64,
    pub truncated: bool,
    pub total_rows: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryHistoryEntry {
    pub id: String,
    pub connection_id: String,
    pub query: String,
    pub execution_time_ms: u64,
    pub success: bool,
    pub error: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ===== Schema Types =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    /// Column comment (COMMENT clause). `None` when absent.
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub schema: String,
    pub row_count: Option<u64>,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    /// Table comment (COMMENT clause). `None` when absent.
    #[serde(default)]
    pub comment: Option<String>,
    /// Trigger names that fire on this table.
    #[serde(default)]
    pub triggers: Vec<String>,
}

/// Lightweight table reference for cross-schema search results.
/// Unlike [`TableInfo`] it carries no columns/indexes payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRef {
    pub schema: String,
    /// Empty string when the hit is a database-level entry (e.g. Mongo db-name search).
    pub name: String,
    pub row_count: Option<u64>,
}

// ===== Table paging (keyset navigation) =====

/// Navigation target for [`crate::drivers::mysql::MySQLDriver::table_page`].
///
/// Keyset modes (`next`/`prev`) carry the boundary primary-key values of the
/// adjacent page; `offset` is the fallback for tables without a primary key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum PageNav {
    First,
    Next { key: Vec<serde_json::Value> },
    Prev { key: Vec<serde_json::Value> },
    Offset { offset: u64 },
}

/// One page of table rows plus the boundaries the frontend needs to keep
/// navigating in both directions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TablePage {
    pub columns: Vec<String>,
    pub rows: Vec<HashMap<String, serde_json::Value>>,
    /// `true` when at least one more row exists past this page.
    pub has_more: bool,
    /// Primary-key values of this page's first/last row (keyset mode only).
    #[serde(default)]
    pub first_key: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub last_key: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSchema {
    pub name: String,
    pub tables: Vec<TableInfo>,
    pub views: Vec<TableInfo>,
}

// ===== Streaming export (Feature C3) =====

/// What to export: a whole table (keyset-stable order) or one readonly query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum ExportSource {
    Table { schema: String, table: String },
    Query { sql: String },
}

/// Export file format. No xlsx (Excel hard-caps at 1,048,576 rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    JsonLines,
}

/// Final stats returned when an export ends (completed / truncated / cancelled).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSummary {
    pub rows: u64,
    pub bytes: u64,
    pub elapsed_ms: u64,
    /// Stopped early because the user hit cancel.
    pub cancelled: bool,
    /// Stopped early because `max_rows` was reached.
    pub truncated: bool,
}

/// Emitted as `export-progress` after every batch during an export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportProgress {
    pub rows: u64,
}

// ===== Schema object types (grouped under a database) =====

/// A schema object that is not a table (view / procedure / function / event / trigger).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaObject {
    pub name: String,
    /// human-friendly summary, e.g. "def(user, host)" for procedures, definition preview for views
    pub summary: Option<String>,
}

/// Grouped list of schema objects under a database.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchemaObjects {
    pub tables: Vec<TableInfo>,
    pub views: Vec<SchemaObject>,
    pub procedures: Vec<SchemaObject>,
    pub functions: Vec<SchemaObject>,
    pub events: Vec<SchemaObject>,
    pub triggers: Vec<SchemaObject>,
}

impl SchemaObjects {
    pub fn total(&self) -> usize {
        self.tables.len()
            + self.views.len()
            + self.procedures.len()
            + self.functions.len()
            + self.events.len()
            + self.triggers.len()
    }
}

/// Which kind of schema object a DDL request refers to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Table,
    View,
    Procedure,
    Function,
    Event,
    Trigger,
}

/// The result of a "SHOW CREATE ..." style DDL fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdlInfo {
    pub object: String,
    pub kind: ObjectKind,
    pub ddl: String,
}

// ===== Connection Status =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum ConnectionStatus {
    Idle,
    Connecting,
    Connected,
    Error { message: String },
}

// ===== Collection Group (for connection sidebar) =====

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionGroup {
    pub name: String,
    pub connection_ids: Vec<String>,
    pub collapsed: bool,
}
