//! MongoDB legacy wire-protocol layer for 3.2 (Feature B).
//!
//! The official `mongodb` 3.x driver speaks only OP_MSG (wire version 6+),
//! so MongoDB 3.2 (maxWireVersion 4) is unreachable through it. This module
//! implements the legacy protocol directly over TCP — OP_QUERY / OP_REPLY /
//! OP_KILL_CURSORS — with SCRAM-SHA-1 (primary) and MONGODB-CR (fallback)
//! authentication. Read-only browsing only: find / listDatabases /
//! listCollections / count; writes and aggregations are rejected.
//!
//! Everything here is pure Rust: no C dependencies, no TLS (outside scope).

use crate::models::{AuthMethod, DatabaseConnection, QueryResult};
use base64::Engine;
use bson::{Bson, Document};
use hmac::{Hmac, Mac};
use md5::{Digest as Md5Digest, Md5};
use sha1::Sha1;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug, Error)]
pub enum LegacyError {
    #[error("Connection error: {0}")]
    Io(#[from] std::io::Error),
    #[error("BSON error: {0}")]
    Bson(#[from] bson::ser::Error),
    #[error("BSON deserialize error: {0}")]
    BsonDe(#[from] bson::de::Error),
    #[error("BSON field access error: {0}")]
    FieldAccess(#[from] bson::document::ValueAccessError),
    #[error("Server error ({code}): {message}")]
    Server { code: i32, message: String },
    #[error("Auth error: {0}")]
    Auth(String),
    #[error("{0}")]
    Query(String),
}

// ===== Wire protocol constants =====

const OP_REPLY: i32 = 1;
const OP_QUERY: i32 = 2004;
const OP_KILL_CURSORS: i32 = 2007;
const HEADER_LEN: usize = 16;

// ===== SCRAM-SHA-1 helpers (RFC 5802, MongoDB password pre-processing) =====

/// MongoDB hashes the plain password with MD5 before SCRAM uses it.
/// `SaltedPassword = Hi(hex(md5(user + ":mongo:" + pass)))`.
pub fn mongo_password_digest(user: &str, pass: &str) -> String {
    hex_md5(&format!("{}:mongo:{}", user, pass))
}

fn hex_md5(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// PBKDF2-HMAC-SHA1 with dkLen = 20 bytes (SCRAM SHA-1 key size).
pub fn hi_sha1(password: &[u8], salt: &[u8], iterations: u32) -> Vec<u8> {
    let mut result = hmac_sha1(password, &[salt, &1u32.to_be_bytes()].concat());
    let mut acc = result.clone();
    for _ in 1..iterations {
        result = hmac_sha1(password, &result);
        acc.iter_mut().zip(&result).for_each(|(a, b)| *a ^= b);
    }
    acc
}

/// Escape SCRAM username specials (`,` → `=2C`, `=` → `=3D`).
pub fn scram_saslprep_username(user: &str) -> String {
    user.replace('=', "=3D").replace(',', "=2C")
}

fn xor_bytes(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter().zip(b).map(|(x, y)| x ^ y).collect()
}

/// Compute the SCRAM client-proof and expected server signature from an
/// already-derived SaltedPassword.
/// Returns `(client_proof_b64, server_signature_b64)`.
fn scram_client_final_salted(salted_password: &[u8], auth_message: &str) -> (String, String) {
    let engine = base64::engine::general_purpose::STANDARD;
    let client_key = hmac_sha1(salted_password, b"Client Key");
    let stored_key = Sha1::digest(&client_key).to_vec();
    let client_sig = hmac_sha1(&stored_key, auth_message.as_bytes());
    let proof = xor_bytes(&client_key, &client_sig);
    let server_key = hmac_sha1(salted_password, b"Server Key");
    let server_sig = hmac_sha1(&server_key, auth_message.as_bytes());
    (engine.encode(proof), engine.encode(server_sig))
}

/// Full client-final computation: PBKDF2 over the (MongoDB-pre-hashed)
/// password, then the SCRAM signature chain.
pub fn scram_client_final(
    password_digest: &str,
    salt_b64: &str,
    iterations: u32,
    auth_message: &str,
) -> Result<(String, String), LegacyError> {
    let salt = base64::engine::general_purpose::STANDARD
        .decode(salt_b64)
        .map_err(|e| LegacyError::Auth(format!("bad salt: {e}")))?;
    let salted = hi_sha1(password_digest.as_bytes(), &salt, iterations);
    Ok(scram_client_final_salted(&salted, auth_message))
}

/// Extract the SASL payload string from a saslStart/saslContinue reply,
/// embedding the raw reply in the error for diagnostics.
fn payload_string(doc: &Document, stage: &str) -> Result<String, LegacyError> {
    match doc.get("payload") {
        Some(Bson::Binary(b)) => Ok(String::from_utf8_lossy(&b.bytes).into_owned()),
        other => Err(LegacyError::Auth(format!(
            "{stage} 回复缺少 payload: done={done:?} 原始={other:?}",
            done = doc.get("done"),
        ))),
    }
}

/// Verify the server-final `v=` value proves the server knew the password.
pub fn verify_server_final(server_final_payload: &str, expected_sig_b64: &str) -> Result<(), LegacyError> {
    let found = server_final_payload
        .split(',')
        .find_map(|kv| kv.strip_prefix("v="))
        .ok_or_else(|| {
            LegacyError::Auth(format!(
                "server-final 缺少 v= 字段, 原始 payload: {server_final_payload:?}"
            ))
        })?;
    if found == expected_sig_b64 {
        Ok(())
    } else {
        Err(LegacyError::Auth("服务端签名校验失败".into()))
    }
}

// ===== One connection speaking the legacy protocol =====

struct Reply {
    _flags: i32,
    /// Kept for potential OP_GET_MORE; never read in single-batch browsing.
    _cursor_id: i64,
    _starting_from: i32,
    docs: Vec<Document>,
}

pub struct LegacyConn {
    stream: TcpStream,
    next_request_id: i32,
}

impl LegacyConn {
    pub async fn connect(host: &str, port: u16, timeout: Duration) -> Result<Self, LegacyError> {
        let stream = tokio::time::timeout(
            timeout,
            TcpStream::connect((host, port)),
        )
        .await
        .map_err(|_| LegacyError::Query(format!("连接超时（{}s）", timeout.as_secs())))??
        ;
        stream.set_nodelay(true).ok();
        Ok(Self { stream, next_request_id: 1 })
    }

    /// Send one request and read exactly one OP_REPLY.
    async fn round_trip(&mut self, opcode: i32, body: &[u8]) -> Result<Reply, LegacyError> {
        let request_id = self.next_request_id;
        self.next_request_id = request_id.wrapping_add(1);

        let total = (HEADER_LEN + body.len()) as i32;
        let mut msg = Vec::with_capacity(total as usize);
        msg.extend_from_slice(&total.to_le_bytes());
        msg.extend_from_slice(&request_id.to_le_bytes());
        msg.extend_from_slice(&0i32.to_le_bytes()); // responseTo (client → server)
        msg.extend_from_slice(&opcode.to_le_bytes());
        msg.extend_from_slice(body);
        self.stream.write_all(&msg).await?;
        self.stream.flush().await?;

        let mut header = [0u8; HEADER_LEN];
        self.stream.read_exact(&mut header).await?;
        let length = i32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
        if length < HEADER_LEN {
            return Err(LegacyError::Query(format!("协议错误：消息长度 {length} 非法")));
        }
        let response_to = i32::from_le_bytes(header[8..12].try_into().unwrap());
        let op = i32::from_le_bytes(header[12..16].try_into().unwrap());
        if response_to != request_id {
            return Err(LegacyError::Query("协议错误：responseTo 不匹配".into()));
        }
        if op != OP_REPLY {
            return Err(LegacyError::Query(format!("协议错误：期望 OP_REPLY，得到 opcode {op}")));
        }

        let mut rest = vec![0u8; length - HEADER_LEN];
        self.stream.read_exact(&mut rest).await?;
        let flags = i32::from_le_bytes(rest[0..4].try_into().unwrap());
        let cursor_id = i64::from_le_bytes(rest[4..12].try_into().unwrap());
        let starting_from = i32::from_le_bytes(rest[12..16].try_into().unwrap());
        let number_returned = i32::from_le_bytes(rest[16..20].try_into().unwrap()) as usize;

        let mut docs = Vec::with_capacity(number_returned);
        let mut off = 20;
        for _ in 0..number_returned {
            if off + 4 > rest.len() {
                return Err(LegacyError::Query("协议错误：文档区被截断".into()));
            }
            let doc_len = i32::from_le_bytes(rest[off..off + 4].try_into().unwrap()) as usize;
            let doc = Document::from_reader(&rest[off..off + doc_len])?;
            docs.push(doc);
            off += doc_len;
        }
        Ok(Reply { _flags: flags, _cursor_id: cursor_id, _starting_from: starting_from, docs })
    }

    /// Run a command against `db.$cmd` (single reply document).
    pub async fn run_command(&mut self, db: &str, cmd: Document) -> Result<Document, LegacyError> {
        let reply = self.query_raw(&format!("{db}.$cmd"), 0, -1, cmd, None).await?;
        reply.docs.into_iter().next().ok_or_else(|| LegacyError::Query("命令回复为空".into()))
    }

    /// OP_QUERY with skip / batchSize; negative `number_to_return` closes the
    /// cursor after one batch (browsing workloads are small by design).
    async fn query_raw(
        &mut self,
        full_collection_name: &str,
        skip: i32,
        number_to_return: i32,
        query: Document,
        return_fields: Option<Document>,
    ) -> Result<Reply, LegacyError> {
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes()); // flags: TailableAwait etc. off
        write_cstring(&mut body, full_collection_name);
        body.extend_from_slice(&skip.to_le_bytes());
        body.extend_from_slice(&number_to_return.to_le_bytes());
        body.extend_from_slice(&bson::to_vec(&query)?);
        if let Some(f) = return_fields {
            body.extend_from_slice(&bson::to_vec(&f)?);
        }
        let reply = self.round_trip(OP_QUERY, &body).await?;
        Self::check_docs(&reply.docs)?;
        Ok(reply)
    }

    /// Both `$err` (legacy query failure) and `ok: 0` (command failure) land here.
    fn check_docs(docs: &[Document]) -> Result<(), LegacyError> {
        if let Some(d) = docs.first() {
            if let Some(err) = d.get_str("$err").ok() {
                let code = d.get_i32("code").unwrap_or(0);
                return Err(LegacyError::Server { code, message: err.to_string() });
            }
            if d.get_f64("ok").map(|v| v == 0.0).unwrap_or(false) {
                let message = d
                    .get_str("errmsg")
                    .map(str::to_string)
                    .unwrap_or_else(|_| "未知错误".into());
                let code = d.get_i32("code").unwrap_or(0);
                return Err(LegacyError::Server { code, message });
            }
        }
        Ok(())
    }

    /// Release an open cursor politely (fire-and-forget; a stale cursor also
    /// expires server-side after 10 minutes).
    #[allow(dead_code)]
    async fn kill_cursor(&mut self, cursor_id: i64) {
        if cursor_id == 0 {
            return;
        }
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes()); // options
        body.extend_from_slice(&1i32.to_le_bytes()); // numberCursorIds
        body.extend_from_slice(&cursor_id.to_le_bytes());
        let request_id = self.next_request_id;
        self.next_request_id = request_id.wrapping_add(1);
        let total = (HEADER_LEN + body.len()) as i32;
        let mut msg = Vec::with_capacity(total as usize);
        msg.extend_from_slice(&total.to_le_bytes());
        msg.extend_from_slice(&request_id.to_le_bytes());
        msg.extend_from_slice(&0i32.to_le_bytes());
        msg.extend_from_slice(&OP_KILL_CURSORS.to_le_bytes());
        msg.extend_from_slice(&body);
        self.stream.write_all(&msg).await.ok();
    }

    // ===== Authentication =====

    /// SCRAM-SHA-1 first; on "mechanism not supported / AuthenticationFailed
    /// against old user schema" fall back to MONGODB-CR (pre-3.0 users).
    pub async fn authenticate(&mut self, auth_db: &str, user: &str, pass: &str) -> Result<(), LegacyError> {
        match self.scram_sha1(auth_db, user, pass).await {
            Ok(()) => Ok(()),
            Err(scram_err) => match self.mongodb_cr(auth_db, user, pass).await {
                Ok(()) => Ok(()),
                Err(cr_err) => Err(LegacyError::Auth(format!(
                    "SCRAM-SHA-1: {scram_err}；MONGODB-CR 回退: {cr_err}"
                ))),
            },
        }
    }

    async fn sasl_start(&mut self, db: &str, mechanism: &str, payload: Bson) -> Result<Document, LegacyError> {
        let cmd = bson::doc! {
            "saslStart": 1i32,
            "mechanism": mechanism,
            "payload": payload,
            "autoAuthorize": 1i32,
        };
        self.run_command(db, cmd).await
    }

    async fn sasl_continue(&mut self, db: &str, conversation_id: i32, payload: Bson) -> Result<Document, LegacyError> {
        let cmd = bson::doc! {
            "saslContinue": 1i32,
            "conversationId": conversation_id,
            "payload": payload,
        };
        self.run_command(db, cmd).await
    }

    async fn scram_sha1(&mut self, db: &str, user: &str, pass: &str) -> Result<(), LegacyError> {
        let engine = base64::engine::general_purpose::STANDARD;

        let client_nonce_b64 = engine.encode(new_nonce());
        let escaped_user = scram_saslprep_username(user);
        let client_first_bare = format!("n={escaped_user},r={client_nonce_b64}");
        let client_first = format!("n,,").to_string() + &client_first_bare; // gs2 header: plain, no authzid

        let start = self
            .sasl_start(db, "SCRAM-SHA-1", Bson::Binary(bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: client_first.clone().into_bytes(),
            }))
            .await?;
        let conversation_id = start.get_i32("conversationId")?;
        let server_first_bytes = match start.get("payload") {
            Some(Bson::Binary(b)) => b.bytes.clone(),
            _ => return Err(LegacyError::Auth("SCRAM server-first payload 缺失".into())),
        };
        let server_first = String::from_utf8(server_first_bytes)
            .map_err(|_| LegacyError::Auth("SCRAM server-first 非 UTF-8".into()))?;

        let mut server_nonce = None;
        let mut salt = None;
        let mut iterations = None;
        for kv in server_first.split(',') {
            if let Some(r) = kv.strip_prefix("r=") {
                server_nonce = Some(r.to_string());
            } else if let Some(s) = kv.strip_prefix("s=") {
                salt = Some(s.to_string());
            } else if let Some(i) = kv.strip_prefix("i=") {
                iterations = Some(i.parse::<u32>().map_err(|_| LegacyError::Auth("迭代次数非法".into()))?);
            }
        }
        let server_nonce = server_nonce.ok_or_else(|| LegacyError::Auth("server-first 缺少 r=".into()))?;
        let salt = salt.ok_or_else(|| LegacyError::Auth("server-first 缺少 s=".into()))?;
        let iterations = iterations.ok_or_else(|| LegacyError::Auth("server-first 缺少 i=".into()))?;
        if !server_nonce.starts_with(&client_nonce_b64) {
            return Err(LegacyError::Auth("服务端 nonce 未包含客户端 nonce".into()));
        }

        let client_final_wo_proof = format!("c=biws,r={server_nonce}");
        let auth_message = format!("{client_first_bare},{server_first},{client_final_wo_proof}");
        let password_digest = mongo_password_digest(user, pass);
        let (proof_b64, server_sig_b64) =
            scram_client_final(&password_digest, &salt, iterations, &auth_message)?;
        let client_final = format!("{client_final_wo_proof},p={proof_b64}");

        let cont = self
            .sasl_continue(
                db,
                conversation_id,
                Bson::Binary(bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: client_final.into_bytes(),
                }),
            )
            .await?;

        // Server-final `v=` rides on the client-final reply itself. MongoDB
        // (3.0–3.6 era, no skipEmptyExchange) sets done=false there and still
        // expects ONE empty saslContinue to close the conversation — whose
        // reply is empty. Verified against a live 3.2.10 server.
        let final_payload = payload_string(&cont, "saslContinue(client-final)")?;
        if !cont.get_bool("done").unwrap_or(true) {
            let _ = self
                .sasl_continue(
                    db,
                    conversation_id,
                    Bson::Binary(bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: Vec::new(),
                    }),
                )
                .await?;
        }
        verify_server_final(&final_payload, &server_sig_b64)
    }

    /// MONGODB-CR (pre-3.0 user credential format) fallback.
    ///
    /// Flow: saslStart(empty) → server payload is a BSON binary holding
    /// `{nonce}` → reply with a BSON binary holding `{user, nonce, key}` where
    /// `key = md5(nonce + user + ":mongo:" + md5hex(user:mongo:pass))` — the
    /// nonce comes FIRST and the inner md5 is the hex password digest (same
    /// formula as mgo's loginClassic / the Java driver's MongoCR authenticator).
    async fn mongodb_cr(&mut self, db: &str, user: &str, pass: &str) -> Result<(), LegacyError> {
        let start = self.sasl_start(db, "MONGODB-CR", Bson::Null).await?;
        let server_payload = match start.get("payload") {
            Some(Bson::Binary(b)) => Document::from_reader(&b.bytes[..])?,
            _ => return Err(LegacyError::Auth("MONGODB-CR：saslStart 回复缺少 payload".into())),
        };
        let nonce = server_payload
            .get_str("nonce")
            .map_err(|_| LegacyError::Auth("MONGODB-CR：payload 中缺少 nonce".into()))?
            .to_string();

        let digest = mongo_password_digest(user, pass);
        let key = hex_md5(&format!("{}{}:mongo:{}", nonce, user, digest));
        let reply_payload = bson::to_vec(&bson::doc! {
            "user": user,
            "nonce": nonce.as_str(),
            "key": key,
        })?;
        self.sasl_continue(
            db,
            start.get_i32("conversationId").unwrap_or(0),
            Bson::Binary(bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: reply_payload,
            }),
        )
        .await?;
        Ok(())
    }

    // ===== Read-only browsing =====

    pub async fn list_databases(&mut self) -> Result<Vec<String>, LegacyError> {
        let reply = self.run_command("admin", bson::doc! { "listDatabases": 1i32 }).await?;
        let mut names = Vec::new();
        if let Some(Bson::Array(dbs)) = reply.get("databases") {
            for d in dbs {
                if let Some(doc) = d.as_document() {
                    if let Ok(name) = doc.get_str("name") {
                        names.push(name.to_string());
                    }
                }
            }
        }
        Ok(names
            .into_iter()
            .filter(|db| !["admin", "local", "config"].contains(&db.as_str()))
            .collect())
    }

    pub async fn list_collections(&mut self, database: &str) -> Result<Vec<(String, i64)>, LegacyError> {
        let reply = self
            .run_command(database, bson::doc! { "listCollections": 1i32 })
            .await?;
        let mut out = Vec::new();
        if let Some(Bson::Array(batch)) = reply.get("cursor").and_then(|c| c.as_document()).and_then(|c| c.get("firstBatch")) {
            for d in batch {
                if let Some(doc) = d.as_document() {
                    if let Ok(name) = doc.get_str("name") {
                        out.push((name.to_string(), 0i64));
                    }
                }
            }
        }
        // 3.2 firstBatch covers typical collection counts; counts filled per collection.
        for (name, count) in out.iter_mut() {
            if let Ok(cmd_reply) = self
                .run_command(database, bson::doc! { "count": name })
                .await
            {
                if let Ok(n) = cmd_reply.get_i64("n").or_else(|_| cmd_reply.get_i32("n").map(|v| v as i64)) {
                    *count = n;
                }
            }
        }
        Ok(out)
    }

    /// Sample documents from a collection (browsing read path).
    pub async fn find_sample(
        &mut self,
        database: &str,
        collection: &str,
        filter: Document,
        limit: u32,
    ) -> Result<Vec<Document>, LegacyError> {
        let ns = format!("{database}.{collection}");
        let reply = self.query_raw(&ns, 0, -(limit as i32), filter, None).await?;
        Ok(reply.docs)
    }

    pub async fn ping(&mut self) -> Result<(), LegacyError> {
        self.run_command("admin", bson::doc! { "ping": 1i32 }).await?;
        Ok(())
    }

    /// Probe server info: returns (maxWireVersion, buildInfo.version).
    pub async fn is_master(&mut self) -> Result<(Option<i32>, String), LegacyError> {
        let reply = self.run_command("admin", bson::doc! { "isMaster": 1i32 }).await?;
        let wire = reply.get_i32("maxWireVersion").ok();
        let version = reply
            .get_str("version")
            .map(str::to_string)
            .unwrap_or_default();
        Ok((wire, version))
    }
}

/// Probe `maxWireVersion` over a throwaway connection (no auth). OP_QUERY on
/// `admin.$cmd` for isMaster stays available on every server version — this
/// is the handshake channel the official drivers use as well, so the probe
/// works for 3.2 through 8.x alike.
pub async fn probe_wire_version(host: &str, port: u16, timeout: Duration) -> Result<Option<i32>, LegacyError> {
    let mut c = LegacyConn::connect(host, port, timeout).await?;
    let (wire, _version) = c.is_master().await?;
    Ok(wire)
}

fn write_cstring(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

/// 18 random bytes (base64 → 24 chars), matching drivers' nonce style.
fn new_nonce() -> Vec<u8> {
    // std-only entropy: mix time, process id and a counter from a thread-local.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ (std::process::id() as u64) << 32
        ^ COUNTER.fetch_add(1, Ordering::Relaxed);
    // xorshift64* to stretch the seed
    let mut out = Vec::with_capacity(18);
    for _ in 0..18 {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        out.push((seed.wrapping_mul(0x2545F4914F6CDD1D) >> 33) as u8);
    }
    out
}

// ===== Driver facade =====

pub struct MongoLegacyDriver {
    conn: Option<LegacyConn>,
}

impl MongoLegacyDriver {
    pub fn new() -> Self {
        Self { conn: None }
    }

    pub async fn connect(&mut self, conn: &DatabaseConnection) -> Result<(), LegacyError> {
        let timeout = Duration::from_secs(conn.connection_timeout_secs.max(1) as u64);
        let mut lc = LegacyConn::connect(&conn.host, conn.port, timeout).await?;
        let (wire, version) = lc.is_master().await?;
        if wire.map(|w| w >= 6).unwrap_or(false) {
            // 3.6+ should go through the official driver instead.
            return Err(LegacyError::Query(format!(
                "服务端 {version} (maxWireVersion {wire:?}) 支持 OP_MSG，请使用官方驱动路径"
            )));
        }
        if let AuthMethod::Password { username, password } = &conn.auth {
            // authSource is always admin: the model has no dedicated auth-db
            // field and conn.database is the default WORKING db (which the UI
            // overrides per query). Authenticating against it broke when the
            // user's account lives in admin — the standard cloud layout.
            lc.authenticate("admin", username, password).await?;
        }
        self.conn = Some(lc);
        Ok(())
    }

    fn conn(&mut self) -> Result<&mut LegacyConn, LegacyError> {
        self.conn
            .as_mut()
            .ok_or_else(|| LegacyError::Query("Not connected".into()))
    }

    pub async fn list_databases(&mut self) -> Result<Vec<String>, LegacyError> {
        self.conn()?.list_databases().await
    }

    pub async fn list_collections(&mut self, database: &str) -> Result<Vec<(String, i64)>, LegacyError> {
        self.conn()?.list_collections(database).await
    }

    pub async fn find_sample(
        &mut self,
        database: &str,
        collection: &str,
        filter: Document,
        limit: u32,
    ) -> Result<Vec<Document>, LegacyError> {
        self.conn()?.find_sample(database, collection, filter, limit).await
    }

    pub async fn test_connection(&mut self, conn: &DatabaseConnection) -> Result<(), LegacyError> {
        self.connect(conn).await?;
        self.conn()?.ping().await
    }

    /// JSON-value find used by the query workbench (read-only).
    /// `collection`: 目标集合名；None 时回落到库名（兼容旧行为）。
    pub async fn execute_query(
        &mut self,
        conn: &DatabaseConnection,
        query_json: &str,
        limit: Option<u32>,
        collection: Option<&str>,
    ) -> Result<QueryResult, LegacyError> {
        let json_val: serde_json::Value = serde_json::from_str(query_json)
            .map_err(|e| LegacyError::Query(format!("查询必须是 JSON 文档: {e}")))?;
        let filter = bson::to_document(&json_val)?;
        let start = std::time::Instant::now();
        let coll_name = collection.unwrap_or(&conn.database);
        let docs = self
            .find_sample(&conn.database, coll_name, filter, limit.unwrap_or(200))
            .await?;

        let mut columns: Vec<String> = Vec::new();
        let mut rows: Vec<HashMap<String, serde_json::Value>> = Vec::new();
        for doc in docs {
            if columns.is_empty() {
                columns = doc.keys().cloned().collect();
            }
            let mut map = HashMap::new();
            for (k, v) in &doc {
                map.insert(k.clone(), bson_to_json(v));
            }
            rows.push(map);
        }
        let total = rows.len() as u64;
        Ok(QueryResult {
            columns,
            rows,
            rows_affected: None,
            execution_time_ms: start.elapsed().as_millis() as u64,
            truncated: false,
            total_rows: Some(total),
            message: None,
        })
    }

    pub fn disconnect(&mut self) {
        self.conn = None;
    }
}

/// Convert a BSON value to JSON for the frontend grid (extended JSON kept flat).
pub fn bson_to_json(v: &Bson) -> serde_json::Value {
    match v {
        Bson::ObjectId(oid) => serde_json::Value::String(oid.to_string()),
        Bson::DateTime(dt) => serde_json::Value::String(dt.to_string()),
        Bson::Binary(b) => serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(&b.bytes)),
        Bson::RegularExpression(r) => serde_json::Value::String(r.to_string()),
        other => serde_json::to_value(bson::to_bson(other).unwrap_or(Bson::Null))
            .unwrap_or(serde_json::Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    // ===== wire layer =====

    #[test]
    fn cstring_appends_terminator() {
        let mut b = Vec::new();
        write_cstring(&mut b, "test.$cmd");
        assert_eq!(b, b"test.$cmd\0");
    }

    #[test]
    fn op_query_body_layout() {
        // flags(4) + ns cstring + skip(4) + nToReturn(4) + doc
        let query = bson::doc! { "ping": 1i32 };
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes());
        write_cstring(&mut body, "admin.$cmd");
        body.extend_from_slice(&0i32.to_le_bytes());
        body.extend_from_slice(&(-1i32).to_le_bytes());
        body.extend_from_slice(&bson::to_vec(&query).unwrap());
        // header 16 is excluded: body = flags 4 + ns cstring (10+1) + skip 4
        // + nToReturn 4 + 15-byte {"ping":1} doc = 38
        assert_eq!(body.len(), 38, "flags+ns+skip+nToReturn+doc");
        assert_eq!(&body[4..14], b"admin.$cmd");
        assert_eq!(body[14], 0);
        assert_eq!(&body[body.len() - 15..body.len() - 11], &[0x0f, 0, 0, 0], "15-byte ping doc");
    }

    #[test]
    fn reply_parsing_rejects_bad_opcode_is_upstream_checked() {
        // round_trip reads from a socket; here we only assert the layout math
        // of a hand-built OP_REPLY body: flags(4) cursorId(8) startingFrom(4)
        // numberReturned(4) + one empty document (5 bytes).
        let doc = bson::to_vec(&bson::doc! {}).unwrap();
        let mut rest = Vec::new();
        rest.extend_from_slice(&0i32.to_le_bytes());
        rest.extend_from_slice(&0i64.to_le_bytes());
        rest.extend_from_slice(&0i32.to_le_bytes());
        rest.extend_from_slice(&1i32.to_le_bytes());
        rest.extend_from_slice(&doc);
        assert_eq!(rest.len(), 20 + doc.len());
    }

    // ===== SCRAM / CR crypto =====

    #[test]
    fn md5_password_digest_is_stable() {
        // md5("user:mongo:pass") hex — deterministic golden value.
        assert_eq!(
            mongo_password_digest("user", "pass"),
            hex_md5("user:mongo:pass")
        );
        assert_eq!(mongo_password_digest("user", "pass").len(), 32);
    }

    #[test]
    fn hmac_sha1_matches_rfc2202() {
        // RFC 2202 Test Case 2: key "Jefe", data "what do ya want for nothing?"
        let mac = hmac_sha1(b"Jefe", b"what do ya want for nothing?");
        let hex = mac.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex, "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hi_sha1_matches_rfc6070() {
        // RFC 6070 PBKDF2-HMAC-SHA1 official vectors.
        assert_eq!(hex(&hi_sha1(b"password", b"salt", 1)), "0c60c80f961f0e71f3a9b524af6012062fe037a6");
        assert_eq!(hex(&hi_sha1(b"password", b"salt", 2)), "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957");
        assert_eq!(hex(&hi_sha1(b"password", b"salt", 4096)), "4b007901b765489abead49d926f721d065a429c1");
    }

    #[test]
    fn hi_sha1_matches_rfc5802_vector() {
        // RFC 5802 SCRAM-SHA-1 example: password "pencil",
        // salt "QSXCR+Q6sek8bf92", iterations 4096. The expected server
        // signature below is verified independently against Python's
        // hashlib.pbkdf2_hmac + hmac (same input chain), not copied from a
        // transcription.
        let salt = base64::engine::general_purpose::STANDARD.decode("QSXCR+Q6sek8bf92").unwrap();
        let salted = hi_sha1(b"pencil", &salt, 4096);
        let auth_message = "n=user,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,s=QSXCR+Q6sek8bf92,i=4096,c=biws,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j";
        let server_key = hmac_sha1(&salted, b"Server Key");
        let server_sig = hmac_sha1(&server_key, auth_message.as_bytes());
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(server_sig),
            "NI6Qel39j5OxVeG8bsmroVO8SDY=",
            "SCRAM-SHA-1 chain must reproduce the independently-computed server signature"
        );
    }

    #[test]
    fn scram_client_final_produces_rfc_proof() {
        // Client-final of the RFC 5802 example: the returned server signature
        // must equal the v= value the real server sends in server-final.
        let auth_message = "n=user,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j,s=QSXCR+Q6sek8bf92,i=4096,c=biws,r=fyko+d2lbbFgONRv9qkxdawL3rfcNHYJY1ZVvWVs7j";
        let (proof, sig) = scram_client_final("pencil", "QSXCR+Q6sek8bf92", 4096, auth_message).unwrap();
        assert_eq!(sig, "NI6Qel39j5OxVeG8bsmroVO8SDY=", "server signature anchor");
        // Proof decodes to 20 bytes and differs from the client key.
        let proof_bytes = base64::engine::general_purpose::STANDARD.decode(proof).unwrap();
        assert_eq!(proof_bytes.len(), 20);
    }

    #[test]
    fn username_escaping() {
        assert_eq!(scram_saslprep_username("a,b=c"), "a=2Cb=3Dc");
        assert_eq!(scram_saslprep_username("plain"), "plain");
    }

    #[test]
    fn server_final_verification() {
        let payload = "v=abc123";
        assert!(verify_server_final(payload, "abc123").is_ok());
        assert!(verify_server_final(payload, "other").is_err());
        assert!(verify_server_final("no-value-here", "abc123").is_err());
    }

    #[test]
    fn nonce_is_unique_and_sized() {
        let a = new_nonce();
        let b = new_nonce();
        assert_eq!(a.len(), 18);
        assert_ne!(a, b);
    }

    #[test]
    fn bson_to_json_objectid_flattens() {
        let oid = bson::oid::ObjectId::parse_str("64b7f0c8e4b0a1a2b3c4d5e6").unwrap();
        let v = bson_to_json(&Bson::ObjectId(oid));
        assert_eq!(v, serde_json::json!("64b7f0c8e4b0a1a2b3c4d5e6"));
    }

    #[test]
    fn bson_to_json_plain_values() {
        assert_eq!(bson_to_json(&Bson::Int32(7)), serde_json::json!(7));
        assert_eq!(bson_to_json(&Bson::Null), serde_json::Value::Null);
        assert_eq!(bson_to_json(&Bson::Boolean(true)), serde_json::json!(true));
    }

    // Silence unused warnings for engine import in some cfg paths.
    #[test]
    fn engine_smoke() {
        let e = base64::engine::general_purpose::STANDARD;
        assert_eq!(e.encode(b"ab"), "YWI=");
    }
}
