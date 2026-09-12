//! MongoDB 3.2 integration tests (Feature B) against the user's real instance.
//!
//! Verifies the full legacy path: wire-version probe → Backend routing →
//! SCRAM-SHA-1 (or MONGODB-CR) auth → read-only browsing commands.

use dbclient_lib::drivers::mongo_backend::MongoBackend;
use dbclient_lib::drivers::mongo_legacy::{probe_wire_version, LegacyConn};
use dbclient_lib::models::*;
use std::time::Duration;

const HOST: &str = "42.192.252.23";
const PORT: u16 = 27017;
const AUTH_DB: &str = "admin";

fn make_test_conn() -> DatabaseConnection {
    DatabaseConnection {
        id: "test-mongo-32".into(),
        name: "Mongo 3.2 测试".into(),
        kind: DbType::Mongo,
        group: "Test".into(),
        host: HOST.into(),
        port: PORT,
        database: AUTH_DB.into(),
        auth: AuthMethod::Password {
            username: "mongouser".into(),
            password: "tke@dongfeng123".into(),
        },
        ssl: false,
        connection_timeout_secs: 10,
        ssh_tunnel: None,
    }
}

#[tokio::test]
async fn test_probe_wire_version_reports_3_2() {
    let wire = probe_wire_version(HOST, PORT, Duration::from_secs(10))
        .await
        .expect("探测 maxWireVersion 失败");
    println!("✓ maxWireVersion = {:?}", wire);
    assert_eq!(wire, Some(4), "3.2 的 maxWireVersion 应为 4");
}

#[tokio::test]
async fn test_build_info_version_string() {
    let mut lc = LegacyConn::connect(HOST, PORT, Duration::from_secs(10))
        .await
        .expect("TCP+isMaster 失败");
    // isMaster 回复不含 version；版本串在 buildInfo 命令里。
    let info = lc
        .run_command("admin", bson::doc! { "buildInfo": 1i32 })
        .await
        .expect("buildInfo 失败");
    let (wire, _) = lc.is_master().await.expect("isMaster 失败");
    let version = info.get_str("version").unwrap_or("(missing)");
    println!("✓ server version = {}, wire = {:?}", version, wire);
    assert_eq!(wire, Some(4), "3.2 的 maxWireVersion 应为 4");
    assert!(version.starts_with("3."), "版本串应以 3 开头");
}

#[tokio::test]
async fn test_backend_connect_and_list_databases() {
    let mut backend = MongoBackend::connect(&make_test_conn())
        .await
        .expect("Backend 连接（含认证）失败");
    let dbs = backend.list_databases().await.expect("listDatabases 失败");
    println!("✓ 可见业务库 {} 个: {:?}", dbs.len(), dbs);
    // admin/local/config 被过滤；用户库可能为 0，但命令本身必须成功
}

#[tokio::test]
async fn test_list_collections_and_find_sample() {
    // 注意：该腾讯云账号可列 admin 集合但读不了 admin 数据（error 73
    // "not allowed to visit admin"，腾讯 CMONGO 防护层），业务库可读。
    let conn = make_test_conn();
    let mut backend = MongoBackend::connect(&conn).await.expect("连接失败");

    let dbs = backend.list_databases().await.expect("listDatabases 失败");
    assert!(!dbs.is_empty(), "应有可见业务库");
    let target_db = "order";
    assert!(dbs.contains(&target_db.to_string()), "业务库 {target_db} 应可见，实际: {:?}", dbs);

    let colls = backend
        .list_collections(target_db)
        .await
        .expect("listCollections(order) 失败");
    println!("✓ order 集合 {} 个: {:?}", colls.len(), colls.iter().map(|c| &c.name).collect::<Vec<_>>());
    assert!(!colls.is_empty(), "order 库应有集合");

    let first = &colls[0];
    let cols = backend
        .get_collection_schema(target_db, &first.name)
        .await
        .expect("字段采样推断失败");
    println!("✓ 集合 {} 推断出 {} 个字段: {:?}", first.name, cols.len(), cols.iter().map(|c| &c.name).collect::<Vec<_>>());
    assert!(!cols.is_empty(), "真实集合应至少推断出 _id 字段");
    println!("  行数(count 命令) = {:?}", first.row_count);
}

#[tokio::test]
async fn test_execute_query_json_filter() {
    let mut conn = make_test_conn();
    // 业务库可读；MVP 语义：库名同时用作集合名，order.order 大概率不存在，
    // find 空集合应返回空批次而非报错（与官方驱动路径行为一致）。
    conn.database = "order".into();
    let mut backend = MongoBackend::connect(&conn).await.expect("连接失败");
    let result = backend
        .execute_query(&conn, "{}", Some(10))
        .await
        .expect("execute_query 失败");
    println!("✓ execute_query 返回 {} 行, 列 {:?}", result.rows.len(), result.columns);
}

#[tokio::test]
async fn test_wrong_password_rejected() {
    let mut conn = make_test_conn();
    conn.auth = AuthMethod::Password {
        username: "mongouser".into(),
        password: "wrong-password".into(),
    };
    let result = MongoBackend::connect(&conn).await;
    assert!(result.is_err(), "错误密码必须认证失败");
    let err = result.err().unwrap();
    println!("✓ 错误密码被拒绝: {}", err);
}
