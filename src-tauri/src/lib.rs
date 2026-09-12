use tauri::State;
use tauri::Emitter;
use crate::models::{DatabaseConnection, DbType};

pub mod drivers;
pub mod error;
pub mod export;
pub mod models;
pub mod storage;
#[cfg(test)]
mod lib_tests;
pub use error::DbError;
pub use storage::ConnectionStore;

use crate::drivers::{mongo_backend, mysql, redis};

// ===== Tauri Commands =====

#[tauri::command]
async fn test_connection(conn: DatabaseConnection) -> Result<String, String> {
    match conn.kind {
        DbType::MySQL => {
            let driver = mysql::MySQLDriver::new();
            driver.test_connection(&conn).await.map_err(|e| e.to_string())?;
        }
        DbType::Mongo => {
            mongo_backend::MongoBackend::test_connection(&conn).await.map_err(|e| e.to_string())?;
        }
        DbType::Redis => {
            let driver = redis::RedisDriver::new();
            driver.test_connection(&conn).await.map_err(|e| e.to_string())?;
        }
    }
    Ok("Connection successful".to_string())
}

#[tauri::command]
async fn save_connection(store: State<'_, ConnectionStore>, conn: DatabaseConnection) -> Result<String, String> {
    let existing = store.get_connection(&conn.id).map_err(|e| e.to_string())?;
    if existing.is_some() {
        store.update_connection(&conn).map_err(|e| e.to_string())?;
    } else {
        store.add_connection(&conn).map_err(|e| e.to_string())?;
    }
    Ok(conn.id)
}

#[tauri::command]
async fn delete_connection(store: State<'_, ConnectionStore>, id: String) -> Result<(), String> {
    store.delete_connection(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_connections(store: State<'_, ConnectionStore>) -> Result<Vec<DatabaseConnection>, String> {
    store.load_connections().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_connection(store: State<'_, ConnectionStore>, id: String) -> Result<Option<DatabaseConnection>, String> {
    store.get_connection(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn execute_query(
    mut conn: DatabaseConnection,
    query: String,
    limit: Option<u32>,
    database: Option<String>,
    collection: Option<String>,
) -> Result<crate::models::QueryResult, String> {
    // 查询页/侧栏选定的"当前数据库"优先于连接配置里的默认库；
    // 空串视为未选择，沿用连接自身配置。
    if let Some(db) = database.filter(|d| !d.trim().is_empty()) {
        conn.database = db;
    }
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.execute_query(&query, limit.map(|l| l as u64)).await.map_err(|e| e.to_string())
        }
        DbType::Mongo => {
            let mut backend = mongo_backend::MongoBackend::connect(&conn).await?;
            backend
                .execute_query(&conn, &query, limit, collection.as_deref())
                .await
        }
        DbType::Redis => {
            redis::execute_command(&conn, &query).await.map_err(|e| e.to_string())
        }
    }
}

#[tauri::command]
async fn update_table_rows(
    conn: DatabaseConnection,
    schema: String,
    table: String,
    updates: Vec<crate::models::RowUpdate>,
) -> Result<u64, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver
                .update_rows(&schema, &table, &updates)
                .await
                .map_err(|e| e.to_string())
        }
        _ => Err("Bulk table editing is only supported for MySQL connections".to_string()),
    }
}

#[tauri::command]
async fn get_schemas(conn: DatabaseConnection) -> Result<Vec<String>, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.get_schemas().await.map_err(|e| e.to_string())
        }
        DbType::Mongo => {
            let mut backend = mongo_backend::MongoBackend::connect(&conn).await?;
            backend.list_databases().await
        }
        DbType::Redis => {
            Ok(vec!["default".to_string()])
        }
    }
}

#[tauri::command]
async fn list_tables(conn: DatabaseConnection, schema: String) -> Result<Vec<crate::models::TableInfo>, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.get_tables(&schema).await.map_err(|e| e.to_string())
        }
        DbType::Mongo => {
            let mut backend = mongo_backend::MongoBackend::connect(&conn).await?;
            backend.list_collections(&schema).await
        }
        DbType::Redis => Ok(Vec::new()),
    }
}

#[tauri::command]
async fn get_table_columns(conn: DatabaseConnection, schema: String, table: String) -> Result<Vec<crate::models::ColumnInfo>, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.get_columns_for_table(&schema, &table).await.map_err(|e| e.to_string())
        }
        DbType::Mongo => {
            let mut backend = mongo_backend::MongoBackend::connect(&conn).await?;
            backend.get_collection_schema(&schema, &table).await
        }
        DbType::Redis => Ok(Vec::new()),
    }
}

#[tauri::command]
async fn list_schema_objects(
    conn: DatabaseConnection,
    schema: String,
) -> Result<crate::models::SchemaObjects, String> {
    use crate::models::SchemaObjects;
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.get_schema_objects(&schema).await.map_err(|e| e.to_string())
        }
        DbType::Mongo => {
            // Mongo 集合放进 tables 分组展示（collections ≈ tables），
            // views/procedures 等 MySQL 特有对象对 Mongo 恒为空。
            let mut backend = mongo_backend::MongoBackend::connect(&conn).await?;
            let tables = backend.list_collections(&schema).await?;
            Ok(SchemaObjects {
                tables,
                ..SchemaObjects::default()
            })
        }
        _ => Ok(SchemaObjects::default()),
    }
}

#[tauri::command]
async fn get_object_ddl(
    conn: DatabaseConnection,
    schema: String,
    object: String,
    kind: crate::models::ObjectKind,
) -> Result<crate::models::DdlInfo, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.get_object_ddl(&schema, &object, &kind).await.map_err(|e| e.to_string())
        }
        _ => Ok(crate::models::DdlInfo {
            object,
            kind,
            ddl: String::new(),
        }),
    }
}

#[tauri::command]
async fn get_table_detail(
    conn: DatabaseConnection,
    schema: String,
    table: String,
) -> Result<crate::models::TableInfo, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.get_table_detail(&schema, &table).await.map_err(|e| e.to_string())
        }
        DbType::Mongo => {
            let mut backend = mongo_backend::MongoBackend::connect(&conn).await?;
            let columns = backend
                .get_collection_schema(&schema, &table)
                .await?;
            Ok(crate::models::TableInfo {
                name: table,
                schema,
                row_count: None,
                columns,
                indexes: Vec::new(),
                comment: None,
                triggers: Vec::new(),
            })
        }
        DbType::Redis => Err("表详情仅支持 MySQL / MongoDB".to_string()),
    }
}

#[tauri::command]
async fn search_tables(conn: DatabaseConnection, pattern: String) -> Result<Vec<crate::models::TableRef>, String> {
    let pattern = pattern.trim().to_string();
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.search_tables(&pattern).await.map_err(|e| e.to_string())
        }
        DbType::Mongo => {
            // Mongo 没有跨库集合元数据查询：降级为库名搜索，name 为空表示
            // 命中的是库本身，前端点击后切换到该库。
            let mut backend = mongo_backend::MongoBackend::connect(&conn).await?;
            let dbs = backend.list_databases().await?;
            let p = pattern.to_lowercase();
            Ok(dbs
                .into_iter()
                .filter(|d| d.to_lowercase().contains(&p))
                .map(|d| crate::models::TableRef {
                    schema: d,
                    name: String::new(),
                    row_count: None,
                })
                .collect())
        }
        DbType::Redis => Ok(Vec::new()),
    }
}

#[tauri::command]
async fn table_page(
    conn: DatabaseConnection,
    schema: String,
    table: String,
    pk_columns: Vec<String>,
    nav: crate::models::PageNav,
    page_size: Option<u32>,
) -> Result<crate::models::TablePage, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver
                .table_page(&schema, &table, &pk_columns, &nav, page_size.unwrap_or(200))
                .await
                .map_err(|e| e.to_string())
        }
        _ => Err("数据表翻页仅支持 MySQL".to_string()),
    }
}

#[tauri::command]
async fn count_table_rows(
    conn: DatabaseConnection,
    schema: String,
    table: String,
) -> Result<u64, String> {
    match conn.kind {
        DbType::MySQL => {
            let mut driver = mysql::MySQLDriver::new();
            driver.connect(&conn).await.map_err(|e| e.to_string())?;
            driver.count_table_rows(&schema, &table).await.map_err(|e| e.to_string())
        }
        _ => Err("精确计数仅支持 MySQL".to_string()),
    }
}

#[tauri::command]
async fn add_query_history(
    store: State<'_, ConnectionStore>,
    connection_id: String,
    query: String,
    success: bool,
    error: Option<String>,
    execution_time_ms: u64,
) -> Result<String, String> {
    let entry = crate::models::QueryHistoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        connection_id,
        query,
        execution_time_ms,
        success,
        error,
        timestamp: chrono::Utc::now(),
    };
    store.append_history(&entry).map_err(|e| e.to_string())?;
    Ok(entry.id)
}

#[tauri::command]
async fn get_query_history(
    store: State<'_, ConnectionStore>,
    connection_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<crate::models::QueryHistoryEntry>, String> {
    let entries = match connection_id {
        Some(id) => store.get_history_for_connection(&id).map_err(|e| e.to_string())?,
        None => store.load_history().map_err(|e| e.to_string())?,
    };

    let entries = if let Some(limit) = limit {
        let len = entries.len();
        entries.into_iter().skip(len.saturating_sub(limit as usize)).collect()
    } else {
        entries
    };

    Ok(entries)
}

#[tauri::command]
async fn clear_query_history(store: State<'_, ConnectionStore>) -> Result<(), String> {
    store.clear_history().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_storage_dir(store: State<'_, ConnectionStore>) -> Result<String, String> {
    Ok(store.data_dir().to_string_lossy().to_string())
}

// ===== Streaming export (Feature C3) =====

/// Cancel flag for the (single) in-flight export. Personal tool: one export
/// at a time; starting a new export resets this flag.
static EXPORT_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Default row cap for exports (50M rows ≈ tens of GB of CSV).
const EXPORT_MAX_ROWS_DEFAULT: u64 = 50_000_000;

/// Table exports run a single streaming `SELECT` (sqlx consumes rows off the
/// wire incrementally, so no keyset loop is needed) — PK-ordered when a PK
/// exists so repeated exports of the same table are byte-stable.
#[tauri::command]
async fn export_stream(
    app: tauri::AppHandle,
    conn: DatabaseConnection,
    source: crate::models::ExportSource,
    format: crate::models::ExportFormat,
    path: String,
    max_rows: Option<u64>,
) -> Result<crate::models::ExportSummary, String> {
    use std::sync::atomic::Ordering;
    EXPORT_CANCEL.store(false, Ordering::Relaxed);

    let mut driver = mysql::MySQLDriver::new();
    driver.connect(&conn).await.map_err(|e| e.to_string())?;

    let (sql, fallback_columns): (String, Option<Vec<String>>) = match &source {
        crate::models::ExportSource::Table { schema, table } => {
            let cols = driver
                .get_columns_for_table(schema, table)
                .await
                .map_err(|e| e.to_string())?;
            let pks: Vec<&crate::models::ColumnInfo> =
                cols.iter().filter(|c| c.is_primary_key).collect();
            let order_by = if pks.is_empty() {
                String::new()
            } else {
                let names: Vec<String> = pks
                    .iter()
                    .map(|c| mysql::object_identifier(&c.name))
                    .collect();
                format!(" ORDER BY {}", names.join(", "))
            };
            let names: Vec<String> = cols.iter().map(|c| c.name.clone()).collect();
            (
                format!(
                    "SELECT * FROM {}{}.{}{}",
                    mysql::object_identifier(schema),
                    ".",
                    mysql::object_identifier(table),
                    order_by
                ),
                Some(names),
            )
        }
        crate::models::ExportSource::Query { sql } => {
            let body = sql.trim().trim_end_matches(';').trim();
            let upper = body.to_uppercase();
            let readonly = (upper.starts_with("SELECT")
                || upper.starts_with("WITH")
                || upper.starts_with('('))
                && !body.contains(';');
            if !readonly {
                return Err("导出仅支持单条只读 SELECT / WITH 查询".to_string());
            }
            (body.to_string(), None)
        }
    };

    let emitter = app.clone();
    let summary = driver
        .export_stream(
            &sql,
            format,
            &path,
            max_rows.unwrap_or(EXPORT_MAX_ROWS_DEFAULT),
            fallback_columns.as_deref(),
            &EXPORT_CANCEL,
            move |rows| {
                let _ = emitter.emit(
                    "export-progress",
                    crate::models::ExportProgress { rows },
                );
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(summary)
}

#[tauri::command]
fn export_cancel() {
    EXPORT_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
}

// ===== Main Entry =====

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ConnectionStore::new().expect("Failed to create connection store"))
        .invoke_handler(tauri::generate_handler![
            test_connection,
            save_connection,
            delete_connection,
            list_connections,
            get_connection,
            execute_query,
            update_table_rows,
            get_schemas,
            list_tables,
            get_table_columns,
            get_table_detail,
            list_schema_objects,
            get_object_ddl,
            search_tables,
            table_page,
            count_table_rows,
            export_stream,
            export_cancel,
            add_query_history,
            get_query_history,
            clear_query_history,
            get_storage_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
