use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("MySQL error: {0}")]
    MySQL(#[from] crate::drivers::mysql::MySQLError),

    #[error("MongoDB error: {0}")]
    Mongo(#[from] crate::drivers::mongo::MongoError),

    #[error("Redis error: {0}")]
    Redis(#[from] crate::drivers::redis::RedisError),

    #[error("Not connected")]
    NotConnected,

    #[error("Unsupported database type")]
    UnsupportedType,

    #[error("Store error: {0}")]
    Store(#[from] crate::storage::StoreError),
}
