use crate::models::*;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Not found")]
    NotFound,
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct ConnectionStore {
    data_dir: PathBuf,
    connections_file: PathBuf,
    history_file: PathBuf,
    groups_file: PathBuf,
}

impl ConnectionStore {
    pub fn new() -> Result<Self> {
        let data_dir = Self::default_data_dir()?;
        fs::create_dir_all(&data_dir)?;
        Ok(Self {
            connections_file: data_dir.join("connections.json"),
            history_file: data_dir.join("query_history.json"),
            groups_file: data_dir.join("groups.json"),
            data_dir,
        })
    }

    fn default_data_dir() -> std::io::Result<PathBuf> {
        // 测试可通过 DBCLIENT_DATA_DIR 重定向数据目录，避免读写真实的 ~/.dbclient
        if let Ok(dir) = std::env::var("DBCLIENT_DATA_DIR") {
            return Ok(PathBuf::from(dir));
        }
        let home = dirs::home_dir().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Home directory not found")
        })?;
        Ok(home.join(".dbclient"))
    }

    // ===== Connections =====

    pub fn load_connections(&self) -> Result<Vec<DatabaseConnection>> {
        if !self.connections_file.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&self.connections_file)?;
        let connections: Vec<DatabaseConnection> = serde_json::from_str(&data)?;
        Ok(connections)
    }

    pub fn save_connections(&self, connections: &[DatabaseConnection]) -> Result<()> {
        let data = serde_json::to_string_pretty(connections)?;
        fs::write(&self.connections_file, data)?;
        Ok(())
    }

    pub fn add_connection(&self, conn: &DatabaseConnection) -> Result<()> {
        let mut connections = self.load_connections()?;
        connections.push(conn.clone());
        self.save_connections(&connections)
    }

    pub fn update_connection(&self, conn: &DatabaseConnection) -> Result<()> {
        let mut connections = self.load_connections()?;
        if let Some(pos) = connections.iter().position(|c| c.id == conn.id) {
            connections[pos] = conn.clone();
            self.save_connections(&connections)
        } else {
            Err(StoreError::NotFound)
        }
    }

    pub fn delete_connection(&self, id: &str) -> Result<()> {
        let connections = self.load_connections()?;
        let filtered: Vec<DatabaseConnection> = connections
            .into_iter()
            .filter(|c| c.id != id)
            .collect();
        self.save_connections(&filtered)
    }

    pub fn get_connection(&self, id: &str) -> Result<Option<DatabaseConnection>> {
        let connections = self.load_connections()?;
        Ok(connections.into_iter().find(|c| c.id == id))
    }

    // ===== Query History =====

    pub fn load_history(&self) -> Result<Vec<QueryHistoryEntry>> {
        if !self.history_file.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&self.history_file)?;
        let history: Vec<QueryHistoryEntry> = serde_json::from_str(&data)?;
        Ok(history)
    }

    pub fn append_history(&self, entry: &QueryHistoryEntry) -> Result<()> {
        let mut history = self.load_history()?;
        history.push(entry.clone());
        // Keep last 1000 entries
        if history.len() > 1000 {
            history = history.split_off(history.len() - 1000);
        }
        let data = serde_json::to_string_pretty(&history)?;
        fs::write(&self.history_file, data)?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<()> {
        fs::write(&self.history_file, "[]")?;
        Ok(())
    }

    pub fn get_history_for_connection(&self, connection_id: &str) -> Result<Vec<QueryHistoryEntry>> {
        let history = self.load_history()?;
        Ok(history
            .into_iter()
            .filter(|h| h.connection_id == connection_id)
            .collect())
    }

    // ===== Groups =====

    pub fn load_groups(&self) -> Result<Vec<ConnectionGroup>> {
        if !self.groups_file.exists() {
            return Ok(vec![ConnectionGroup {
                name: "Default".to_string(),
                connection_ids: Vec::new(),
                collapsed: false,
            }]);
        }
        let data = fs::read_to_string(&self.groups_file)?;
        let groups: Vec<ConnectionGroup> = serde_json::from_str(&data)?;
        Ok(groups)
    }

    pub fn save_groups(&self, groups: &[ConnectionGroup]) -> Result<()> {
        let data = serde_json::to_string_pretty(groups)?;
        fs::write(&self.groups_file, data)?;
        Ok(())
    }

    pub fn reorder_groups(&self, groups: &[ConnectionGroup]) -> Result<()> {
        self.save_groups(groups)
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }
}

impl Default for ConnectionStore {
    fn default() -> Self {
        Self::new().expect("Failed to create connection store")
    }
}
