use dbclient_lib::models::*;
use dbclient_lib::drivers::mysql::MySQLDriver;

fn make_test_conn() -> DatabaseConnection {
    DatabaseConnection {
        id: "test".into(),
        name: "MySQL 测试".into(),
        kind: DbType::MySQL,
        group: "Test".into(),
        host: "42.192.176.87".into(),
        port: 8306,
        database: String::new(),
        auth: AuthMethod::Password {
            username: "root@PuvVDiFH68oo165z".into(),
            password: "n5zmr2m2aYVILYBq4lPG3tJFUSSHUudT".into(),
        },
        ssl: false,
        connection_timeout_secs: 10,
        ssh_tunnel: None,
    }
}

#[tokio::test]
async fn test_connect_mysql() {
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    let result = driver.connect(&conn).await;
    assert!(result.is_ok(), "连接失败: {:?}", result);
    println!("✓ MySQL 连接成功");
}

#[tokio::test]
async fn test_select_1() {
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    let result = driver.execute_query("SELECT 1 AS answer", Some(10)).await.unwrap();
    assert_eq!(result.columns, vec!["answer"]);
    assert_eq!(result.rows.len(), 1);
    // `SELECT 1` is an integer; the driver correctly maps it to a JSON Number, not a string.
    assert_eq!(result.rows[0]["answer"], serde_json::Value::Number(serde_json::Number::from(1)));
    println!("✓ SELECT 1 返回: {:?}", result.rows[0]);
}

#[tokio::test]
async fn test_show_databases() {
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    let result = driver.execute_query("SHOW DATABASES", Some(100)).await.unwrap();
    assert!(result.columns.contains(&"Database".to_string()));
    assert!(result.rows.len() > 0);
    let db_names: Vec<String> = result.rows.iter().map(|r| r["Database"].as_str().unwrap_or("").to_string()).collect();
    assert!(db_names.contains(&"information_schema".to_string()) || db_names.contains(&"mysql".to_string()));
    println!("✓ SHOW DATABASES 返回 {} 个库: {:?}", result.rows.len(), db_names);
}

#[tokio::test]
async fn test_show_tables() {
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    let result = driver.execute_query("SHOW TABLES FROM mysql", Some(50)).await.unwrap();
    assert!(result.rows.len() > 0);
    println!("✓ SHOW TABLES FROM mysql 返回 {} 张表", result.rows.len());
}

#[tokio::test]
async fn test_describe_user() {
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    let result = driver.execute_query("DESCRIBE mysql.user", Some(20)).await.unwrap();
    assert!(result.rows.len() > 0);
    let cols: Vec<String> = result.rows[0].keys().cloned().collect();
    assert!(cols.contains(&"Field".to_string()));
    println!("✓ DESCRIBE mysql.user 返回 {} 列, 字段: {:?}", result.rows.len(), cols);
}

#[tokio::test]
async fn test_real_data_select() {
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    // Aggregates on a real system table — validates the LIMIT subquery wrapper +
    // row-to-map on actual data. COUNT never exposes sensitive values.
    let result = driver.execute_query("SELECT COUNT(*) AS cnt FROM mysql.user", Some(10)).await.unwrap();
    assert_eq!(result.columns, vec!["cnt"]);
    assert_eq!(result.rows.len(), 1);
    let cnt = result.rows[0]["cnt"].clone();
    println!("✓ SELECT COUNT(*) FROM mysql.user => {}", cnt);

    // A real multi-row read with the driver's LIMIT applied on top.
    let result2 = driver.execute_query("SELECT Host, User FROM mysql.user", Some(3)).await.unwrap();
    assert!(result2.rows.len() <= 3, "LIMIT 3 respected, got {}", result2.rows.len());
    // Host/User are VARBINARY in mysql.user; verify the driver surfaces them (not NULL).
    let first = result2.rows.first().unwrap();
    let host_val = first.get("Host").cloned().unwrap_or(serde_json::Value::Null);
    assert_ne!(host_val, serde_json::Value::Null, "Host should not be NULL");
    println!("✓ SELECT Host,User (LIMIT 3) 返回 {} 行, 首行 Host={:?} User={:?}",
        result2.rows.len(),
        host_val,
        first.get("User").cloned().unwrap_or(serde_json::Value::Null));
}

#[tokio::test]
async fn test_schema_objects_grouping() {
    use dbclient_lib::models::SchemaObjects;
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    // The mysql system db has real tables, views, procedures, functions, events.
    let objects: SchemaObjects = driver.get_schema_objects("mysql").await.unwrap();
    assert!(!objects.tables.is_empty(), "mysql db should have tables");
    let total = objects
        .tables
        .len()
        + objects.views.len()
        + objects.procedures.len()
        + objects.functions.len()
        + objects.events.len()
        + objects.triggers.len();
    println!(
        "✓ mysql 库对象: 表={} 视图={} 存储过程={} 函数={} 事件={} 触发器={} (共 {})",
        objects.tables.len(),
        objects.views.len(),
        objects.procedures.len(),
        objects.functions.len(),
        objects.events.len(),
        objects.triggers.len(),
        total
    );

    // The listing is lightweight: columns are loaded lazily per table.
    let user_tbl = objects
        .tables
        .iter()
        .find(|t| t.name == "user")
        .expect("mysql.user table present");
    assert!(user_tbl.columns.is_empty(), "listing should not eagerly fetch columns");

    // Table detail (lazy path) must return the columns.
    let detail = driver.get_table_detail("mysql", "user").await.unwrap();
    assert!(!detail.columns.is_empty(), "mysql.user should have columns");
    assert!(detail.columns.iter().any(|c| c.name == "User"));
    assert!(
        detail.columns.iter().any(|c| c.data_type.starts_with("char")),
        "column types should be present"
    );
    println!("✓ mysql.user 列数（懒加载详情）= {}", detail.columns.len());
}

#[tokio::test]
async fn test_object_ddl() {
    use dbclient_lib::models::ObjectKind;
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    let ddl = driver
        .get_object_ddl("mysql", "user", &ObjectKind::Table)
        .await
        .unwrap();
    assert!(!ddl.ddl.is_empty(), "DDL should not be empty");
    assert!(ddl.ddl.to_uppercase().contains("CREATE TABLE"), "DDL contains CREATE TABLE");
    println!("✓ mysql.user DDL 前 120 字符:\n{}", &ddl.ddl.chars().take(120).collect::<String>());
}

#[tokio::test]
async fn test_per_table_triggers_field() {
    // The per-table `triggers` vector must be populated without error.
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    let objects = driver.get_schema_objects("mysql").await.unwrap();
    assert!(!objects.tables.is_empty(), "mysql db has tables");
    let with_triggers = objects.tables.iter().filter(|t| !t.triggers.is_empty()).count();
    println!(
        "✓ mysql 库 {} 张表，其中有触发器的表 = {}",
        objects.tables.len(),
        with_triggers
    );
}

#[tokio::test]
async fn test_comment_retrieval_end_to_end() {
    // Find a real BASE TABLE that has a non-empty table comment, then verify
    // get_schema_objects surfaces it (plus its column comments).
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    let probe = driver
        .execute_query(
            "SELECT TABLE_SCHEMA, TABLE_NAME FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_TYPE='BASE TABLE' AND TABLE_COMMENT <> '' \
             AND TABLE_SCHEMA NOT IN ('mysql','information_schema','performance_schema','sys') \
             LIMIT 1",
            Some(1),
        )
        .await
        .unwrap();

    if probe.rows.is_empty() {
        println!("（该库没有带 COMMENT 的表，跳过）");
        return;
    }
    let schema = probe.rows[0]["TABLE_SCHEMA"].as_str().unwrap_or("").to_string();
    let table = probe.rows[0]["TABLE_NAME"].as_str().unwrap_or("").to_string();
    println!("✓ 找到带 COMMENT 的表: {}.{}", schema, table);

    let objects = driver.get_schema_objects(&schema).await.unwrap();
    let t = objects.tables.iter().find(|t| t.name == table).expect("table found");
    // The lightweight listing must already carry the table comment.
    assert!(t.comment.as_deref().map(|c| !c.is_empty()).unwrap_or(false),
        "table comment should be non-empty");
    println!("✓ 表 COMMENT = {:?}", t.comment);

    // Column-level detail (incl. comments) comes from the lazy detail call.
    let detail = driver.get_table_detail(&schema, &table).await.unwrap();
    println!("✓ 字段数 = {}, 带 COMMENT 的字段 = {}",
        detail.columns.len(),
        detail.columns.iter().filter(|c| c.comment.is_some()).count());
    assert!(!detail.columns.is_empty(), "detail should include columns");
}

#[tokio::test]
async fn test_column_comment_retrieval() {
    let mut driver = MySQLDriver::new();
    let conn = make_test_conn();
    driver.connect(&conn).await.unwrap();

    // Find a column that has a comment.
    let probe = driver
        .execute_query(
            "SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, COLUMN_COMMENT \
             FROM INFORMATION_SCHEMA.COLUMNS \
             WHERE COLUMN_COMMENT <> '' \
             AND TABLE_SCHEMA NOT IN ('mysql','information_schema','performance_schema','sys') \
             LIMIT 1",
            Some(1),
        )
        .await
        .unwrap();
    if probe.rows.is_empty() {
        println!("（该库没有带 COMMENT 的字段，跳过）");
        return;
    }
    let schema = probe.rows[0]["TABLE_SCHEMA"].as_str().unwrap_or("").to_string();
    let table = probe.rows[0]["TABLE_NAME"].as_str().unwrap_or("").to_string();
    let column = probe.rows[0]["COLUMN_NAME"].as_str().unwrap_or("").to_string();
    let expect_comment = probe.rows[0]["COLUMN_COMMENT"].as_str().unwrap_or("").to_string();
    println!("✓ 找到带 COMMENT 的字段: {}.{}.{}", schema, table, column);

    let objects = driver.get_schema_objects(&schema).await.unwrap();
    assert!(objects.tables.iter().any(|t| t.name == table), "table found in listing");
    let detail = driver.get_table_detail(&schema, &table).await.unwrap();
    let c = detail.columns.iter().find(|c| c.name == column).expect("column found");
    println!("✓ 字段类型={} 可空={} COMMENT={:?}", c.data_type, c.is_nullable, c.comment);
    assert_eq!(c.comment.as_deref(), Some(expect_comment.as_str()),
        "column comment should match INFORMATION_SCHEMA");
}
