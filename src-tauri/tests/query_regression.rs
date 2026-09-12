//! 查询执行回归测试（真实 MySQL）：
//! 1. 用户以分号结尾的 SELECT 曾因 LIMIT 子查询包装落入括号内而报 1064；
//! 2. 连接未指定默认库时，查询页/侧栏选定的"当前数据库"必须生效（覆盖 conn.database）。

use dbclient_lib::drivers::mysql::MySQLDriver;
use dbclient_lib::models::*;

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
async fn test_select_with_trailing_semicolon() {
    let mut driver = MySQLDriver::new();
    driver.connect(&make_test_conn()).await.unwrap();
    let result = driver
        .execute_query("SELECT 1 AS answer;", Some(500))
        .await
        .expect("分号结尾的 SELECT 应正常执行");
    assert_eq!(result.columns, vec!["answer"]);
    assert_eq!(result.rows.len(), 1);
}

#[tokio::test]
async fn test_query_uses_selected_database() {
    let mut conn = make_test_conn();
    conn.database = "mysql".to_string(); // 模拟"当前数据库"覆盖连接默认库
    let mut driver = MySQLDriver::new();
    driver.connect(&conn).await.unwrap();
    let result = driver
        .execute_query("SELECT DATABASE() AS db", Some(10))
        .await
        .expect("带库连接应可执行无前缀查询");
    assert_eq!(result.rows[0]["db"], serde_json::Value::String("mysql".into()));
}

#[tokio::test]
async fn test_create_procedure_with_body_semicolons() {
    // 存储过程体内含分号：不能按 ';' 拆分，且预处理协议不支持（1295），
    // 必须经 text 协议整句执行。验证 CREATE -> EXISTS -> CALL -> DROP 全链路。
    let mut conn = make_test_conn();
    conn.database = "mysql".to_string();
    let mut driver = MySQLDriver::new();
    driver.connect(&conn).await.unwrap();

    let name = "_dbclient_proc_regression";
    // 防御性清理：上次运行中断可能遗留过程
    let _ = driver.execute_query(&format!("DROP PROCEDURE IF EXISTS `mysql`.`{}`", name), None).await;
    let create = format!(
        "CREATE PROCEDURE `mysql`.`{}`(IN p_id BIGINT)\nBEGIN\n    SELECT p_id AS v;\nEND",
        name
    );
    let create_result = driver
        .execute_query(&create, None)
        .await
        .expect("CREATE PROCEDURE 应可执行");
    assert_eq!(create_result.rows_affected, Some(0));

    let call = driver
        .execute_query(&format!("CALL `mysql`.`{}`(42)", name), Some(10))
        .await
        .expect("CALL 应可执行");
    assert_eq!(call.rows[0]["v"], serde_json::Value::Number(serde_json::Number::from(42)));

    // 删除也走 text 协议（DROP PROCEDURE 同样不在预处理白名单内）
    driver
        .execute_query(&format!("DROP PROCEDURE `mysql`.`{}`", name), None)
        .await
        .expect("DROP PROCEDURE 应可执行");
}

#[tokio::test]
async fn test_routine_ddl_reconstruction_includes_params() {
    // 修改存储流程依赖 get_object_ddl：重建的 DDL 必须带参数列表与库限定
    let mut conn = make_test_conn();
    conn.database = "mysql".to_string();
    let mut driver = MySQLDriver::new();
    driver.connect(&conn).await.unwrap();

    let name = "_dbclient_proc_ddl_recon";
    // 防御性清理：上次运行断言失败可能遗留过程
    let _ = driver.execute_query(&format!("DROP PROCEDURE IF EXISTS `mysql`.`{}`", name), None).await;
    driver
        .execute_query(
            &format!(
                "CREATE PROCEDURE `mysql`.`{}`(IN p_a INT, OUT p_b INT)\nBEGIN\n    SET p_b = p_a * 2;\nEND",
                name
            ),
            None,
        )
        .await
        .expect("创建测试存储过程");

    let ddl = driver
        .get_object_ddl("mysql", name, &ObjectKind::Procedure)
        .await
        .expect("读取存储过程 DDL");
    println!("RECONSTRUCTED DDL:\n{}", ddl.ddl);

    assert!(ddl.ddl.contains("CREATE PROCEDURE `mysql`.`_dbclient_proc_ddl_recon`"));
    // MySQL INFORMATION_SCHEMA 返回小写类型名、标识符带反引号
    assert!(ddl.ddl.to_lowercase().contains("in `p_a` int"), "应包含 IN 参数: {}", ddl.ddl);
    assert!(ddl.ddl.to_lowercase().contains("out `p_b` int"), "应包含 OUT 参数: {}", ddl.ddl);

    driver
        .execute_query(&format!("DROP PROCEDURE `mysql`.`{}`", name), None)
        .await
        .expect("清理测试存储过程");
}
