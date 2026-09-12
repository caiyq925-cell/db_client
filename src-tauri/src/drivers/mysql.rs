use crate::models::*;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};
use sqlx::{Column, Row};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MySQLError {
    #[error("Connection error: {0}")]
    Connection(#[from] sqlx::Error),
    #[error("Query error: {0}")]
    Query(String),
    #[error("File I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Store error: {0}")]
    Store(#[from] crate::storage::StoreError),
}

pub struct MySQLDriver {
    pool: Option<MySqlPool>,
}

impl MySQLDriver {
    pub fn new() -> Self {
        Self { pool: None }
    }

    pub async fn connect(&mut self, conn: &DatabaseConnection) -> Result<(), MySQLError> {
        let timeout = Duration::from_secs(conn.connection_timeout_secs as u64);
        let host = conn.ssh_tunnel
            .as_ref()
            .filter(|s| s.enabled)
            .map(|s| s.host.clone())
            .unwrap_or_else(|| conn.host.clone());

        let port = conn.ssh_tunnel
            .as_ref()
            .filter(|s| s.enabled)
            .map(|s| s.port)
            .unwrap_or(conn.port);

        let mut opts = MySqlConnectOptions::new();
        opts = opts.host(&host);
        opts = opts.port(port);
        opts = opts.database(&conn.database);

        match &conn.auth {
            AuthMethod::Password { username, password } => {
                opts = opts.username(username);
                if !password.is_empty() {
                    opts = opts.password(password);
                }
            }
            AuthMethod::None => {}
            AuthMethod::ConnectionString { url } => {
                let parsed = url.parse::<MySqlConnectOptions>()
                    .map_err(|e| MySQLError::Query(format!("Invalid URL: {}", e)))?;
                opts = parsed;
            }
        }

        let pool = MySqlPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(timeout)
            .connect_with(opts)
            .await?;

        self.pool = Some(pool);
        Ok(())
    }

    fn row_to_map(row: &sqlx::mysql::MySqlRow, columns: &[String]) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        for (i, col) in columns.iter().enumerate() {
            let value: serde_json::Value = Self::decode_cell(row, i);
            map.insert(col.clone(), value);
        }
        map
    }

    /// JS `Number` can only hold integers up to 2^53-1; anything larger
    /// (e.g. 19-digit snowflake BIGINT IDs) silently loses precision when the
    /// frontend JSON.parse's the payload. BIGINT values beyond that limit are
    /// therefore shipped as strings — displayed verbatim, and `value_to_bind`
    /// still round-trips them as exact numeric text for WHERE/SET.
    const JS_SAFE_MAX: i128 = 9_007_199_254_740_991;

    fn decode_cell(row: &sqlx::mysql::MySqlRow, i: usize) -> serde_json::Value {
        // Ordered decoders: most MySQL column types decode cleanly to one of these.
        if let Ok(Some(s)) = row.try_get::<Option<String>, _>(i) {
            return serde_json::Value::String(s);
        }
        if let Ok(Some(n)) = row.try_get::<Option<i64>, _>(i) {
            return if (n as i128).abs() > Self::JS_SAFE_MAX {
                serde_json::Value::String(n.to_string())
            } else {
                serde_json::Value::Number(n.into())
            };
        }
        // BIGINT UNSIGNED: u64 decode must come before f64 (which truncates).
        if let Ok(Some(n)) = row.try_get::<Option<u64>, _>(i) {
            return if (n as i128) > Self::JS_SAFE_MAX {
                serde_json::Value::String(n.to_string())
            } else {
                serde_json::Value::Number(serde_json::Number::from(n))
            };
        }
        if let Ok(Some(f)) = row.try_get::<Option<f64>, _>(i) {
            if let Some(num) = serde_json::Number::from_f64(f) {
                return serde_json::Value::Number(num);
            }
        }
        if let Ok(Some(b)) = row.try_get::<Option<bool>, _>(i) {
            return serde_json::Value::Bool(b);
        }
        // Binary/VARBINARY columns (e.g. mysql.user.Host) decode to Vec<u8>;
        // render as lossy UTF-8 so the table shows the value instead of NULL.
        if let Ok(bytes) = row.try_get::<Vec<u8>, _>(i) {
            let text = String::from_utf8_lossy(&bytes);
            return serde_json::Value::String(text.into_owned());
        }
        // NULL / undecodable
        serde_json::Value::Null
    }

    async fn execute_select(
        &self,
        query: &str,
        limit: Option<u64>,
    ) -> Result<QueryResult, MySQLError> {
        let pool = self.pool.as_ref().unwrap();
        let start = std::time::Instant::now();

        let query_with_limit = match limit {
            Some(l) => format!("SELECT * FROM ({}) AS t LIMIT {}", query.trim(), l),
            None => query.to_string(),
        };

        let rows = sqlx::query(&query_with_limit)
            .fetch_all(pool)
            .await?;

        let columns: Vec<String> = if let Some(row) = rows.first() {
            row.columns().iter().map(|c| c.name().to_string()).collect()
        } else {
            Vec::new()
        };

        let mut result_rows = Vec::new();
        for row in &rows {
            result_rows.push(Self::row_to_map(row, &columns));
        }

        let execution_time_ms = start.elapsed().as_millis() as u64;

        Ok(QueryResult {
            columns,
            rows: result_rows,
            rows_affected: None,
            execution_time_ms,
            truncated: limit.is_some() && rows.len() >= (limit.unwrap_or(0) as usize),
            total_rows: None,
            message: None,
        })
    }

    async fn execute_non_select(
        &self,
        query: &str,
    ) -> Result<QueryResult, MySQLError> {
        let pool = self.pool.as_ref().unwrap();
        let start = std::time::Instant::now();

        let statements: Vec<&str> = query.split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if statements.len() > 1 {
            let mut total_affected = 0u64;
            let mut messages = Vec::new();

            for stmt in statements {
                match sqlx::query(stmt).execute(pool).await {
                    Ok(result) => {
                        total_affected += result.rows_affected();
                        messages.push(format!("{} rows affected", result.rows_affected()));
                    }
                    Err(e) => {
                        return Err(MySQLError::Query(format!("Statement failed: {}", e)));
                    }
                }
            }

            return Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                rows_affected: Some(total_affected),
                execution_time_ms: start.elapsed().as_millis() as u64,
                truncated: false,
                total_rows: None,
                message: Some(messages.join("; ")),
            });
        }

        let result = sqlx::query(query)
            .execute(pool)
            .await?;

        Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: Some(result.rows_affected()),
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated: false,
            total_rows: None,
            message: Some(format!("{} rows affected", result.rows_affected())),
        })
    }

    pub async fn execute_query(&self, query: &str, limit: Option<u64>) -> Result<QueryResult, MySQLError> {
        // 用户习惯以分号结尾：SELECT 走 LIMIT 子查询包装时，末尾分号会落在
        // 括号内造成 1064 语法错误，这里统一剥掉（仅剥结尾，不影响语句中间）。
        let trimmed = query.trim().trim_end_matches(';').trim();
        let up = trimmed.to_uppercase();
        let is_pure_select = up.starts_with("SELECT") || up.starts_with("WITH");
        let is_readonly_meta = up.starts_with("SHOW")
            || up.starts_with("DESCRIBE")
            || up.starts_with("DESC ")
            || up.starts_with("EXPLAIN")
            || up.starts_with("STATUS")
            || up.starts_with("USE")
            // CALL 可能返回结果集，不能进 LIMIT 子查询包装，也不能丢弃结果
            || up.starts_with("CALL");

        if is_pure_select {
            self.execute_select(trimmed, limit).await
        } else if is_readonly_meta {
            // SHOW/DESCRIBE/EXPLAIN can't be wrapped in a subquery — run directly.
            self.execute_readonly(trimmed).await
        } else if Self::is_routine_ddl(&up) {
            // 存储过程/函数/触发器/事件 DDL：语句体内含分号，不能按 ';' 拆分；
            // 且 MySQL 预处理协议不支持（1295），必须走 text 协议整句发送。
            self.execute_routine_ddl(trimmed).await
        } else {
            self.execute_non_select(trimmed).await
        }
    }

    /// 存储过程/函数/触发器/事件 DDL 判定（CREATE/ALTER/DROP）。
    /// CREATE 可能带 `OR REPLACE`、`DEFINER=...` 前缀，故按关键词包含判断。
    fn is_routine_ddl(up: &str) -> bool {
        let routine_word = up.contains("PROCEDURE")
            || up.contains(" FUNCTION")
            || up.contains(" TRIGGER")
            || up.contains(" EVENT");
        let is_create = up.starts_with("CREATE") && routine_word;
        let is_alter = up.starts_with("ALTER ")
            && (up.contains("PROCEDURE")
                || up.contains(" FUNCTION")
                || up.contains(" TRIGGER")
                || up.contains(" EVENT"));
        let is_drop = up.starts_with("DROP ")
            && (up.contains("PROCEDURE")
                || up.contains(" FUNCTION")
                || up.contains(" TRIGGER")
                || up.contains(" EVENT"));
        is_create || is_alter || is_drop
    }

    /// 例行 DDL（存储过程等）经 text 协议整句执行，绕过预处理协议 1295 限制。
    async fn execute_routine_ddl(&self, query: &str) -> Result<QueryResult, MySQLError> {
        let pool = self.pool.as_ref().unwrap();
        let start = std::time::Instant::now();
        let result = sqlx::raw_sql(query).execute(pool).await?;
        Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: Some(result.rows_affected()),
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated: false,
            total_rows: None,
            message: Some(format!("{} rows affected", result.rows_affected())),
        })
    }

    async fn execute_readonly(&self, query: &str) -> Result<QueryResult, MySQLError> {
        let pool = self.pool.as_ref().unwrap();
        let start = std::time::Instant::now();

        let rows = sqlx::query(query).fetch_all(pool).await?;

        let columns: Vec<String> = if let Some(row) = rows.first() {
            row.columns().iter().map(|c| c.name().to_string()).collect()
        } else {
            Vec::new()
        };

        let mut result_rows = Vec::new();
        for row in &rows {
            result_rows.push(Self::row_to_map(row, &columns));
        }

        let execution_time_ms = start.elapsed().as_millis() as u64;

        Ok(QueryResult {
            columns,
            rows: result_rows,
            rows_affected: None,
            execution_time_ms,
            truncated: false,
            total_rows: None,
            message: None,
        })
    }

    pub async fn get_schemas(&self) -> Result<Vec<String>, MySQLError> {
        let pool = self.pool.as_ref().unwrap();
        let rows = sqlx::query("SHOW DATABASES")
            .fetch_all(pool)
            .await?;

        let mut schemas = Vec::new();
        for row in rows {
            if let Ok(name) = row.try_get::<String, _>(0) {
                if !["information_schema", "performance_schema", "sys", "mysql"]
                    .contains(&name.as_str())
                {
                    schemas.push(name);
                }
            }
        }
        Ok(schemas)
    }

    /// Lightweight listing for the schema tab: ONE information_schema query.
    /// Columns / indexes / triggers are NOT loaded here — a database with
    /// thousands of tables would otherwise issue thousands of extra queries
    /// and the tab would appear to hang. Full detail is fetched lazily per
    /// table via [`Self::get_table_detail`].
    pub async fn get_tables(&self, schema: &str) -> Result<Vec<TableInfo>, MySQLError> {
        let pool = self.pool.as_ref().unwrap();
        let rows = sqlx::query(
            "SELECT TABLE_NAME, TABLE_ROWS, TABLE_COMMENT FROM INFORMATION_SCHEMA.TABLES
             WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE'
             ORDER BY TABLE_NAME",
        )
        .bind(schema)
        .fetch_all(pool)
        .await?;

        let mut result = Vec::new();
        for table in rows {
            let table_name: String = table.try_get(0)?;
            let row_count: Option<u64> = table.try_get(1).ok();
            let comment: Option<String> = table
                .try_get::<Option<String>, _>(2)
                .ok()
                .flatten()
                .filter(|c| !c.is_empty());

            result.push(TableInfo {
                name: table_name,
                schema: schema.to_string(),
                row_count,
                columns: Vec::new(),
                indexes: Vec::new(),
                comment,
                triggers: Vec::new(),
            });
        }

        Ok(result)
    }

    /// Build a substring-matching LIKE pattern, escaping the user's `%`, `_`
    /// and `\` so they match literally (search box semantics, not wildcard).
    pub(crate) fn like_pattern(pattern: &str) -> String {
        let escaped = pattern
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("%{}%", escaped)
    }

    /// Cross-schema table search for the "find a table" box. Single
    /// information_schema query, capped at 100 hits, sorted biggest-first.
    pub async fn search_tables(&self, pattern: &str) -> Result<Vec<TableRef>, MySQLError> {
        let pool = self.pool.as_ref().unwrap();
        let rows = sqlx::query(
            "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_ROWS FROM INFORMATION_SCHEMA.TABLES
             WHERE TABLE_TYPE = 'BASE TABLE'
               AND TABLE_NAME LIKE ?
               AND TABLE_SCHEMA NOT IN ('mysql', 'information_schema', 'performance_schema', 'sys')
             ORDER BY TABLE_ROWS DESC, TABLE_SCHEMA, TABLE_NAME
             LIMIT 100",
        )
        .bind(Self::like_pattern(pattern))
        .fetch_all(pool)
        .await?;

        let mut result = Vec::new();
        for row in rows {
            result.push(TableRef {
                schema: row.try_get(0)?,
                name: row.try_get(1)?,
                row_count: row.try_get(2).ok(),
            });
        }
        Ok(result)
    }

    pub async fn get_columns_for_table(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>, MySQLError> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            MySQLError::Query("Not connected".to_string())
        })?;
        self.get_columns(pool, schema, table).await
    }

    pub async fn get_columns(&self, pool: &MySqlPool, schema: &str, table: &str) -> Result<Vec<ColumnInfo>, MySQLError> {
        let rows = sqlx::query(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, COLUMN_COMMENT
             FROM INFORMATION_SCHEMA.COLUMNS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY ORDINAL_POSITION"
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await?;

        let mut result = Vec::new();
        for col in rows {
            let name: String = col.try_get(0)?;
            let data_type: String = col.try_get(1)?;
            let is_nullable: String = col.try_get(2)?;
            let default_val: Option<String> = col.try_get(3).ok();
            let col_key: Option<String> = col.try_get(4).ok();
            let comment: Option<String> = col
                .try_get::<Option<String>, _>(5)
                .ok()
                .flatten()
                .filter(|c| !c.is_empty());

            result.push(ColumnInfo {
                name,
                data_type,
                is_nullable: is_nullable == "YES",
                default_value: default_val,
                is_primary_key: col_key.as_deref() == Some("PRI"),
                comment,
            });
        }

        Ok(result)
    }

    async fn get_indexes(&self, pool: &MySqlPool, schema: &str, table: &str) -> Result<Vec<IndexInfo>, MySQLError> {
        let rows = sqlx::query(
            "SELECT INDEX_NAME, COLUMN_NAME, NON_UNIQUE
             FROM INFORMATION_SCHEMA.STATISTICS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY INDEX_NAME, SEQ_IN_INDEX"
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await?;

        let mut result = Vec::new();
        let mut index_map: HashMap<String, (Vec<String>, bool)> = HashMap::new();

        for idx in rows {
            let idx_name: String = idx.try_get(0)?;
            let col_name: String = idx.try_get(1)?;
            // NON_UNIQUE is TINYINT normally but some MySQL builds report it as BIGINT —
            // decode as i64 to be safe, then normalize to a boolean.
            let non_unique: i64 = idx.try_get(2)?;
            let is_unique = non_unique == 0;

            index_map
                .entry(idx_name)
                .or_insert((Vec::new(), is_unique))
                .0
                .push(col_name);
        }

        for (name, (columns, is_unique)) in index_map {
            result.push(IndexInfo { name, columns, is_unique });
        }

        Ok(result)
    }

    /// Fetch the table's COMMENT plus the names of triggers attached to it.
    async fn get_table_meta(
        &self,
        pool: &MySqlPool,
        schema: &str,
        table: &str,
    ) -> (Option<String>, Vec<String>) {
        let comment = sqlx::query(
            "SELECT TABLE_COMMENT FROM INFORMATION_SCHEMA.TABLES
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter().next().and_then(|r| {
                r.try_get::<Option<String>, _>(0)
                    .ok()
                    .and_then(|c| c.filter(|s| !s.is_empty()))
            })
        });

        let triggers = sqlx::query(
            "SELECT TRIGGER_NAME FROM INFORMATION_SCHEMA.TRIGGERS
             WHERE TRIGGER_SCHEMA = ? AND EVENT_OBJECT_TABLE = ?
             ORDER BY ACTION_ORDER",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .ok()
        .map(|rows| {
            rows.into_iter()
                .filter_map(|r| r.try_get::<String, _>(0).ok())
                .collect()
        })
        .unwrap_or_default();

        (comment, triggers)
    }

    pub async fn get_schema_objects(&self, schema: &str) -> Result<SchemaObjects, MySQLError> {
        let pool = self.pool.as_ref().unwrap();

        // Lightweight listing: tables + views in ONE query (name/rows/comment).
        // Columns & indexes are loaded lazily per table via `get_table_detail`,
        // otherwise a database with hundreds of tables would issue hundreds of
        // extra queries and make the tree appear to hang.
        let table_rows = sqlx::query(
            "SELECT TABLE_NAME, TABLE_TYPE, TABLE_ROWS, TABLE_COMMENT
             FROM INFORMATION_SCHEMA.TABLES
             WHERE TABLE_SCHEMA = ?
             ORDER BY TABLE_TYPE, TABLE_NAME",
        )
        .bind(schema)
        .fetch_all(pool)
        .await?;

        let mut tables = Vec::new();
        let mut views = Vec::new();
        for row in table_rows {
            let name: String = row.try_get(0)?;
            let table_type: String = row.try_get(1)?;
            let row_count: Option<u64> = row.try_get(2).ok();
            let comment: Option<String> = row
                .try_get::<Option<String>, _>(3)
                .ok()
                .flatten()
                .filter(|c| !c.is_empty());
            if table_type == "BASE TABLE" {
                tables.push(TableInfo {
                    name,
                    schema: schema.to_string(),
                    row_count,
                    columns: Vec::new(),
                    indexes: Vec::new(),
                    comment,
                    triggers: Vec::new(),
                });
            } else {
                views.push(SchemaObject {
                    name,
                    summary: None,
                });
            }
        }

        // All triggers for this schema in ONE query, grouped by table.
        let trigger_rows = sqlx::query(
            "SELECT EVENT_OBJECT_TABLE, TRIGGER_NAME FROM INFORMATION_SCHEMA.TRIGGERS
             WHERE TRIGGER_SCHEMA = ?
             ORDER BY EVENT_OBJECT_TABLE, ACTION_ORDER",
        )
        .bind(schema)
        .fetch_all(pool)
        .await?;
        let mut trigger_map: HashMap<String, Vec<String>> = HashMap::new();
        for r in trigger_rows {
            let tname: String = r.try_get(0)?;
            let trg: String = r.try_get(1)?;
            trigger_map.entry(tname).or_default().push(trg);
        }
        for t in &mut tables {
            if let Some(v) = trigger_map.get(&t.name) {
                t.triggers = v.clone();
            }
        }

        let mut procedure_rows = sqlx::query("SELECT ROUTINE_NAME FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'PROCEDURE'")
            .bind(schema)
            .fetch_all(pool)
            .await?;

        let procedures = collect_object_names(&mut procedure_rows);

        let mut function_rows = sqlx::query("SELECT ROUTINE_NAME FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'FUNCTION'")
            .bind(schema)
            .fetch_all(pool)
            .await?;

        let functions = collect_object_names(&mut function_rows);

        let mut event_rows = sqlx::query("SELECT EVENT_NAME FROM INFORMATION_SCHEMA.EVENTS WHERE EVENT_SCHEMA = ?")
            .bind(schema)
            .fetch_all(pool)
            .await?;

        let events = collect_object_names(&mut event_rows);

        Ok(SchemaObjects {
            tables,
            views,
            procedures,
            functions,
            events,
            triggers: Vec::new(),
        })
    }

    /// Full detail for a single table: columns (+types/comments), indexes,
    /// comment and triggers. Called lazily when the user expands a table node.
    pub async fn get_table_detail(&self, schema: &str, table: &str) -> Result<TableInfo, MySQLError> {
        let pool = self.pool.as_ref().unwrap();

        let columns = self.get_columns(pool, schema, table).await?;
        let indexes = self.get_indexes(pool, schema, table).await?;
        let (comment, triggers) = self.get_table_meta(pool, schema, table).await;

        let row_count: Option<u64> = sqlx::query(
            "SELECT TABLE_ROWS FROM INFORMATION_SCHEMA.TABLES
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .next()
                .and_then(|r| r.try_get::<Option<u64>, _>(0).ok().flatten())
        });

        Ok(TableInfo {
            name: table.to_string(),
            schema: schema.to_string(),
            row_count,
            columns,
            indexes,
            comment,
            triggers,
        })
    }

    pub async fn get_object_ddl(
        &self,
        schema: &str,
        object: &str,
        kind: &ObjectKind,
    ) -> Result<DdlInfo, MySQLError> {
        let pool = self.pool.as_ref().unwrap();

        let ddl = match kind {
            ObjectKind::Table => self.reconstruct_table_ddl(pool, schema, object).await?,
            ObjectKind::View => self.reconstruct_view_ddl(pool, schema, object).await?,
            ObjectKind::Procedure | ObjectKind::Function => {
                self.reconstruct_routine_ddl(pool, schema, object, kind).await?
            }
            _ => String::new(),
        };

        Ok(DdlInfo {
            object: object.to_string(),
            kind: kind.clone(),
            ddl,
        })
    }

    /// Reconstruct a CREATE TABLE statement from INFORMATION_SCHEMA.
    /// (SHOW CREATE TABLE is not usable via sqlx's prepared protocol — error 1295.)
    async fn reconstruct_table_ddl(
        &self,
        pool: &MySqlPool,
        schema: &str,
        table: &str,
    ) -> Result<String, MySQLError> {
        // columns
        let cols = sqlx::query(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA
             FROM INFORMATION_SCHEMA.COLUMNS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
             ORDER BY ORDINAL_POSITION",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await?;

        let mut ddl = String::new();
        let tbl = object_identifier(table);
        let mut first = true;

        for col in cols {
            let c: String = col.try_get(0)?;
            let ctype: String = col.try_get(1)?;
            let nullable: String = col.try_get(2)?;
            let default_v: Option<String> = col.try_get(3).ok();
            let ckey: Option<String> = col.try_get(4).ok();
            let extra: Option<String> = col.try_get(5).ok();

            let _ = ckey;

            let null_part = if nullable == "NO" {
                "NOT NULL"
            } else {
                ""
            };
            let default_part = default_v
                .as_ref()
                .filter(|d| !d.is_empty())
                .map(|d| {
                    let rendered = if d.eq_ignore_ascii_case("CURRENT_TIMESTAMP") {
                        d.clone()
                    } else {
                        format!("'{}'", d.replace('\'', "''"))
                    };
                    format!(" DEFAULT {}", rendered)
                })
                .unwrap_or_default();
            let extra_part = extra
                .as_ref()
                .filter(|e| !e.is_empty() && !e.eq_ignore_ascii_case("NO"))
                .map(|e| format!(" {}", e))
                .unwrap_or_default();

            ddl.push_str(&format!(
                "{}  {} {} {}{}{}",
                if first { "\n" } else { ",\n" },
                object_identifier(&c),
                ctype,
                null_part,
                default_part,
                extra_part
            ));
            first = false;

            // mark PK columns so we skip adding a separate PK if inlined
            let _ = ckey;
        }

        // primary key
        let pk_cols: Vec<String> = sqlx::query(
            "SELECT COLUMN_NAME FROM INFORMATION_SCHEMA.KEY_COLUMN_USAGE
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND CONSTRAINT_NAME = 'PRIMARY'
             ORDER BY ORDINAL_POSITION",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await?
        .iter()
        .filter_map(|r| r.try_get::<String, _>(0).ok())
        .collect();

        if !pk_cols.is_empty() {
            ddl.push_str(&format!(
                ",\n  PRIMARY KEY ({})",
                pk_cols.iter().map(|c| object_identifier(c)).collect::<Vec<_>>().join(", ")
            ));
        }

        // secondary / unique indexes
        let idx_rows = sqlx::query(
            "SELECT INDEX_NAME, COLUMN_NAME, NON_UNIQUE
             FROM INFORMATION_SCHEMA.STATISTICS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND INDEX_NAME <> 'PRIMARY'
             ORDER BY INDEX_NAME, SEQ_IN_INDEX",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await?;

        let mut idx_map: HashMap<String, (Vec<String>, i64)> = HashMap::new();
        for r in idx_rows {
            let iname: String = r.try_get(0)?;
            let icol: String = r.try_get(1)?;
            let non_unique: i64 = r.try_get(2)?;
            idx_map.entry(iname).or_insert((Vec::new(), non_unique)).0.push(icol);
        }
        for (iname, (cols_v, non_unique)) in idx_map {
            let is_unique = non_unique == 0;
            let prefix = if is_unique { "UNIQUE KEY " } else { "KEY " };
            ddl.push_str(&format!(
                ",\n  {}{} ({})",
                prefix,
                object_identifier(&iname),
                cols_v.iter().map(|c| object_identifier(c)).collect::<Vec<_>>().join(", ")
            ));
        }

        // Assemble the header. Use a comment for the schema so the statement runs
        // cleanly in the connection's default database; the body already ends with the
        // PK / index clauses, just wrap them.
        let header = if schema.is_empty() {
            format!("CREATE TABLE {} (", tbl)
        } else {
            format!(
                "-- Database: {}\nCREATE TABLE {} (",
                object_identifier(schema),
                tbl
            )
        };
        let footer = ") ENGINE=InnoDB;\n-- 提示：由 DBClient 依据 INFORMATION_SCHEMA 重建，执行前请核对";
        let ddl_body = ddl.trim_end();
        let out = format!("{}{}{}", header, ddl_body, footer);

        Ok(out)
    }

    async fn reconstruct_view_ddl(
        &self,
        pool: &MySqlPool,
        schema: &str,
        view: &str,
    ) -> Result<String, MySQLError> {
        let row = sqlx::query(
            "SELECT VIEW_DEFINITION FROM INFORMATION_SCHEMA.VIEWS
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(schema)
        .bind(view)
        .fetch_all(pool)
        .await?;
        if let Some(r) = row.first() {
            let def: Option<String> = r.try_get(0).ok();
            if let Some(d) = def {
                return Ok(format!(
                    "CREATE OR REPLACE VIEW {} AS\n{};\n",
                    object_identifier(view),
                    d
                ));
            }
        }
        Ok(String::new())
    }

    async fn reconstruct_routine_ddl(
        &self,
        pool: &MySqlPool,
        schema: &str,
        name: &str,
        kind: &ObjectKind,
    ) -> Result<String, MySQLError> {
        let rtype = match kind {
            ObjectKind::Procedure => "PROCEDURE",
            _ => "FUNCTION",
        };
        let rows = sqlx::query(
            "SELECT ROUTINE_DEFINITION FROM INFORMATION_SCHEMA.ROUTINES
             WHERE ROUTINE_SCHEMA = ? AND ROUTINE_NAME = ? AND ROUTINE_TYPE = ?",
        )
        .bind(schema)
        .bind(name)
        .bind(rtype)
        .fetch_all(pool)
        .await?;
        let body = match rows.first() {
            Some(r) => match r.try_get::<Option<String>, _>(0).ok().flatten() {
                Some(d) => d,
                None => return Ok(String::new()),
            },
            None => return Ok(String::new()),
        };

        // 参数列表来自 INFORMATION_SCHEMA.PARAMETERS：
        // 函数的返回值也是一行（ORDINAL_POSITION = 0，PARAMETER_NAME 为 NULL）。
        let params = sqlx::query(
            "SELECT ORDINAL_POSITION, PARAMETER_NAME, PARAMETER_MODE, DATA_TYPE
             FROM INFORMATION_SCHEMA.PARAMETERS
             WHERE SPECIFIC_SCHEMA = ? AND SPECIFIC_NAME = ? AND ROUTINE_TYPE = ?
             ORDER BY ORDINAL_POSITION",
        )
        .bind(schema)
        .bind(name)
        .bind(rtype)
        .fetch_all(pool)
        .await?;

        let mut sig_parts: Vec<String> = Vec::new();
        let mut returns = String::new();
        for p in &params {
            let pos: i64 = p.try_get(0).unwrap_or(0);
            let pname: Option<String> = p.try_get(1).ok().flatten();
            let mode: Option<String> = p.try_get(2).ok().flatten();
            let dtype: String = p.try_get(3).unwrap_or_default();
            match pname {
                // 函数返回值行
                None => returns = dtype,
                Some(pn) => {
                    let m = mode.unwrap_or_else(|| "IN".into());
                    sig_parts.push(format!("{} {} {}", m, object_identifier(&pn), dtype));
                }
            }
            let _ = pos;
        }

        let qualified = format!(
            "{}.{}",
            object_identifier(schema),
            object_identifier(name)
        );
        let signature = if rtype == "PROCEDURE" {
            format!("CREATE PROCEDURE {}({})", qualified, sig_parts.join(", "))
        } else {
            let ret = if returns.is_empty() {
                String::new()
            } else {
                format!(" RETURNS {}", returns)
            };
            format!("CREATE FUNCTION {}({}){}", qualified, sig_parts.join(", "), ret)
        };
        Ok(format!("{}\n{}", signature, body))
    }

    pub async fn list_databases(&self) -> Result<Vec<String>, MySQLError> {
        self.get_schemas().await
    }

    /// Apply a batch of row edits inside a single transaction.
    ///
    /// Each [`RowUpdate`] builds a parameterised
    /// `UPDATE `table` SET col=?, ... WHERE pk=?` using the row's original
    /// primary-key values to locate it. Identifiers are backtick-quoted;
    /// all values are bound as parameters (string form), which MySQL coerces
    /// to the column type — safe against injection and type drift.
    pub async fn update_rows(
        &self,
        schema: &str,
        table: &str,
        updates: &[RowUpdate],
    ) -> Result<u64, MySQLError> {
        let pool = self.pool.as_ref().ok_or_else(|| MySQLError::Query("Not connected".into()))?;
        if updates.is_empty() {
            return Ok(0);
        }

        let mut tx = pool.begin().await?;
        let mut total = 0u64;

        for upd in updates {
            if upd.set.is_empty() {
                continue;
            }
            // Deterministic column order so the SET list is stable.
            let mut set_cols: Vec<&String> = upd.set.keys().collect();
            set_cols.sort();

            let mut qb = sqlx::QueryBuilder::<sqlx::MySql>::new("UPDATE ");
            // schema 与 table 必须分别加反引号；整体包裹 `db.table` 会被
            // MySQL 当作单个表名导致 UPDATE 找不到表。
            qb.push(format!(
                "{}.{}",
                object_identifier(schema),
                object_identifier(table)
            ));
            qb.push(" SET ");
            for (i, c) in set_cols.iter().enumerate() {
                if i > 0 {
                    qb.push(", ");
                }
                qb.push(object_identifier(c));
                qb.push(" = ");
                let v = &upd.set[*c];
                qb.push_bind(value_to_bind(v));
            }
            if !upd.where_key.is_empty() {
                let mut where_cols: Vec<&String> = upd.where_key.keys().collect();
                where_cols.sort();
                qb.push(" WHERE ");
                for (i, c) in where_cols.iter().enumerate() {
                    if i > 0 {
                        qb.push(" AND ");
                    }
                    qb.push(object_identifier(c));
                    qb.push(" = ");
                    let v = &upd.where_key[*c];
                    if v.is_null() {
                        qb.push(format!("{} IS NULL", object_identifier(c)));
                    } else {
                        qb.push_bind(value_to_bind(v));
                    }
                }
            }
            let sql = qb.build();
            let result = sql.execute(&mut *tx).await?;
            total += result.rows_affected();
        }

        tx.commit().await?;
        Ok(total)
    }

    pub async fn test_connection(&self, conn: &DatabaseConnection) -> Result<(), MySQLError> {
        let mut driver = Self::new();
        driver.connect(conn).await?;
        Ok(())
    }

    /// Fetch one page of table rows with keyset navigation.
    ///
    /// Keyset paging (`WHERE (pk…) > (last_key)` + `ORDER BY pk`) stays fast on
    /// tables with hundreds of millions of rows, unlike deep `LIMIT … OFFSET`.
    /// One extra row beyond `page_size` is fetched to compute `has_more` and
    /// then dropped. Tables without a primary key fall back to `Offset` nav.
    pub async fn table_page(
        &self,
        schema: &str,
        table: &str,
        pk_columns: &[String],
        nav: &PageNav,
        page_size: u32,
    ) -> Result<TablePage, MySQLError> {
        let pool = self.pool.as_ref().ok_or_else(|| MySQLError::Query("Not connected".into()))?;
        let fetch = page_size.max(1) as usize + 1;
        let qualified = format!("{}.{}", object_identifier(schema), object_identifier(table));

        let pk_ids: Vec<String> = pk_columns.iter().map(|c| object_identifier(c)).collect();
        let order_asc = if pk_ids.is_empty() {
            String::new()
        } else {
            format!(" ORDER BY {}", pk_ids.join(", "))
        };
        let order_desc = if pk_ids.is_empty() {
            String::new()
        } else {
            format!(" ORDER BY {} DESC", pk_ids.join(", "))
        };

        let sql: String;
        let key_binds: Vec<serde_json::Value>;
        // Prev pages are read DESC then reversed so the caller always sees ASC rows.
        let reverse: bool;

        match nav {
            PageNav::First => {
                sql = format!("SELECT * FROM {qualified}{order_asc} LIMIT {fetch}");
                key_binds = Vec::new();
                reverse = false;
            }
            PageNav::Next { key } => {
                let tuple = pk_ids.join(", ");
                let marks = vec!["?"; key.len()].join(", ");
                sql = format!("SELECT * FROM {qualified} WHERE ({tuple}) > ({marks}){order_asc} LIMIT {fetch}");
                key_binds = key.clone();
                reverse = false;
            }
            PageNav::Prev { key } => {
                let tuple = pk_ids.join(", ");
                let marks = vec!["?"; key.len()].join(", ");
                sql = format!("SELECT * FROM {qualified} WHERE ({tuple}) < ({marks}){order_desc} LIMIT {fetch}");
                key_binds = key.clone();
                reverse = true;
            }
            PageNav::Offset { offset } => {
                sql = format!("SELECT * FROM {qualified}{order_asc} LIMIT {fetch} OFFSET {offset}");
                key_binds = Vec::new();
                reverse = false;
            }
        }

        let mut q = sqlx::query(&sql);
        for v in &key_binds {
            // PK columns are NOT NULL; bind string form and let MySQL coerce.
            q = q.bind(value_to_bind(v).unwrap_or_default());
        }
        let mut rows = q.fetch_all(pool).await?;
        // has_more + truncation happen in FETCH order: the extra row is the
        // last one read (DESC tail / ASC tail). Reversal comes afterwards so
        // the caller always sees rows in ASC keyset order.
        let has_more = rows.len() >= fetch;
        if rows.len() >= fetch {
            rows.truncate(fetch - 1);
        }
        if reverse {
            rows.reverse();
        }

        let columns: Vec<String> = rows
            .first()
            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();
        let result_rows: Vec<HashMap<String, serde_json::Value>> = rows
            .iter()
            .map(|r| Self::row_to_map(r, &columns))
            .collect();

        let (first_key, last_key) = if pk_columns.is_empty() || result_rows.is_empty() {
            (None, None)
        } else {
            let extract = |row: &HashMap<String, serde_json::Value>| {
                pk_columns
                    .iter()
                    .map(|c| row.get(c).cloned().unwrap_or(serde_json::Value::Null))
                    .collect::<Vec<_>>()
            };
            (Some(extract(&result_rows[0])), Some(extract(result_rows.last().unwrap())))
        };

        Ok(TablePage {
            columns,
            rows: result_rows,
            has_more,
            first_key,
            last_key,
        })
    }

    /// Exact `SELECT COUNT(*)`. Slow on huge InnoDB tables — the UI treats
    /// this as an explicit action; the estimate comes from INFORMATION_SCHEMA.
    pub async fn count_table_rows(&self, schema: &str, table: &str) -> Result<u64, MySQLError> {
        let pool = self.pool.as_ref().ok_or_else(|| MySQLError::Query("Not connected".into()))?;
        let sql = format!(
            "SELECT COUNT(*) FROM {}.{}",
            object_identifier(schema),
            object_identifier(table)
        );
        let row = sqlx::query(&sql).fetch_one(pool).await?;
        Ok(row.try_get::<i64, _>(0)? as u64)
    }

    /// Stream a readonly SELECT to `path` as CSV / JSONL with O(batch) memory.
    ///
    /// A single `SELECT` runs and rows are consumed from the wire as they
    /// arrive (sqlx `fetch` is a Stream — the driver never buffers the whole
    /// result set), so this works for tables with hundreds of millions of
    /// rows. Progress is reported every `BATCH` rows via `on_progress`; a
    /// batch boundary is also where the file is flushed and `cancel` is
    /// consulted.
    ///
    /// `fallback_columns` supplies the CSV header when the result set is
    /// empty (table exports get INFORMATION_SCHEMA columns; queries get none,
    /// so an empty query export produces an empty file). With rows present,
    /// the header comes from the first row's column metadata.
    pub async fn export_stream(
        &self,
        sql: &str,
        format: crate::models::ExportFormat,
        path: &str,
        max_rows: u64,
        fallback_columns: Option<&[String]>,
        cancel: &std::sync::atomic::AtomicBool,
        mut on_progress: impl FnMut(u64),
    ) -> Result<crate::models::ExportSummary, MySQLError> {
        use crate::models::{ExportFormat, ExportSummary};
        use futures::StreamExt;
        use std::io::Write;
        use std::sync::atomic::Ordering;

        const BATCH: u64 = 5000;
        const BUF_CAP: usize = 1 << 20; // 1 MiB write buffer

        let pool = self.pool.as_ref().ok_or_else(|| MySQLError::Query("Not connected".into()))?;
        let start = std::time::Instant::now();

        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::with_capacity(BUF_CAP, file);

        let mut columns: Vec<String> = Vec::new();
        let mut header_written = false;

        // CSV table export: columns are known up front, so the header is
        // written immediately and an empty table still yields a valid CSV.
        // Query exports discover columns from the first row instead.
        if format == ExportFormat::Csv {
            if let Some(cols) = fallback_columns {
                writer.write_all(crate::export::csv_header(cols).as_bytes())?;
                writer.write_all(b"\n")?;
                columns = cols.to_vec();
                header_written = true;
            }
        }

        let mut stream = sqlx::query(sql).fetch(pool);
        let mut rows_done: u64 = 0;
        let mut truncated = false;
        let mut cancelled = false;

        while let Some(item) = stream.next().await {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let row = item.map_err(MySQLError::from)?;

            if !header_written {
                columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                if format == crate::models::ExportFormat::Csv {
                    writer.write_all(crate::export::csv_header(&columns).as_bytes())?;
                    writer.write_all(b"\n")?;
                }
                header_written = true;
            }

            let map = Self::row_to_map(&row, &columns);
            let line = match format {
                crate::models::ExportFormat::Csv => crate::export::csv_line(&columns, &map),
                crate::models::ExportFormat::JsonLines => crate::export::json_line(&columns, &map),
            };
            writer.write_all(line.as_bytes())?;
            writer.write_all(b"\n")?;
            rows_done += 1;

            if rows_done % BATCH == 0 {
                on_progress(rows_done);
                writer.flush()?;
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
            }
            if rows_done >= max_rows {
                truncated = true;
                break;
            }
        }

        on_progress(rows_done);
        writer.flush()?;

        let bytes = writer.get_ref().metadata()?.len();
        Ok(ExportSummary {
            rows: rows_done,
            bytes,
            elapsed_ms: start.elapsed().as_millis() as u64,
            cancelled,
            truncated,
        })
    }

    pub fn disconnect(&mut self) {
        self.pool = None;
    }
}

/// Extract the first column name from a result set into `SchemaObject` entries.
fn collect_object_names(rows: &mut Vec<sqlx::mysql::MySqlRow>) -> Vec<SchemaObject> {
    let mut out = Vec::new();
    while let Some(row) = rows.pop() {
        if let Ok(name) = row.try_get::<String, _>(0) {
            out.push(SchemaObject {
                name,
                summary: None,
            });
        }
    }
    out
}

/// Backtick-quote a SQL identifier, escaping embedded backticks.
pub(crate) fn object_identifier(s: &str) -> String {
    format!("`{}`", s.replace('`', "``"))
}

/// Render a JSON value as the string form to bind as a parameter.
///
/// Returns `None` for JSON null (binds as SQL `NULL`), otherwise the text
/// representation. IMPORTANT: strings must be bound as their raw text —
/// `Value::to_string()` would JSON-encode them (`"abc"` with quotes) and the
/// quotes would land in the database. Numbers keep their plain text form;
/// arrays/objects are stored as JSON text (for JSON columns).
fn value_to_bind(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(if *b { "1".into() } else { "0".into() }),
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}
