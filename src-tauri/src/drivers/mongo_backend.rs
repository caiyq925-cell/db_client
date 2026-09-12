//! Mongo backend routing (Feature B).
//!
//! One entry point for every Mongo command: probe `maxWireVersion` over a
//! throwaway legacy-protocol connection (OP_QUERY on `admin.$cmd` for
//! isMaster stays available on all server versions — it is the handshake
//! channel the official drivers use too), then route to the official
//! `mongodb` driver (wire ≥ 6, i.e. 3.6+) or the legacy protocol layer
//! (wire < 6, i.e. 3.2 and older).

use crate::drivers::{mongo, mongo_legacy};
use crate::models::{AuthMethod, ColumnInfo, DatabaseConnection, QueryResult, TableInfo};
use std::collections::HashMap;
use std::time::Duration;

pub enum MongoBackend {
    Official(mongo::MongoDriver),
    Legacy(mongo_legacy::MongoLegacyDriver),
}

/// Effective host/port honouring an SSH tunnel — same rule as mysql/redis.
fn effective_endpoint(conn: &DatabaseConnection) -> (String, u16) {
    match &conn.ssh_tunnel {
        Some(ssh) if ssh.enabled => (ssh.host.clone(), ssh.port),
        _ => (conn.host.clone(), conn.port),
    }
}

impl MongoBackend {
    pub async fn connect(conn: &DatabaseConnection) -> Result<Self, String> {
        // Connection strings bypass local probing (the endpoint is inside the
        // URL); the official driver owns parsing for that path.
        if matches!(conn.auth, AuthMethod::ConnectionString { .. }) {
            let mut d = mongo::MongoDriver::new();
            d.connect(conn).await.map_err(|e| e.to_string())?;
            return Ok(MongoBackend::Official(d));
        }

        let timeout = Duration::from_secs(conn.connection_timeout_secs.max(1) as u64);
        let (host, port) = effective_endpoint(conn);
        let wire = mongo_legacy::probe_wire_version(&host, port, timeout)
            .await
            .map_err(|e| e.to_string())?;

        if wire.is_some_and(|v| v >= 6) {
            let mut d = mongo::MongoDriver::new();
            d.connect(conn).await.map_err(|e| e.to_string())?;
            Ok(MongoBackend::Official(d))
        } else {
            let mut d = mongo_legacy::MongoLegacyDriver::new();
            d.connect(conn).await.map_err(|e| e.to_string())?;
            Ok(MongoBackend::Legacy(d))
        }
    }

    pub async fn test_connection(conn: &DatabaseConnection) -> Result<(), String> {
        // Official connect() pings; legacy connect() runs isMaster + auth.
        // A list_databases round-trip on top keeps the unauthenticated
        // legacy path honest and never fails on empty instances.
        let mut backend = Self::connect(conn).await?;
        backend.list_databases().await?;
        Ok(())
    }

    pub async fn list_databases(&mut self) -> Result<Vec<String>, String> {
        match self {
            MongoBackend::Official(d) => d.list_databases().await.map_err(|e| e.to_string()),
            MongoBackend::Legacy(d) => d.list_databases().await.map_err(|e| e.to_string()),
        }
    }

    pub async fn list_collections(&mut self, database: &str) -> Result<Vec<TableInfo>, String> {
        match self {
            MongoBackend::Official(d) => d.list_collections(database).await.map_err(|e| e.to_string()),
            MongoBackend::Legacy(d) => {
                let items = d.list_collections(database).await.map_err(|e| e.to_string())?;
                Ok(items
                    .into_iter()
                    .map(|(name, count)| TableInfo {
                        name,
                        schema: database.to_string(),
                        row_count: Some(count.max(0) as u64),
                        columns: Vec::new(),
                        indexes: Vec::new(),
                        comment: None,
                        triggers: Vec::new(),
                    })
                    .collect())
            }
        }
    }

    pub async fn get_collection_schema(&mut self, database: &str, collection: &str) -> Result<Vec<ColumnInfo>, String> {
        match self {
            MongoBackend::Official(d) => {
                d.get_collection_schema(database, collection)
                    .await
                    .map_err(|e| e.to_string())
            }
            MongoBackend::Legacy(d) => legacy_collection_schema(d, database, collection).await,
        }
    }

    pub async fn execute_query(
        &mut self,
        conn: &DatabaseConnection,
        query: &str,
        limit: Option<u32>,
    ) -> Result<QueryResult, String> {
        match self {
            MongoBackend::Official(d) => {
                d.execute_query(conn, query, limit).await.map_err(|e| e.to_string())
            }
            MongoBackend::Legacy(d) => {
                d.execute_query(conn, query, limit).await.map_err(|e| e.to_string())
            }
        }
    }
}

/// Legacy schema inference: sample up to 100 documents and flatten field
/// paths, mirroring `MongoDriver::get_collection_schema`.
async fn legacy_collection_schema(
    d: &mut mongo_legacy::MongoLegacyDriver,
    database: &str,
    collection: &str,
) -> Result<Vec<ColumnInfo>, String> {
    let docs = d
        .find_sample(database, collection, bson::Document::new(), 100)
        .await
        .map_err(|e| e.to_string())?;

    let mut field_map: HashMap<String, (Vec<String>, bool)> = HashMap::new();
    for doc in &docs {
        mongo::MongoDriver::infer_fields(doc, "", &mut field_map);
    }

    let mut columns: Vec<ColumnInfo> = field_map
        .into_iter()
        .map(|(name, (types, has_null))| ColumnInfo {
            name,
            data_type: types.join(" | "),
            is_nullable: has_null,
            default_value: None,
            is_primary_key: false,
            comment: None,
        })
        .collect();
    columns.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(columns)
}
