use crate::models::*;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("Connection error: {0}")]
    Connection(#[from] redis::RedisError),
    #[error("Query error: {0}")]
    Query(String),
    #[error("Store error: {0}")]
    Store(#[from] crate::storage::StoreError),
}

pub struct RedisDriver {
    client: Option<redis::Client>,
}

impl RedisDriver {
    pub fn new() -> Self {
        Self { client: None }
    }

    pub async fn connect(&mut self, conn: &DatabaseConnection) -> Result<(), RedisError> {
        let host = match &conn.ssh_tunnel {
            Some(ref ssh) if ssh.enabled => ssh.host.clone(),
            _ => conn.host.clone(),
        };
        let port = match &conn.ssh_tunnel {
            Some(ref ssh) if ssh.enabled => ssh.port,
            _ => conn.port,
        };

        let url = match &conn.auth {
            AuthMethod::Password { username, password } => {
                if username.is_empty() {
                    format!("redis://{}:{}", host, port)
                } else if password.is_empty() {
                    format!("redis://{}@{}:{}", username, host, port)
                } else {
                    format!("redis://{}:{}@{}:{}", username, password, host, port)
                }
            }
            AuthMethod::None => format!("redis://{}:{}", host, port),
            AuthMethod::ConnectionString { url } => url.clone(),
        };

        let client = redis::Client::open(url)?;
        self.client = Some(client);
        Ok(())
    }

    pub async fn execute_command(&self, command: &str) -> Result<QueryResult, RedisError> {
        let client = self.client.as_ref().ok_or_else(|| RedisError::Query("Not connected".to_string()))?;
        let start = std::time::Instant::now();
        let mut con = client.get_multiplexed_async_connection().await?;

        let parts: Vec<&str> = command.trim().split_whitespace().collect();
        if parts.is_empty() {
            return Err(RedisError::Query("Empty command".to_string()));
        }

        let cmd_name = parts[0].to_uppercase();
        let result: QueryResult;

        match cmd_name.as_str() {
            "GET" | "GETEX" => {
                let val: Option<String> = redis::cmd("GET")
                    .arg(parts[1])
                    .query_async(&mut con)
                    .await?;
                result = self.format_key_value(parts[1], val, start);
            }
            "SET" => {
                let mut cmd = redis::cmd("SET");
                for i in 1..parts.len() {
                    cmd.arg(parts[i]);
                }
                cmd.query_async::<()>(&mut con).await?;
                result = QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    rows_affected: Some(1),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                    total_rows: None,
                    message: Some("OK".to_string()),
                };
            }
            "DEL" => {
                let mut cmd = redis::cmd("DEL");
                for i in 1..parts.len() {
                    cmd.arg(parts[i]);
                }
                let count: u64 = cmd.query_async(&mut con).await?;
                result = QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    rows_affected: Some(count),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                    total_rows: None,
                    message: Some(format!("Deleted {} key(s)", count)),
                };
            }
            "KEYS" => {
                let pattern = parts.get(1).copied().unwrap_or("*");
                let keys: Vec<String> = redis::cmd("KEYS")
                    .arg(pattern)
                    .query_async(&mut con)
                    .await?;

                let mut rows = Vec::new();
                for key in &keys {
                    let ttl: i64 = redis::cmd("TTL").arg(&key[..]).query_async(&mut con).await.unwrap_or(-1);
                    let key_type: String = redis::cmd("TYPE").arg(&key[..]).query_async(&mut con).await.unwrap_or_default();

                    let mut map = HashMap::new();
                    map.insert("key".to_string(), serde_json::Value::String(key.clone()));
                    map.insert("type".to_string(), serde_json::Value::String(key_type));
                    map.insert("ttl".to_string(), serde_json::Value::Number(ttl.into()));
                    rows.push(map);
                }

                result = QueryResult {
                    columns: vec!["key".to_string(), "type".to_string(), "ttl".to_string()],
                    rows,
                    rows_affected: Some(keys.len() as u64),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                    total_rows: None,
                    message: Some(format!("{} keys found", keys.len())),
                };
            }
            "DBSIZE" => {
                let size: u64 = redis::cmd("DBSIZE").query_async(&mut con).await?;
                let mut map = HashMap::new();
                map.insert("dbsize".to_string(), serde_json::Value::Number(size.into()));
                result = QueryResult {
                    columns: vec!["dbsize".to_string()],
                    rows: vec![map],
                    rows_affected: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                    total_rows: None,
                    message: Some(format!("Database size: {}", size)),
                };
            }
            "INFO" => {
                let info: String = redis::cmd("INFO").query_async(&mut con).await?;
                let mut map = HashMap::new();
                map.insert("info".to_string(), serde_json::Value::String(info));
                result = QueryResult {
                    columns: vec!["info".to_string()],
                    rows: vec![map],
                    rows_affected: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                    total_rows: None,
                    message: None,
                };
            }
            "FLUSHDB" | "FLUSHALL" => {
                redis::cmd(&cmd_name).query_async::<()>(&mut con).await?;
                result = QueryResult {
                    columns: Vec::new(),
                    rows: Vec::new(),
                    rows_affected: Some(0),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                    total_rows: None,
                    message: Some("OK".to_string()),
                };
            }
            _ => {
                let raw: String = redis::cmd(parts[0])
                    .query_async(&mut con)
                    .await?;
                let mut map = HashMap::new();
                map.insert("result".to_string(), serde_json::Value::String(raw));
                result = QueryResult {
                    columns: vec!["result".to_string()],
                    rows: vec![map],
                    rows_affected: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                    total_rows: None,
                    message: None,
                };
            }
        }

        Ok(result)
    }

    fn format_key_value(&self, key: &str, val: Option<String>, start: std::time::Instant) -> QueryResult {
        let mut map = HashMap::new();
        match val {
            Some(v) => {
                map.insert("key".to_string(), serde_json::Value::String(key.to_string()));
                map.insert("value".to_string(), serde_json::Value::String(v));
            }
            None => {
                map.insert("key".to_string(), serde_json::Value::String(key.to_string()));
                map.insert("value".to_string(), serde_json::Value::Null);
            }
        }

        QueryResult {
            columns: vec!["key".to_string(), "value".to_string()],
            rows: vec![map],
            rows_affected: None,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated: false,
            total_rows: None,
            message: None,
        }
    }

    pub async fn test_connection(&self, conn: &DatabaseConnection) -> Result<(), RedisError> {
        let mut driver = Self::new();
        driver.connect(conn).await?;
        let client = driver.client.as_ref().ok_or_else(|| RedisError::Query("Connection failed".to_string()))?;
        // 连接 + PING 都受 connection_timeout_secs 约束，超时立即返回。
        let timeout = std::time::Duration::from_secs(conn.connection_timeout_secs as u64);
        let mut con = tokio::time::timeout(timeout, client.get_multiplexed_async_connection())
            .await
            .map_err(|_| {
                RedisError::Query(format!("Connection timed out after {}s", conn.connection_timeout_secs))
            })??;
        let ping_cmd = redis::cmd("PING");
        let ping = ping_cmd.query_async::<String>(&mut con);
        tokio::time::timeout(timeout, ping)
            .await
            .map_err(|_| RedisError::Query(format!("PING timed out after {}s", conn.connection_timeout_secs)))??;
        Ok(())
    }
}

pub async fn execute_command(conn: &DatabaseConnection, command: &str) -> Result<QueryResult, RedisError> {
    let mut driver = RedisDriver::new();
    driver.connect(conn).await?;
    driver.execute_command(command).await
}
