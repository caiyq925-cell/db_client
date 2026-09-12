#[cfg(test)]
mod tests {
    use crate::models::*;
    use crate::storage::ConnectionStore;
    use std::fs;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    // Tests share ~/.dbclient; serialize all storage access to avoid lost updates.
    static STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock_store() -> MutexGuard<'static, ()> {
        let m = STORE_LOCK.get_or_init(|| Mutex::new(()));
        m.lock().unwrap()
    }

    fn temp_store() -> ConnectionStore {
        // 重定向数据目录到临时路径：避免污染/依赖真实的 ~/.dbclient
        let dir = std::env::temp_dir().join("dbclient-lib-tests");
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("DBCLIENT_DATA_DIR", &dir);
        ConnectionStore::new().unwrap()
    }

    fn sample_conn(name: &str, kind: DbType) -> DatabaseConnection {
        DatabaseConnection {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            kind: kind.clone(),
            group: format!("TestGroup-{}", name),
            host: "127.0.0.1".to_string(),
            port: match kind {
                DbType::MySQL => 3306,
                DbType::Mongo => 27017,
                DbType::Redis => 6379,
            },
            database: "testdb".to_string(),
            auth: AuthMethod::Password {
                username: "root".to_string(),
                password: "secret".to_string(),
            },
            ssl: false,
            connection_timeout_secs: 5,
            ssh_tunnel: None,
        }
    }

    #[test]
    fn test_add_and_list_connections() {
        let _g = lock_store();
        let store = temp_store();
        let before = store.load_connections().unwrap().len();

        let conn = sample_conn("smoke-mysql", DbType::MySQL);
        store.add_connection(&conn).unwrap();

        let all = store.load_connections().unwrap();
        assert_eq!(all.len(), before + 1);
        assert!(all.iter().any(|c| c.id == conn.id && c.name == "smoke-mysql"));

        store.delete_connection(&conn.id).unwrap();
        let after = store.load_connections().unwrap();
        assert_eq!(after.len(), before);
    }

    #[test]
    fn test_update_connection() {
        let _g = lock_store();
        let store = temp_store();
        let mut conn = sample_conn("smoke-mongo", DbType::Mongo);
        store.add_connection(&conn).unwrap();

        conn.name = "smoke-mongo-renamed".to_string();
        store.update_connection(&conn).unwrap();

        let fetched = store.get_connection(&conn.id).unwrap().unwrap();
        assert_eq!(fetched.name, "smoke-mongo-renamed");

        store.delete_connection(&conn.id).unwrap();
    }

    #[test]
    fn test_history_append_and_query() {
        let _g = lock_store();
        let store = temp_store();

        let marker = format!("SELECT {}", uuid::Uuid::new_v4());
        let entry = QueryHistoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            connection_id: format!("hist-{}", uuid::Uuid::new_v4()),
            query: marker.clone(),
            execution_time_ms: 12,
            success: true,
            error: None,
            timestamp: chrono::Utc::now(),
        };
        store.append_history(&entry).unwrap();

        let for_conn = store.get_history_for_connection(&entry.connection_id).unwrap();
        assert!(for_conn.iter().any(|h| h.query == marker));
    }

    #[test]
    fn test_connection_serialization_roundtrip() {
        // The serde wire format between Rust and the frontend must stay stable.
        let conn = sample_conn("serde-check", DbType::Redis);
        let json = serde_json::to_string(&conn).unwrap();
        println!("SERIALIZED: {}", json);
        let parsed: DatabaseConnection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, conn.id);
        assert_eq!(parsed.kind, DbType::Redis);
        assert!(matches!(parsed.auth, AuthMethod::Password { .. }));
    }

    /// End-to-end check against the dev MySQL server (task: 保存不生效 bug).
    /// Verifies update_rows uses properly qualified `db`.`table` names and
    /// applies one UPDATE per changed row. Creates and drops its own temp
    /// table, so real data is never touched.
    #[tokio::test]
    async fn test_update_rows_qualified_names() {
        use crate::drivers::mysql::MySQLDriver;
        use std::collections::HashMap;

        let conn = DatabaseConnection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "update-rows-verify".to_string(),
            kind: DbType::MySQL,
            group: "TestGroup".to_string(),
            host: "42.192.176.87".to_string(),
            port: 8306,
            database: String::new(),
            auth: AuthMethod::Password {
                username: "root@PuvVDiFH68oo165z".to_string(),
                password: "n5zmr2m2aYVILYBq4lPG3tJFUSSHUudT".to_string(),
            },
            ssl: false,
            connection_timeout_secs: 10,
            ssh_tunnel: None,
        };

        let mut d = MySQLDriver::new();
        d.connect(&conn).await.expect("connect to dev mysql");

        // 动态挑一个有写权限的业务库（临时表随连接结束自动清理）。
        // 服务器上存在名字里带单引号的怪库（如 'user_center'），跳过。
        let schemas = d.get_schemas().await.expect("list schemas");
        println!("SCHEMAS: {:?}", schemas);
        let target = schemas
            .into_iter()
            .find(|s| {
                !s.contains('\'')
                    && !matches!(
                        s.as_str(),
                        "information_schema" | "mysql" | "performance_schema" | "sys" | "__tencentdb__"
                    )
            })
            .expect("at least one writable schema");
        println!("TARGET SCHEMA: {}", target);

        // 建独立命名的验证表（连接池多连接，TEMPORARY 表会话级不可用），测完 DROP。
        // 池化连接下每个 statement 可能走不同连接，必须用真实表。
        // 主键用 BIGINT + 19 位雪花 ID，覆盖 JS 2^53 精度边界（task: 保存0行/丢精度）。
        let table = format!("_dbclient_verify_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let qualified = format!("{target}.`{table}`");
        d.execute_query(&format!("DROP TABLE IF EXISTS {qualified}"), None)
            .await
            .expect("pre-drop");
        d.execute_query(
            &format!("CREATE TABLE {qualified} (id BIGINT PRIMARY KEY, val VARCHAR(50))"),
            None,
        )
        .await
        .expect("create verify table");
        let big_id: i64 = 8_123_456_789_012_345_678; // 19 位，超过 JS Number 2^53
        d.execute_query(
            &format!("INSERT INTO {qualified} (id, val) VALUES ({big_id}, 'a'), (1, 'b')"),
            None,
        )
        .await
        .expect("insert seed rows");

        // 读回：大 ID 必须以完整字符串返回（JSON number 会丢精度）。ORDER BY id 后 id=1 在前。
        let readback = d
            .execute_query(&format!("SELECT id, val FROM {qualified} ORDER BY id"), None)
            .await
            .expect("read back");
        assert_eq!(readback.rows.len(), 2);
        assert_eq!(readback.rows[0]["id"], serde_json::Value::Number(1.into()));
        assert_eq!(
            readback.rows[1]["id"],
            serde_json::Value::String(big_id.to_string()),
            "BIGINT beyond 2^53 must arrive as an exact string, not a lossy number"
        );

        // 两行各改一列：应生成两条独立的参数化 UPDATE，大 ID WHERE 必须精确命中
        let mk = |s: &str| serde_json::Value::String(s.to_string());
        let mut where_big = HashMap::new();
        where_big.insert("id".to_string(), serde_json::Value::String(big_id.to_string()));
        let mut set_big = HashMap::new();
        set_big.insert("val".to_string(), mk("a-updated"));
        let mut where1 = HashMap::new();
        where1.insert("id".to_string(), serde_json::Value::Number(1.into()));
        let mut set1 = HashMap::new();
        set1.insert("val".to_string(), mk("b-updated"));

        let affected = d
            .update_rows(
                &target,
                &table,
                &[
                    RowUpdate { where_key: where_big, set: set_big },
                    RowUpdate { where_key: where1, set: set1 },
                ],
            )
            .await
            .expect("update_rows must succeed with qualified `db`.`table`");
        assert_eq!(affected, 2, "both UPDATE statements should match one row each");

        // 读回验证：id=1 在前（val='b-updated'），大 ID 在后（val='a-updated'）
        let result = d
            .execute_query(&format!("SELECT id, val FROM {qualified} ORDER BY id"), None)
            .await
            .expect("read back");
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[1]["val"], mk("a-updated"));
        assert_eq!(result.rows[0]["val"], mk("b-updated"));

        d.execute_query(&format!("DROP TABLE {qualified}"), None)
            .await
            .expect("drop verify table");
        d.disconnect();
    }

    #[test]
    fn test_like_pattern_escapes_wildcards() {
        use crate::drivers::mysql::MySQLDriver;
        // 搜索框语义：用户输入的 % _ \ 必须按字面匹配，不当通配符
        assert_eq!(MySQLDriver::like_pattern("order"), "%order%");
        assert_eq!(MySQLDriver::like_pattern("100%"), "%100\\%%");
        assert_eq!(MySQLDriver::like_pattern("a_b"), "%a\\_b%");
        assert_eq!(MySQLDriver::like_pattern("c\\d"), "%c\\\\d%");
    }

    /// Integration test against the dev MySQL server: cross-schema search
    /// finds a freshly created table, and the schema listing stays lightweight
    /// (columns/indexes deferred to get_table_detail). Uses its own verify
    /// table, dropped afterwards — real data is never touched.
    #[tokio::test]
    async fn test_search_tables_and_lightweight_listing() {
        use crate::drivers::mysql::MySQLDriver;

        let conn = DatabaseConnection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "search-tables-verify".to_string(),
            kind: DbType::MySQL,
            group: "TestGroup".to_string(),
            host: "42.192.176.87".to_string(),
            port: 8306,
            database: String::new(),
            auth: AuthMethod::Password {
                username: "root@PuvVDiFH68oo165z".to_string(),
                password: "n5zmr2m2aYVILYBq4lPG3tJFUSSHUudT".to_string(),
            },
            ssl: false,
            connection_timeout_secs: 10,
            ssh_tunnel: None,
        };

        let mut d = MySQLDriver::new();
        d.connect(&conn).await.expect("connect to dev mysql");

        // 1) 全实例搜索：命中 ≤ 100，绝不返回系统库
        let hits = d.search_tables("a").await.expect("search_tables");
        assert!(hits.len() <= 100);
        assert!(hits
            .iter()
            .all(|t| !matches!(t.schema.as_str(), "mysql" | "information_schema" | "performance_schema" | "sys")));

        // 2) 建独立验证表（含单列主键），搜索必须精确命中它
        let schemas = d.get_schemas().await.expect("list schemas");
        let target = schemas
            .into_iter()
            .find(|s| {
                !s.contains('\'')
                    && !matches!(
                        s.as_str(),
                        "information_schema" | "mysql" | "performance_schema" | "sys" | "__tencentdb__"
                    )
            })
            .expect("at least one usable schema");
        let marker = format!("_dbclient_srch_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let qualified = format!("{target}.`{marker}`");
        d.execute_query(&format!("DROP TABLE IF EXISTS {qualified}"), None).await.expect("pre-drop");
        d.execute_query(
            &format!("CREATE TABLE {qualified} (id BIGINT PRIMARY KEY, val VARCHAR(50))"),
            None,
        )
        .await
        .expect("create verify table");

        let hits = d.search_tables(&marker).await.expect("search marker table");
        assert_eq!(hits.len(), 1, "marker table must be found exactly once");
        assert_eq!(hits[0].schema, target);
        assert_eq!(hits[0].name, marker);

        // 3) 轻量列表：get_tables 不得扇出拉列/索引（3000 表场景的关键约束）
        let tables = d.get_tables(&target).await.expect("get_tables");
        let listed = tables.iter().find(|t| t.name == marker).expect("marker listed");
        assert!(listed.columns.is_empty(), "listing must stay lightweight");
        assert!(listed.indexes.is_empty(), "listing must stay lightweight");

        // 4) 详情按需：get_table_detail 才带列与主键信息
        let detail = d.get_table_detail(&target, &marker).await.expect("table detail");
        assert_eq!(detail.columns.len(), 2);
        assert!(detail.columns.iter().any(|c| c.name == "id" && c.is_primary_key));

        d.execute_query(&format!("DROP TABLE {qualified}"), None).await.expect("drop verify table");
        d.disconnect();
    }

    /// Keyset paging against the dev MySQL: forward/backward round-trips,
    /// has_more boundary, composite PK row-value comparison, no-PK OFFSET
    /// fallback and exact COUNT. Uses its own verify tables, dropped after.
    #[tokio::test]
    async fn test_table_page_keyset_navigation() {
        use crate::drivers::mysql::MySQLDriver;
        use crate::models::PageNav;

        let conn = DatabaseConnection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "table-page-verify".to_string(),
            kind: DbType::MySQL,
            group: "TestGroup".to_string(),
            host: "42.192.176.87".to_string(),
            port: 8306,
            database: String::new(),
            auth: AuthMethod::Password {
                username: "root@PuvVDiFH68oo165z".to_string(),
                password: "n5zmr2m2aYVILYBq4lPG3tJFUSSHUudT".to_string(),
            },
            ssl: false,
            connection_timeout_secs: 10,
            ssh_tunnel: None,
        };

        let mut d = MySQLDriver::new();
        d.connect(&conn).await.expect("connect to dev mysql");

        let schemas = d.get_schemas().await.expect("list schemas");
        let target = schemas
            .into_iter()
            .find(|s| {
                !s.contains('\'')
                    && !matches!(
                        s.as_str(),
                        "information_schema" | "mysql" | "performance_schema" | "sys" | "__tencentdb__"
                    )
            })
            .expect("at least one usable schema");

        // ---- 单列主键表：5 行，页大小 2 ----
        let t1 = format!("_dbclient_pg1_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let q1 = format!("{target}.`{t1}`");
        d.execute_query(&format!("DROP TABLE IF EXISTS {q1}"), None).await.expect("pre-drop");
        d.execute_query(
            &format!("CREATE TABLE {q1} (id BIGINT PRIMARY KEY, val VARCHAR(10))"),
            None,
        )
        .await
        .expect("create t1");
        d.execute_query(
            &format!(
                "INSERT INTO {q1} (id, val) VALUES (10,'a'),(20,'b'),(30,'c'),(40,'d'),(50,'e')"
            ),
            None,
        )
        .await
        .expect("seed t1");
        let pk1 = vec!["id".to_string()];

        // 首页：2 行，has_more
        let p1 = d.table_page(&target, &t1, &pk1, &PageNav::First, 2).await.expect("first page");
        assert_eq!(p1.rows.len(), 2);
        assert!(p1.has_more);
        assert_eq!(p1.rows[0]["id"], serde_json::json!(10));
        assert_eq!(p1.last_key, Some(vec![serde_json::json!(20)]), "last row of page 1 is id=20");

        // 第二页：id > 20 → 30,40
        let p2 = d
            .table_page(
                &target,
                &t1,
                &pk1,
                &PageNav::Next { key: p1.last_key.clone().unwrap() },
                2,
            )
            .await
            .expect("second page");
        assert_eq!(p2.rows.len(), 2);
        assert!(p2.has_more);
        assert_eq!(p2.rows[0]["id"], serde_json::json!(30));

        // 最后一页：id > 40 → 50，无更多
        let p3 = d
            .table_page(
                &target,
                &t1,
                &pk1,
                &PageNav::Next { key: p2.last_key.clone().unwrap() },
                2,
            )
            .await
            .expect("third page");
        assert_eq!(p3.rows.len(), 1);
        assert!(!p3.has_more, "row 50 must be the last page");
        assert_eq!(p3.rows[0]["id"], serde_json::json!(50));

        // 回退：pk < 50（DESC 读取后反转）→ 30,40
        let p2b = d
            .table_page(
                &target,
                &t1,
                &pk1,
                &PageNav::Prev { key: p3.first_key.clone().unwrap() },
                2,
            )
            .await
            .expect("prev page");
        assert_eq!(p2b.rows.len(), 2);
        assert_eq!(p2b.rows[0]["id"], serde_json::json!(30), "prev page must come back in ASC order");
        assert_eq!(p2b.rows[1]["id"], serde_json::json!(40));

        // 首页边界：pk < 10 → 空
        let p0 = d
            .table_page(
                &target,
                &t1,
                &pk1,
                &PageNav::Prev { key: vec![serde_json::json!(10)] },
                2,
            )
            .await
            .expect("before-first page");
        assert!(p0.rows.is_empty());
        assert!(!p0.has_more);

        // 精确计数
        assert_eq!(d.count_table_rows(&target, &t1).await.expect("count"), 5);

        // ---- 复合主键表：(a,b) 行值比较 ----
        let t2 = format!("_dbclient_pg2_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let q2 = format!("{target}.`{t2}`");
        d.execute_query(&format!("DROP TABLE IF EXISTS {q2}"), None).await.expect("pre-drop");
        d.execute_query(
            &format!("CREATE TABLE {q2} (a INT, b INT, val VARCHAR(10), PRIMARY KEY (a, b))"),
            None,
        )
        .await
        .expect("create t2");
        d.execute_query(
            &format!(
                "INSERT INTO {q2} (a, b, val) VALUES (1,1,'x'),(1,2,'y'),(1,3,'z'),(2,1,'w'),(2,2,'v')"
            ),
            None,
        )
        .await
        .expect("seed t2");
        let pk2 = vec!["a".to_string(), "b".to_string()];

        let c1 = d.table_page(&target, &t2, &pk2, &PageNav::First, 2).await.expect("c first");
        assert_eq!(c1.rows.len(), 2);
        assert_eq!(c1.last_key, Some(vec![serde_json::json!(1), serde_json::json!(2)]));

        let c2 = d
            .table_page(
                &target,
                &t2,
                &pk2,
                &PageNav::Next { key: c1.last_key.clone().unwrap() },
                2,
            )
            .await
            .expect("c second");
        assert_eq!(c2.rows[0]["val"], serde_json::json!("z"), "row (1,3) must follow (1,2) by row-value comparison");
        assert_eq!(c2.rows[1]["val"], serde_json::json!("w"), "row (2,1) must follow (1,3)");

        // ---- 无主键表：OFFSET 退化 ----
        let t3 = format!("_dbclient_pg3_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let q3 = format!("{target}.`{t3}`");
        d.execute_query(&format!("DROP TABLE IF EXISTS {q3}"), None).await.expect("pre-drop");
        d.execute_query(
            &format!("CREATE TABLE {q3} (val VARCHAR(10))"),
            None,
        )
        .await
        .expect("create t3");
        d.execute_query(
            &format!("INSERT INTO {q3} (val) VALUES ('a'),('b'),('c')"),
            None,
        )
        .await
        .expect("seed t3");

        let o2 = d
            .table_page(
                &target,
                &t3,
                &[],
                &PageNav::Offset { offset: 2 },
                2,
            )
            .await
            .expect("offset page");
        assert_eq!(o2.rows.len(), 1);
        assert!(!o2.has_more);
        assert_eq!(o2.rows[0]["val"], serde_json::json!("c"));
        assert!(o2.first_key.is_none(), "no-PK pages carry no keyset boundaries");

        for q in [&q1, &q2, &q3] {
            d.execute_query(&format!("DROP TABLE {q}"), None).await.expect("drop verify table");
        }
        d.disconnect();
    }

    /// Feature C3: streaming export — CSV escaping, max_rows truncation,
    /// pre-set cancel (header-only file), and JSONL round-trip.
    #[tokio::test]
    async fn test_export_stream_csv_jsonl_truncate_cancel() {
        use crate::drivers::mysql::MySQLDriver;
        use crate::models::ExportFormat;
        use std::sync::atomic::{AtomicBool, Ordering};

        let conn = DatabaseConnection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "export-verify".to_string(),
            kind: DbType::MySQL,
            group: "TestGroup".to_string(),
            host: "42.192.176.87".to_string(),
            port: 8306,
            database: String::new(),
            auth: AuthMethod::Password {
                username: "root@PuvVDiFH68oo165z".to_string(),
                password: "n5zmr2m2aYVILYBq4lPG3tJFUSSHUudT".to_string(),
            },
            ssl: false,
            connection_timeout_secs: 10,
            ssh_tunnel: None,
        };

        let mut d = MySQLDriver::new();
        d.connect(&conn).await.expect("connect to dev mysql");

        let schemas = d.get_schemas().await.expect("list schemas");
        let target = schemas
            .into_iter()
            .find(|s| {
                !s.contains('\'')
                    && !matches!(
                        s.as_str(),
                        "information_schema" | "mysql" | "performance_schema" | "sys" | "__tencentdb__"
                    )
            })
            .expect("at least one usable schema");

        let t = format!("_dbclient_exp_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let q = format!("{target}.`{t}`");
        d.execute_query(&format!("DROP TABLE IF EXISTS {q}"), None).await.expect("pre-drop");
        d.execute_query(
            &format!("CREATE TABLE {q} (id INT PRIMARY KEY, val VARCHAR(30), note VARCHAR(30))"),
            None,
        )
        .await
        .expect("create export table");
        d.execute_query(
            &format!(
                "INSERT INTO {q} (id, val, note) VALUES \
                 (1, 'plain', 'n1'),\
                 (2, 'a,b', 'n2'),\
                 (3, 'say \"hi\"', 'n3'),\
                 (4, 'two\nlines', 'n4'),\
                 (5, NULL, 'n5'),\
                 (6, 'tail', NULL),\
                 (7, 'end', 'n7')"
            ),
            None,
        )
        .await
        .expect("seed export table");

        // Table-export SQL: PK-ordered, columns from INFORMATION_SCHEMA.
        let cols = d.get_columns_for_table(&target, &t).await.expect("columns");
        let fallback: Vec<String> = cols.iter().map(|c| c.name.clone()).collect();
        let sql = format!(
            "SELECT * FROM {}.{} ORDER BY {}",
            crate::drivers::mysql::object_identifier(&target),
            crate::drivers::mysql::object_identifier(&t),
            crate::drivers::mysql::object_identifier("id")
        );

        let tmp = std::env::temp_dir();
        let out_path = |tag: &str| {
            let mut p = tmp.join(format!("dbclient-exp-test-{tag}-{}.csv", uuid::Uuid::new_v4().simple()));
            p.set_extension("out"); // avoid accidental .csv associations
            p
        };
        let noop = |_: u64| {};

        // ---- 1) CSV full export: 7 data rows + header, RFC 4180 escaping ----
        let p1 = out_path("full");
        let s1 = d
            .export_stream(&sql, ExportFormat::Csv, p1.to_str().unwrap(), 50_000_000, Some(&fallback), &AtomicBool::new(false), noop)
            .await
            .expect("csv export");
        assert_eq!(s1.rows, 7);
        assert!(!s1.truncated && !s1.cancelled);
        let body = std::fs::read_to_string(&p1).unwrap();
        // Field "two\nlines" contains a real newline inside quotes, so the
        // file is compared as a whole instead of line-by-line.
        assert_eq!(
            body,
            concat!(
                "id,val,note\n",
                "1,plain,n1\n",
                "2,\"a,b\",n2\n",
                "3,\"say \"\"hi\"\"\",n3\n",
                "4,\"two\nlines\",n4\n",
                "5,,n5\n",
                "6,tail,\n",
                "7,end,n7\n",
            )
        );
        std::fs::remove_file(&p1).unwrap();

        // ---- 2) max_rows truncation: stops at exactly 3 rows ----
        let p2 = out_path("trunc");
        let s2 = d
            .export_stream(&sql, ExportFormat::Csv, p2.to_str().unwrap(), 3, Some(&fallback), &AtomicBool::new(false), noop)
            .await
            .expect("truncated export");
        assert_eq!(s2.rows, 3);
        assert!(s2.truncated);
        let body2 = std::fs::read_to_string(&p2).unwrap();
        assert_eq!(body2.lines().count(), 4, "header + 3 rows");
        std::fs::remove_file(&p2).unwrap();

        // ---- 3) pre-set cancel: header-only file, cancelled summary ----
        let p3 = out_path("cancel");
        let cancel3 = AtomicBool::new(false);
        cancel3.store(true, Ordering::Relaxed);
        let s3 = d
            .export_stream(&sql, ExportFormat::Csv, p3.to_str().unwrap(), 50_000_000, Some(&fallback), &cancel3, noop)
            .await
            .expect("cancelled export");
        assert_eq!(s3.rows, 0);
        assert!(s3.cancelled);
        let body3 = std::fs::read_to_string(&p3).unwrap();
        assert_eq!(body3, "id,val,note\n", "cancel before first row keeps only the header");
        std::fs::remove_file(&p3).unwrap();

        // ---- 4) JSONL: one parseable object per row, column order kept ----
        let mut p4 = out_path("jsonl");
        p4.set_extension("jsonl");
        let s4 = d
            .export_stream(&sql, ExportFormat::JsonLines, p4.to_str().unwrap(), 50_000_000, None, &AtomicBool::new(false), noop)
            .await
            .expect("jsonl export");
        assert_eq!(s4.rows, 7);
        let body4 = std::fs::read_to_string(&p4).unwrap();
        let parsed: Vec<serde_json::Value> = body4
            .lines()
            .map(|l| serde_json::from_str(l).expect("each JSONL line must parse"))
            .collect();
        assert_eq!(parsed.len(), 7);
        assert_eq!(parsed[0], serde_json::json!({"id":1,"val":"plain","note":"n1"}));
        assert_eq!(parsed[4]["val"], serde_json::Value::Null, "NULL stays null in JSONL");
        std::fs::remove_file(&p4).unwrap();

        d.execute_query(&format!("DROP TABLE {q}"), None).await.expect("drop export table");
        d.disconnect();
    }
}
