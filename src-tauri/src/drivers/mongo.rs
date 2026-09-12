use crate::models::*;
use futures::TryStreamExt;
use mongodb::bson::{self, Document};
use mongodb::{Client, options::ClientOptions};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MongoError {
    #[error("Connection error: {0}")]
    Connection(#[from] mongodb::error::Error),
    #[error("BSON error: {0}")]
    Bson(#[from] bson::ser::Error),
    #[error("BSON deserialize error: {0}")]
    BsonDe(#[from] bson::de::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Query error: {0}")]
    Query(String),
    #[error("Store error: {0}")]
    Store(#[from] crate::storage::StoreError),
}

pub struct MongoDriver {
    client: Option<Client>,
}

impl MongoDriver {
    pub fn new() -> Self {
        Self { client: None }
    }

    pub async fn connect(&mut self, conn: &DatabaseConnection) -> Result<(), MongoError> {
        let timeout = Duration::from_secs(conn.connection_timeout_secs as u64);
        let connection_string = match &conn.auth {
            AuthMethod::Password { username, password } => {
                format!("mongodb://{}:{}@{}:{}/?connectTimeoutMS={}",
                    username, password, conn.host, conn.port, timeout.as_millis())
            }
            AuthMethod::None => {
                format!("mongodb://{}:{}/?connectTimeoutMS={}", conn.host, conn.port, timeout.as_millis())
            }
            AuthMethod::ConnectionString { url } => url.clone(),
        };

        let mut options = ClientOptions::parse(&connection_string).await?;
        options.connect_timeout = Some(timeout);
        options.server_selection_timeout = Some(timeout);
        let client = Client::with_options(options)?;
        client.database("admin").run_command(bson::doc! { "ping": 1 }).await?;
        self.client = Some(client);
        Ok(())
    }

    pub async fn execute_query(&self, conn: &DatabaseConnection, query: &str, limit: Option<u32>) -> Result<QueryResult, MongoError> {
        let client = self.client.as_ref().ok_or_else(|| MongoError::Query("Not connected".into()))?;
        let start = std::time::Instant::now();

        let json_val: serde_json::Value = serde_json::from_str(query).map_err(MongoError::Json)?;
        let filter: Document = bson::to_document(&json_val)?;
        let db = client.database(&conn.database);
        let coll = db.collection::<Document>(&conn.database);

        let mut find_action = coll.find(filter);
        if let Some(l) = limit {
            find_action = find_action.limit(l as i64);
        }

        let mut cursor = find_action.await?;
        let mut columns: Vec<String> = Vec::new();
        let mut rows: Vec<HashMap<String, serde_json::Value>> = Vec::new();

        while let Some(doc) = cursor.try_next().await? {
            if columns.is_empty() {
                columns = doc.keys().cloned().collect();
            }
            let mut map = HashMap::new();
            for (k, v) in doc {
                let json_v: serde_json::Value = bson::from_slice(&bson::to_vec(&v)?)?;
                map.insert(k, json_v);
            }
            rows.push(map);
        }

        let execution_time_ms = start.elapsed().as_millis() as u64;
        let total = rows.len() as u64;
        Ok(QueryResult {
            columns,
            rows,
            rows_affected: None,
            execution_time_ms,
            truncated: false,
            total_rows: Some(total),
            message: None,
        })
    }

    pub async fn list_databases(&self) -> Result<Vec<String>, MongoError> {
        let client = self.client.as_ref().ok_or_else(|| MongoError::Query("Not connected".into()))?;
        let dbs = client.list_database_names().await?;
        Ok(dbs.into_iter().filter(|db| !["admin","local","config"].contains(&db.as_str())).collect())
    }

    pub async fn list_collections(&self, database: &str) -> Result<Vec<TableInfo>, MongoError> {
        let client = self.client.as_ref().ok_or_else(|| MongoError::Query("Not connected".into()))?;
        let db = client.database(database);
        let collections = db.list_collection_names().await?;
        let mut result = Vec::new();
        for coll_name in collections {
            let count = db.collection::<Document>(&coll_name).estimated_document_count().await?;
            result.push(TableInfo {
                name: coll_name,
                schema: database.to_string(),
                row_count: Some(count),
                columns: Vec::new(),
                indexes: Vec::new(),
                comment: None,
                triggers: Vec::new(),
            });
        }
        Ok(result)
    }

    pub async fn get_collection_schema(&self, database: &str, collection: &str) -> Result<Vec<ColumnInfo>, MongoError> {
        let client = self.client.as_ref().ok_or_else(|| MongoError::Query("Not connected".into()))?;
        let db = client.database(database);
        let coll = db.collection::<Document>(collection);
        let mut cursor = coll.find(Document::new()).await?;
        let mut field_map: HashMap<String, (Vec<String>, bool)> = HashMap::new();
        let mut count = 0u32;

        while let Some(doc) = cursor.try_next().await? {
            if count >= 100 { break; }
            Self::infer_fields(&doc, "", &mut field_map);
            count += 1;
        }

        let mut columns: Vec<ColumnInfo> = field_map.into_iter()
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

    pub fn infer_fields(doc: &Document, prefix: &str, map: &mut HashMap<String, (Vec<String>, bool)>) {
        for (key, value) in doc {
            let full_key = if prefix.is_empty() { key.clone() } else { format!("{}.{}", prefix, key) };
            let type_name = match value {
                mongodb::bson::Bson::Null => "null",
                mongodb::bson::Bson::Int32(_) => "int32",
                mongodb::bson::Bson::Int64(_) => "int64",
                mongodb::bson::Bson::Double(_) => "double",
                mongodb::bson::Bson::String(_) => "string",
                mongodb::bson::Bson::Boolean(_) => "boolean",
                mongodb::bson::Bson::DateTime(_) => "datetime",
                mongodb::bson::Bson::Array(_) => "array",
                mongodb::bson::Bson::Document(_) => "document",
                mongodb::bson::Bson::ObjectId(_) => "objectId",
                mongodb::bson::Bson::Binary(_) => "binary",
                _ => "unknown",
            }.to_string();
            let entry = map.entry(full_key.clone()).or_insert((Vec::new(), false));
            if !entry.0.contains(&type_name) { entry.0.push(type_name); }
            if value == &mongodb::bson::Bson::Null { entry.1 = true; }
        }
    }

    pub async fn test_connection(&self, conn: &DatabaseConnection) -> Result<(), MongoError> {
        let mut driver = Self::new();
        driver.connect(conn).await?;
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.client = None;
    }
}
