use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use rand::RngCore;
use reqwest::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::task;
use uuid::Uuid;

const SQLITE_FILE_NAME: &str = "sessions.sqlite3";
const GLOBAL_MEMORY_FILE_NAME: &str = "memory.sqlite3";
const CURRENT_CONVERSATION_STATE_KEY: &str = "current_conversation_id";
const LEGACY_IMPORTED_STATE_KEY: &str = "legacy_import_done";
const EMBEDDING_DIMENSION: usize = 1024;
const SECRET_KEYCHAIN_SERVICE: &str = "ai-desktop-assistant";
const SECRET_KEYCHAIN_USERNAME: &str = "memory-master-key";
const SECRET_ALG: &str = "aes-256-gcm";

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(String),
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("base64 error: {0}")]
    Base64(String),
    #[error("embedding error: {0}")]
    Embedding(String),
    #[error("keyring error: {0}")]
    Keyring(String),
    #[error("encryption error: {0}")]
    Encryption(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
}

impl From<rusqlite::Error> for StorageError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InputImageAttachmentSnapshot {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub data_url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InputCardSnapshot {
    pub content: String,
    #[serde(default)]
    pub images: Vec<InputImageAttachmentSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageSnapshot {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub images: Vec<InputImageAttachmentSnapshot>,
    pub timestamp: i64,
    #[serde(default)]
    pub is_streaming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolCardSnapshot {
    pub id: String,
    pub task_id: String,
    pub seq: i64,
    pub direction: String,
    #[serde(rename = "type")]
    pub card_type: String,
    pub payload: Value,
    pub level: String,
    pub summary: String,
    pub timestamp: i64,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub retryable: bool,
    pub retry_input: Option<InputCardSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSnapshot {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub messages: Vec<MessageSnapshot>,
    #[serde(default)]
    pub protocol_cards: Vec<ProtocolCardSnapshot>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LegacySessionSnapshot {
    #[serde(default)]
    pub conversations: Vec<ConversationSnapshot>,
    pub current_conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StorageBootstrapRequest {
    pub legacy: Option<LegacySessionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StorageBootstrapResponse {
    #[serde(default)]
    pub conversations: Vec<ConversationSnapshot>,
    pub current_conversation_id: Option<String>,
    #[serde(default)]
    pub migrated_legacy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemorySearchScope {
    Workspace,
    Global,
    #[default]
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchRequest {
    pub query: String,
    pub scope: Option<MemorySearchScope>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_secrets: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchEntry {
    pub source: String,
    pub kind: String,
    pub memory_id: Option<i64>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub label: String,
    pub snippet: String,
    pub score: f32,
    #[serde(default)]
    pub secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResponse {
    #[serde(default)]
    pub results: Vec<MemorySearchEntry>,
    #[serde(default)]
    pub sqlite_vec_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpsertPersonalNoteRequest {
    pub label: String,
    pub descriptor_text: String,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpsertPersonalSecretRequest {
    pub label: String,
    pub descriptor_text: String,
    pub secret_text: String,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryEntry {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub descriptor_text: String,
    pub scope: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub has_secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryListResponse {
    #[serde(default)]
    pub items: Vec<PersonalMemoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDeletePersonalRequest {
    pub id: i64,
}

#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, StorageError>;
}

#[derive(Clone)]
pub struct GlmEmbeddingClient {
    http_client: Client,
    endpoint: String,
    model: String,
}

impl GlmEmbeddingClient {
    pub fn new() -> Self {
        Self {
            http_client: Client::new(),
            endpoint: std::env::var("GLM_EMBEDDING_ENDPOINT")
                .unwrap_or_else(|_| "https://open.bigmodel.cn/api/paas/v4/embeddings".to_string()),
            model: std::env::var("GLM_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "embedding-3".to_string()),
        }
    }
}

#[async_trait]
impl EmbeddingClient for GlmEmbeddingClient {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, StorageError> {
        let api_key = std::env::var("GLM_API_KEY")
            .map_err(|_| StorageError::Embedding("missing GLM_API_KEY".to_string()))?;
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
        });
        let response = self
            .http_client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|err| StorageError::Embedding(err.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(StorageError::Embedding(format!(
                "glm embedding request failed: status={}, body={}",
                status, body
            )));
        }
        let json: Value = response
            .json()
            .await
            .map_err(|err| StorageError::Embedding(err.to_string()))?;
        extract_embedding_vector(&json)
    }
}

#[derive(Clone)]
pub struct StorageService {
    embedding_client: Arc<dyn EmbeddingClient>,
}

impl Default for StorageService {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageService {
    pub fn new() -> Self {
        Self {
            embedding_client: Arc::new(GlmEmbeddingClient::new()),
        }
    }

    #[cfg(test)]
    pub fn with_embedding_client(embedding_client: Arc<dyn EmbeddingClient>) -> Self {
        Self { embedding_client }
    }

    pub async fn bootstrap_workspace(
        &self,
        workspace_root: &Path,
        request: StorageBootstrapRequest,
    ) -> Result<StorageBootstrapResponse, StorageError> {
        let conn = open_workspace_connection(workspace_root)?;
        let mut migrated_legacy = false;
        let imported = get_workspace_state(&conn, LEGACY_IMPORTED_STATE_KEY)?;
        if imported.as_deref() != Some("1") {
            if let Some(legacy) = request.legacy {
                if !legacy.conversations.is_empty() || legacy.current_conversation_id.is_some() {
                    import_legacy_snapshot(&conn, workspace_root, &legacy)?;
                    migrated_legacy = true;
                }
            }
            set_workspace_state(&conn, LEGACY_IMPORTED_STATE_KEY, Some("1"))?;
        }
        let conversations = load_all_conversations(&conn, workspace_root)?;
        let current_conversation_id = get_workspace_state(&conn, CURRENT_CONVERSATION_STATE_KEY)?;
        Ok(StorageBootstrapResponse {
            conversations,
            current_conversation_id,
            migrated_legacy,
        })
    }

    pub async fn upsert_conversation(
        &self,
        workspace_root: &Path,
        snapshot: ConversationSnapshot,
    ) -> Result<(), StorageError> {
        let conn = open_workspace_connection(workspace_root)?;
        upsert_conversation_snapshot(&conn, workspace_root, &snapshot)?;
        let service = self.clone();
        let workspace = workspace_root.to_path_buf();
        task::spawn(async move {
            let _ = service.process_workspace_index_jobs(&workspace, 24).await;
        });
        Ok(())
    }

    pub async fn delete_conversation(
        &self,
        workspace_root: &Path,
        conversation_id: &str,
    ) -> Result<(), StorageError> {
        let conn = open_workspace_connection(workspace_root)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM conversations WHERE id = ?1",
            params![conversation_id],
        )?;
        if get_workspace_state_with_tx(&tx, CURRENT_CONVERSATION_STATE_KEY)?.as_deref()
            == Some(conversation_id)
        {
            set_workspace_state_with_tx(&tx, CURRENT_CONVERSATION_STATE_KEY, None)?;
        }
        tx.commit()?;
        let assets = conversation_assets_dir(workspace_root, conversation_id);
        if assets.exists() {
            let _ = fs::remove_dir_all(assets);
        }
        Ok(())
    }

    pub async fn set_current_conversation(
        &self,
        workspace_root: &Path,
        conversation_id: Option<String>,
    ) -> Result<(), StorageError> {
        let conn = open_workspace_connection(workspace_root)?;
        set_workspace_state(
            &conn,
            CURRENT_CONVERSATION_STATE_KEY,
            conversation_id.as_deref(),
        )?;
        Ok(())
    }

    pub async fn export_conversation(
        &self,
        workspace_root: &Path,
        conversation_id: &str,
    ) -> Result<ConversationSnapshot, StorageError> {
        let conn = open_workspace_connection(workspace_root)?;
        load_single_conversation(&conn, workspace_root, conversation_id)?.ok_or_else(|| {
            StorageError::NotFound(format!("conversation '{}' not found", conversation_id))
        })
    }

    pub async fn memory_search(
        &self,
        workspace_root: &Path,
        request: MemorySearchRequest,
    ) -> Result<MemorySearchResponse, StorageError> {
        search_memory(self.embedding_client.clone(), workspace_root, request).await
    }

    pub async fn upsert_personal_note(
        &self,
        request: MemoryUpsertPersonalNoteRequest,
    ) -> Result<PersonalMemoryEntry, StorageError> {
        upsert_personal_note(self.embedding_client.clone(), request).await
    }

    pub async fn upsert_personal_secret(
        &self,
        request: MemoryUpsertPersonalSecretRequest,
    ) -> Result<PersonalMemoryEntry, StorageError> {
        upsert_personal_secret(self.embedding_client.clone(), request).await
    }

    pub async fn list_personal(&self) -> Result<PersonalMemoryListResponse, StorageError> {
        list_personal_memories()
    }

    pub async fn delete_personal(
        &self,
        request: MemoryDeletePersonalRequest,
    ) -> Result<(), StorageError> {
        delete_personal_memory(request)
    }

    pub async fn build_recall_context(
        &self,
        workspace_root: &Path,
        query: &str,
    ) -> Result<Option<String>, StorageError> {
        build_recall_context(self.embedding_client.clone(), workspace_root, query).await
    }

    pub async fn process_workspace_index_jobs(
        &self,
        workspace_root: &Path,
        max_jobs: usize,
    ) -> Result<(), StorageError> {
        process_workspace_index_jobs(self.embedding_client.clone(), workspace_root, max_jobs).await
    }
}

fn workspace_database_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(".ah")
        .join("config")
        .join(SQLITE_FILE_NAME)
}

fn global_memory_database_path() -> PathBuf {
    let base = resolve_user_home_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base.join(".ai-helper")
        .join("global")
        .join(GLOBAL_MEMORY_FILE_NAME)
}

fn conversation_assets_dir(workspace_root: &Path, conversation_id: &str) -> PathBuf {
    workspace_root
        .join(".ah")
        .join("assets")
        .join("conversations")
        .join(conversation_id)
}

fn open_workspace_connection(workspace_root: &Path) -> Result<Connection, StorageError> {
    let db_path = workspace_database_path(workspace_root);
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|err| StorageError::Io(err.to_string()))?;
    }
    register_sqlite_vec_auto_extension();
    let conn = Connection::open(db_path)?;
    configure_connection(&conn)?;
    apply_workspace_schema(&conn)?;
    Ok(conn)
}

fn open_global_memory_connection() -> Result<Connection, StorageError> {
    let db_path = global_memory_database_path();
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|err| StorageError::Io(err.to_string()))?;
    }
    register_sqlite_vec_auto_extension();
    let conn = Connection::open(db_path)?;
    configure_connection(&conn)?;
    apply_global_schema(&conn)?;
    Ok(conn)
}

fn configure_connection(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

fn apply_workspace_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations(
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS conversations(
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            pinned INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS messages(
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            is_streaming INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS message_images(
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            name TEXT NOT NULL,
            mime_type TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            asset_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE,
            FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS protocol_cards(
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            direction TEXT NOT NULL,
            type TEXT NOT NULL,
            level TEXT NOT NULL,
            summary TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            streaming INTEGER NOT NULL DEFAULT 0,
            retryable INTEGER NOT NULL DEFAULT 0,
            retry_input_json TEXT,
            FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS workspace_state(
            key TEXT PRIMARY KEY,
            value TEXT
         );
         CREATE TABLE IF NOT EXISTS memory_chunks(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL,
            message_id TEXT NOT NULL,
            chunk_text TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
            FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS memory_index_jobs(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chunk_id INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            scheduled_at INTEGER NOT NULL,
            FOREIGN KEY(chunk_id) REFERENCES memory_chunks(id) ON DELETE CASCADE
         );",
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(1, ?1)",
        params![now_unix_ms()],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(2, ?1)",
        params![now_unix_ms()],
    )?;

    if sqlite_vec_available_on_connection(conn) {
        let ddl = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_chunks_vec USING vec0(embedding float[{EMBEDDING_DIMENSION}]);"
        );
        let _ = conn.execute_batch(&ddl);
    }

    Ok(())
}

fn apply_global_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations(
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS personal_memories(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            descriptor_text TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'global',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS personal_secrets(
            memory_id INTEGER PRIMARY KEY,
            cipher_text TEXT NOT NULL,
            nonce TEXT NOT NULL,
            alg TEXT NOT NULL,
            key_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY(memory_id) REFERENCES personal_memories(id) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS secret_injection_audit(
            id TEXT PRIMARY KEY,
            memory_id INTEGER NOT NULL,
            label TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at INTEGER NOT NULL
         );",
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(1, ?1)",
        params![now_unix_ms()],
    )?;

    if sqlite_vec_available_on_connection(conn) {
        let ddl = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS personal_memories_vec USING vec0(embedding float[{EMBEDDING_DIMENSION}]);"
        );
        let _ = conn.execute_batch(&ddl);
    }

    Ok(())
}

fn import_legacy_snapshot(
    conn: &Connection,
    workspace_root: &Path,
    legacy: &LegacySessionSnapshot,
) -> Result<(), StorageError> {
    let tx = conn.unchecked_transaction()?;
    for conversation in &legacy.conversations {
        upsert_conversation_snapshot_with_tx(&tx, workspace_root, conversation)?;
    }
    set_workspace_state_with_tx(
        &tx,
        CURRENT_CONVERSATION_STATE_KEY,
        legacy.current_conversation_id.as_deref(),
    )?;
    tx.commit()?;
    Ok(())
}

fn upsert_conversation_snapshot(
    conn: &Connection,
    workspace_root: &Path,
    snapshot: &ConversationSnapshot,
) -> Result<(), StorageError> {
    let tx = conn.unchecked_transaction()?;
    upsert_conversation_snapshot_with_tx(&tx, workspace_root, snapshot)?;
    tx.commit()?;
    Ok(())
}

fn upsert_conversation_snapshot_with_tx(
    tx: &rusqlite::Transaction<'_>,
    workspace_root: &Path,
    snapshot: &ConversationSnapshot,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO conversations(id, title, pinned, created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            pinned = excluded.pinned,
            updated_at = excluded.updated_at",
        params![
            snapshot.id,
            snapshot.title,
            bool_to_i64(snapshot.pinned),
            snapshot.created_at,
            snapshot.updated_at
        ],
    )?;

    tx.execute(
        "DELETE FROM protocol_cards WHERE conversation_id = ?1",
        params![snapshot.id],
    )?;
    tx.execute(
        "DELETE FROM messages WHERE conversation_id = ?1",
        params![snapshot.id],
    )?;
    tx.execute(
        "DELETE FROM memory_index_jobs
         WHERE chunk_id IN (SELECT id FROM memory_chunks WHERE conversation_id = ?1)",
        params![snapshot.id],
    )?;
    tx.execute(
        "DELETE FROM memory_chunks WHERE conversation_id = ?1",
        params![snapshot.id],
    )?;

    let assets = conversation_assets_dir(workspace_root, &snapshot.id);
    if assets.exists() {
        let _ = fs::remove_dir_all(&assets);
    }
    fs::create_dir_all(&assets).map_err(|err| StorageError::Io(err.to_string()))?;

    for message in &snapshot.messages {
        tx.execute(
            "INSERT INTO messages(id, conversation_id, role, content, timestamp, is_streaming)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                message.id,
                snapshot.id,
                message.role,
                message.content,
                message.timestamp,
                bool_to_i64(message.is_streaming)
            ],
        )?;

        for image in &message.images {
            let (mime_type, bytes) = decode_data_url(&image.data_url)?;
            let ext = file_extension_from_mime(&mime_type);
            let relative = PathBuf::from(".ah")
                .join("assets")
                .join("conversations")
                .join(&snapshot.id)
                .join(&message.id)
                .join(format!("{}.{}", image.id, ext));
            let absolute = workspace_root.join(&relative);
            if let Some(parent) = absolute.parent() {
                fs::create_dir_all(parent).map_err(|err| StorageError::Io(err.to_string()))?;
            }
            fs::write(&absolute, &bytes).map_err(|err| StorageError::Io(err.to_string()))?;
            tx.execute(
                "INSERT INTO message_images(id, message_id, conversation_id, name, mime_type, size_bytes, asset_path, sha256)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    image.id,
                    message.id,
                    snapshot.id,
                    image.name,
                    mime_type,
                    image.size_bytes as i64,
                    relative.to_string_lossy().to_string(),
                    hex_sha256(&bytes)
                ],
            )?;
        }

        let chunk_text = message.content.trim();
        if !chunk_text.is_empty() {
            tx.execute(
                "INSERT INTO memory_chunks(conversation_id, message_id, chunk_text, content_hash, created_at, updated_at, status)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
                params![
                    snapshot.id,
                    message.id,
                    chunk_text,
                    hex_sha256(chunk_text.as_bytes()),
                    message.timestamp,
                    message.timestamp
                ],
            )?;
            let chunk_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_index_jobs(chunk_id, status, retry_count, last_error, scheduled_at)
                 VALUES(?1, 'pending', 0, NULL, ?2)",
                params![chunk_id, now_unix_ms()],
            )?;
        }
    }

    for card in &snapshot.protocol_cards {
        tx.execute(
            "INSERT INTO protocol_cards(
                id, conversation_id, task_id, seq, direction, type, level, summary,
                payload_json, timestamp, streaming, retryable, retry_input_json
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                card.id,
                snapshot.id,
                card.task_id,
                card.seq,
                card.direction,
                card.card_type,
                card.level,
                card.summary,
                serde_json::to_string(&card.payload)?,
                card.timestamp,
                bool_to_i64(card.streaming),
                bool_to_i64(card.retryable),
                card.retry_input
                    .as_ref()
                    .map(|value| serde_json::to_string(value))
                    .transpose()?
            ],
        )?;
    }

    Ok(())
}

fn load_all_conversations(
    conn: &Connection,
    workspace_root: &Path,
) -> Result<Vec<ConversationSnapshot>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, title, pinned, created_at, updated_at
         FROM conversations
         ORDER BY pinned DESC, updated_at DESC, id DESC",
    )?;
    let mut rows = stmt.query([])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        items.push(ConversationSnapshot {
            id: id.clone(),
            title: row.get(1)?,
            pinned: row.get::<_, i64>(2)? == 1,
            messages: load_messages(conn, workspace_root, &id)?,
            protocol_cards: load_protocol_cards(conn, &id)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        });
    }
    Ok(items)
}

fn load_single_conversation(
    conn: &Connection,
    workspace_root: &Path,
    conversation_id: &str,
) -> Result<Option<ConversationSnapshot>, StorageError> {
    let row = conn
        .query_row(
            "SELECT id, title, pinned, created_at, updated_at
             FROM conversations
             WHERE id = ?1",
            params![conversation_id],
            |row| {
                Ok(ConversationSnapshot {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    pinned: row.get::<_, i64>(2)? == 1,
                    messages: Vec::new(),
                    protocol_cards: Vec::new(),
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .optional()?;
    let Some(mut snapshot) = row else {
        return Ok(None);
    };
    snapshot.messages = load_messages(conn, workspace_root, &snapshot.id)?;
    snapshot.protocol_cards = load_protocol_cards(conn, &snapshot.id)?;
    Ok(Some(snapshot))
}

fn load_messages(
    conn: &Connection,
    workspace_root: &Path,
    conversation_id: &str,
) -> Result<Vec<MessageSnapshot>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content, timestamp, is_streaming
         FROM messages
         WHERE conversation_id = ?1
         ORDER BY timestamp ASC, id ASC",
    )?;
    let mut rows = stmt.query(params![conversation_id])?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next()? {
        let message_id: String = row.get(0)?;
        messages.push(MessageSnapshot {
            id: message_id.clone(),
            role: row.get(1)?,
            content: row.get(2)?,
            images: load_message_images(conn, workspace_root, conversation_id, &message_id)?,
            timestamp: row.get(3)?,
            is_streaming: row.get::<_, i64>(4)? == 1,
        });
    }
    Ok(messages)
}

fn load_message_images(
    conn: &Connection,
    workspace_root: &Path,
    conversation_id: &str,
    message_id: &str,
) -> Result<Vec<InputImageAttachmentSnapshot>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, mime_type, size_bytes, asset_path
         FROM message_images
         WHERE conversation_id = ?1 AND message_id = ?2
         ORDER BY id ASC",
    )?;
    let mut rows = stmt.query(params![conversation_id, message_id])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        let asset_path: String = row.get(4)?;
        let absolute = workspace_root.join(asset_path);
        let data_url = if absolute.exists() {
            let bytes = fs::read(absolute).map_err(|err| StorageError::Io(err.to_string()))?;
            format!(
                "data:{};base64,{}",
                row.get::<_, String>(2)?,
                BASE64_STANDARD.encode(bytes)
            )
        } else {
            String::new()
        };
        items.push(InputImageAttachmentSnapshot {
            id: row.get(0)?,
            name: row.get(1)?,
            mime_type: row.get(2)?,
            data_url,
            size_bytes: row.get::<_, i64>(3)?.max(0) as u64,
        });
    }
    Ok(items)
}

fn load_protocol_cards(
    conn: &Connection,
    conversation_id: &str,
) -> Result<Vec<ProtocolCardSnapshot>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, seq, direction, type, payload_json, level, summary, timestamp, streaming, retryable, retry_input_json
         FROM protocol_cards
         WHERE conversation_id = ?1
         ORDER BY seq ASC, timestamp ASC, id ASC",
    )?;
    let mut rows = stmt.query(params![conversation_id])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        let payload: String = row.get(5)?;
        let retry_input_json: Option<String> = row.get(11)?;
        items.push(ProtocolCardSnapshot {
            id: row.get(0)?,
            task_id: row.get(1)?,
            seq: row.get(2)?,
            direction: row.get(3)?,
            card_type: row.get(4)?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            level: row.get(6)?,
            summary: row.get(7)?,
            timestamp: row.get(8)?,
            streaming: row.get::<_, i64>(9)? == 1,
            retryable: row.get::<_, i64>(10)? == 1,
            retry_input: retry_input_json
                .as_deref()
                .and_then(|text| serde_json::from_str(text).ok()),
        });
    }
    Ok(items)
}

fn get_workspace_state(conn: &Connection, key: &str) -> Result<Option<String>, StorageError> {
    conn.query_row(
        "SELECT value FROM workspace_state WHERE key = ?1",
        params![key],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|value| value.flatten())
    .map_err(Into::into)
}

fn get_workspace_state_with_tx(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
) -> Result<Option<String>, StorageError> {
    tx.query_row(
        "SELECT value FROM workspace_state WHERE key = ?1",
        params![key],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|value| value.flatten())
    .map_err(Into::into)
}

fn set_workspace_state(
    conn: &Connection,
    key: &str,
    value: Option<&str>,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO workspace_state(key, value)
         VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn set_workspace_state_with_tx(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    value: Option<&str>,
) -> Result<(), StorageError> {
    tx.execute(
        "INSERT INTO workspace_state(key, value)
         VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn decode_data_url(data_url: &str) -> Result<(String, Vec<u8>), StorageError> {
    let trimmed = data_url.trim();
    if !trimmed.starts_with("data:") {
        return Err(StorageError::InvalidInput(
            "image dataUrl must start with data:".to_string(),
        ));
    }
    let payload = &trimmed[5..];
    let Some((meta, encoded)) = payload.split_once(',') else {
        return Err(StorageError::InvalidInput(
            "invalid dataUrl format".to_string(),
        ));
    };
    if !meta.contains(";base64") {
        return Err(StorageError::InvalidInput(
            "dataUrl without base64 is not supported".to_string(),
        ));
    }
    let mime_type = meta
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string();
    let bytes = BASE64_STANDARD
        .decode(encoded)
        .map_err(|err| StorageError::Base64(err.to_string()))?;
    Ok((mime_type, bytes))
}

fn file_extension_from_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn now_unix_ms() -> i64 {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_millis(0));
    duration.as_millis() as i64
}

fn hex_sha256(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn resolve_user_home_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    } else {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

fn extract_embedding_vector(value: &Value) -> Result<Vec<f32>, StorageError> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| StorageError::Embedding("embedding response missing data".to_string()))?;
    let first = data
        .first()
        .ok_or_else(|| StorageError::Embedding("embedding response data is empty".to_string()))?;
    let values = first
        .get("embedding")
        .and_then(Value::as_array)
        .ok_or_else(|| StorageError::Embedding("embedding response missing vector".to_string()))?;
    let mut vector = Vec::with_capacity(values.len());
    for value in values {
        if let Some(number) = value.as_f64() {
            vector.push(number as f32);
        }
    }
    if vector.is_empty() {
        return Err(StorageError::Embedding(
            "embedding vector is empty".to_string(),
        ));
    }
    Ok(normalize_embedding_dimension(vector))
}

fn normalize_embedding_dimension(mut vector: Vec<f32>) -> Vec<f32> {
    if vector.len() > EMBEDDING_DIMENSION {
        vector.truncate(EMBEDDING_DIMENSION);
    } else if vector.len() < EMBEDDING_DIMENSION {
        vector.resize(EMBEDDING_DIMENSION, 0.0);
    }
    vector
}

#[derive(Debug, Clone)]
struct EncryptedSecretPayload {
    cipher_text: String,
    nonce: String,
}

fn get_or_create_master_key() -> Result<Vec<u8>, StorageError> {
    let entry = keyring::Entry::new(SECRET_KEYCHAIN_SERVICE, SECRET_KEYCHAIN_USERNAME)
        .map_err(|err| StorageError::Keyring(err.to_string()))?;
    if let Ok(existing) = entry.get_password() {
        if let Ok(decoded) = BASE64_STANDARD.decode(existing) {
            if decoded.len() == 32 {
                return Ok(decoded);
            }
        }
    }
    let mut key = vec![0_u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let encoded = BASE64_STANDARD.encode(&key);
    entry
        .set_password(&encoded)
        .map_err(|err| StorageError::Keyring(err.to_string()))?;
    Ok(key)
}

fn encrypt_secret_text(secret_text: &str) -> Result<EncryptedSecretPayload, StorageError> {
    let key = get_or_create_master_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|err| StorageError::Encryption(err.to_string()))?;
    let mut nonce = [0_u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), secret_text.as_bytes())
        .map_err(|err| StorageError::Encryption(err.to_string()))?;
    Ok(EncryptedSecretPayload {
        cipher_text: BASE64_STANDARD.encode(ciphertext),
        nonce: BASE64_STANDARD.encode(nonce),
    })
}

fn decrypt_secret_text(cipher_text: &str, nonce_b64: &str) -> Result<String, StorageError> {
    let key = get_or_create_master_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|err| StorageError::Encryption(err.to_string()))?;
    let nonce_bytes = BASE64_STANDARD
        .decode(nonce_b64)
        .map_err(|err| StorageError::Base64(err.to_string()))?;
    if nonce_bytes.len() != 12 {
        return Err(StorageError::Encryption("invalid nonce length".to_string()));
    }
    let encrypted_bytes = BASE64_STANDARD
        .decode(cipher_text)
        .map_err(|err| StorageError::Base64(err.to_string()))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), encrypted_bytes.as_ref())
        .map_err(|err| StorageError::Encryption(err.to_string()))?;
    String::from_utf8(plaintext).map_err(|err| StorageError::Encryption(err.to_string()))
}

fn normalize_scope(scope: &str) -> String {
    let trimmed = scope.trim();
    if trimmed.is_empty() {
        "global".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_descriptor_text(descriptor_text: &str, label: &str) -> String {
    let trimmed = descriptor_text.trim();
    if trimmed.is_empty() {
        label.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn score_from_distance(distance: f64) -> f32 {
    (1.0 / (1.0 + distance.max(0.0))) as f32
}

fn truncate_snippet(input: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for ch in input.chars().take(max_chars) {
        output.push(ch);
    }
    if input.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn detect_secret_intent(input: &str) -> bool {
    let normalized = input.to_lowercase();
    let keywords = [
        "password",
        "passwd",
        "token",
        "secret",
        "api key",
        "credential",
        "login",
        "账号",
        "密码",
        "口令",
        "密钥",
    ];
    keywords.iter().any(|keyword| normalized.contains(keyword))
}

async fn search_memory(
    embedding_client: Arc<dyn EmbeddingClient>,
    workspace_root: &Path,
    request: MemorySearchRequest,
) -> Result<MemorySearchResponse, StorageError> {
    let query = request.query.trim();
    if query.is_empty() {
        return Ok(MemorySearchResponse {
            results: Vec::new(),
            sqlite_vec_available: sqlite_vec_available(),
        });
    }
    let scope = request.scope.unwrap_or_default();
    let limit = request.limit.unwrap_or(8).clamp(1, 30);
    let embedding = embedding_client.embed(query).await.ok();
    let mut results = Vec::new();

    if matches!(
        scope,
        MemorySearchScope::Workspace | MemorySearchScope::Both
    ) {
        let conn = open_workspace_connection(workspace_root)?;
        if let Some(vector) = embedding.as_ref() {
            if sqlite_vec_available_on_connection(&conn) {
                let vector_json = serde_json::to_string(vector)?;
                let sql =
                    "SELECT mc.id, mc.chunk_text, mc.conversation_id, mc.message_id, v.distance
                           FROM memory_chunks_vec v
                           JOIN memory_chunks mc ON mc.id = v.rowid
                           WHERE v.embedding MATCH ?1 AND k = ?2
                           ORDER BY v.distance ASC
                           LIMIT ?2";
                if let Ok(mut stmt) = conn.prepare(sql) {
                    let mut rows = stmt.query(params![vector_json, limit as i64])?;
                    while let Some(row) = rows.next()? {
                        results.push(MemorySearchEntry {
                            source: "workspace".to_string(),
                            kind: "conversation".to_string(),
                            memory_id: Some(row.get(0)?),
                            conversation_id: Some(row.get(2)?),
                            message_id: Some(row.get(3)?),
                            label: "Conversation memory".to_string(),
                            snippet: truncate_snippet(&row.get::<_, String>(1)?, 220),
                            score: score_from_distance(row.get::<_, f64>(4).unwrap_or(2.0)),
                            secret: false,
                        });
                    }
                }
            }
        }
        if results.is_empty() {
            let like = format!("%{}%", query);
            let mut stmt = conn.prepare(
                "SELECT id, chunk_text, conversation_id, message_id
                 FROM memory_chunks
                 WHERE chunk_text LIKE ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![like, limit as i64])?;
            while let Some(row) = rows.next()? {
                results.push(MemorySearchEntry {
                    source: "workspace".to_string(),
                    kind: "conversation".to_string(),
                    memory_id: Some(row.get(0)?),
                    conversation_id: Some(row.get(2)?),
                    message_id: Some(row.get(3)?),
                    label: "Conversation memory".to_string(),
                    snippet: truncate_snippet(&row.get::<_, String>(1)?, 220),
                    score: 0.72,
                    secret: false,
                });
            }
        }
    }

    if matches!(scope, MemorySearchScope::Global | MemorySearchScope::Both) {
        let conn = open_global_memory_connection()?;
        let mut global_results = Vec::new();
        if let Some(vector) = embedding.as_ref() {
            if sqlite_vec_available_on_connection(&conn) {
                let vector_json = serde_json::to_string(vector)?;
                let sql = "SELECT pm.id, pm.kind, pm.label, pm.descriptor_text, v.distance
                           FROM personal_memories_vec v
                           JOIN personal_memories pm ON pm.id = v.rowid
                           WHERE v.embedding MATCH ?1 AND k = ?2
                           ORDER BY v.distance ASC
                           LIMIT ?2";
                if let Ok(mut stmt) = conn.prepare(sql) {
                    let mut rows = stmt.query(params![vector_json, limit as i64])?;
                    while let Some(row) = rows.next()? {
                        let kind: String = row.get(1)?;
                        let is_secret = kind == "secret";
                        if is_secret && !request.include_secrets {
                            continue;
                        }
                        let label: String = row.get(2)?;
                        let descriptor: String = row.get(3)?;
                        global_results.push(MemorySearchEntry {
                            source: "global".to_string(),
                            kind,
                            memory_id: Some(row.get(0)?),
                            conversation_id: None,
                            message_id: None,
                            label: label.clone(),
                            snippet: if is_secret {
                                format!("Secret memory: {}", label)
                            } else {
                                truncate_snippet(&descriptor, 220)
                            },
                            score: score_from_distance(row.get::<_, f64>(4).unwrap_or(2.0)),
                            secret: is_secret,
                        });
                    }
                }
            }
        }
        if global_results.is_empty() {
            let like = format!("%{}%", query);
            let mut stmt = conn.prepare(
                "SELECT id, kind, label, descriptor_text
                 FROM personal_memories
                 WHERE label LIKE ?1 OR descriptor_text LIKE ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![like, limit as i64])?;
            while let Some(row) = rows.next()? {
                let kind: String = row.get(1)?;
                let is_secret = kind == "secret";
                if is_secret && !request.include_secrets {
                    continue;
                }
                let label: String = row.get(2)?;
                global_results.push(MemorySearchEntry {
                    source: "global".to_string(),
                    kind,
                    memory_id: Some(row.get(0)?),
                    conversation_id: None,
                    message_id: None,
                    label: label.clone(),
                    snippet: if is_secret {
                        format!("Secret memory: {}", label)
                    } else {
                        truncate_snippet(&row.get::<_, String>(3)?, 220)
                    },
                    score: 0.70,
                    secret: is_secret,
                });
            }
        }
        results.extend(global_results);
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if results.len() > limit {
        results.truncate(limit);
    }
    Ok(MemorySearchResponse {
        results,
        sqlite_vec_available: sqlite_vec_available(),
    })
}

async fn upsert_personal_note(
    embedding_client: Arc<dyn EmbeddingClient>,
    request: MemoryUpsertPersonalNoteRequest,
) -> Result<PersonalMemoryEntry, StorageError> {
    let label = request.label.trim();
    if label.is_empty() {
        return Err(StorageError::InvalidInput(
            "personal note label cannot be empty".to_string(),
        ));
    }
    let descriptor = normalize_descriptor_text(&request.descriptor_text, label);
    let scope = normalize_scope(&request.scope);
    let now = now_unix_ms();
    let memory_id = {
        let conn = open_global_memory_connection()?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO personal_memories(kind, label, descriptor_text, scope, created_at, updated_at)
             VALUES('note', ?1, ?2, ?3, ?4, ?5)",
            params![label, descriptor, scope, now, now],
        )?;
        let memory_id = tx.last_insert_rowid();
        tx.commit()?;
        memory_id
    };

    if sqlite_vec_available() {
        if let Ok(vector) = embedding_client.embed(&descriptor).await {
            let conn = open_global_memory_connection()?;
            if sqlite_vec_available_on_connection(&conn) {
                let vector_json = serde_json::to_string(&normalize_embedding_dimension(vector))?;
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO personal_memories_vec(rowid, embedding) VALUES(?1, ?2)",
                    params![memory_id, vector_json],
                );
            }
        }
    }

    Ok(PersonalMemoryEntry {
        id: memory_id,
        kind: "note".to_string(),
        label: label.to_string(),
        descriptor_text: descriptor,
        scope,
        created_at: now,
        updated_at: now,
        has_secret: false,
    })
}

async fn upsert_personal_secret(
    embedding_client: Arc<dyn EmbeddingClient>,
    request: MemoryUpsertPersonalSecretRequest,
) -> Result<PersonalMemoryEntry, StorageError> {
    let label = request.label.trim();
    if label.is_empty() {
        return Err(StorageError::InvalidInput(
            "personal secret label cannot be empty".to_string(),
        ));
    }
    let secret_text = request.secret_text.trim();
    if secret_text.is_empty() {
        return Err(StorageError::InvalidInput(
            "secret text cannot be empty".to_string(),
        ));
    }
    let descriptor = normalize_descriptor_text(&request.descriptor_text, label);
    let scope = normalize_scope(&request.scope);
    let encrypted = encrypt_secret_text(secret_text)?;
    let now = now_unix_ms();
    let memory_id = {
        let conn = open_global_memory_connection()?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO personal_memories(kind, label, descriptor_text, scope, created_at, updated_at)
             VALUES('secret', ?1, ?2, ?3, ?4, ?5)",
            params![label, descriptor, scope, now, now],
        )?;
        let memory_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO personal_secrets(memory_id, cipher_text, nonce, alg, key_id, created_at, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                memory_id,
                encrypted.cipher_text,
                encrypted.nonce,
                SECRET_ALG,
                SECRET_KEYCHAIN_USERNAME,
                now,
                now
            ],
        )?;
        tx.commit()?;
        memory_id
    };

    if sqlite_vec_available() {
        if let Ok(vector) = embedding_client.embed(&descriptor).await {
            let conn = open_global_memory_connection()?;
            if sqlite_vec_available_on_connection(&conn) {
                let vector_json = serde_json::to_string(&normalize_embedding_dimension(vector))?;
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO personal_memories_vec(rowid, embedding) VALUES(?1, ?2)",
                    params![memory_id, vector_json],
                );
            }
        }
    }

    Ok(PersonalMemoryEntry {
        id: memory_id,
        kind: "secret".to_string(),
        label: label.to_string(),
        descriptor_text: descriptor,
        scope,
        created_at: now,
        updated_at: now,
        has_secret: true,
    })
}

fn list_personal_memories() -> Result<PersonalMemoryListResponse, StorageError> {
    let conn = open_global_memory_connection()?;
    let mut stmt = conn.prepare(
        "SELECT pm.id, pm.kind, pm.label, pm.descriptor_text, pm.scope, pm.created_at, pm.updated_at,
                CASE WHEN ps.memory_id IS NULL THEN 0 ELSE 1 END AS has_secret
         FROM personal_memories pm
         LEFT JOIN personal_secrets ps ON ps.memory_id = pm.id
         ORDER BY pm.updated_at DESC, pm.id DESC",
    )?;
    let mut rows = stmt.query([])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        items.push(PersonalMemoryEntry {
            id: row.get(0)?,
            kind: row.get(1)?,
            label: row.get(2)?,
            descriptor_text: row.get(3)?,
            scope: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            has_secret: row.get::<_, i64>(7)? == 1,
        });
    }
    Ok(PersonalMemoryListResponse { items })
}

fn delete_personal_memory(request: MemoryDeletePersonalRequest) -> Result<(), StorageError> {
    let conn = open_global_memory_connection()?;
    conn.execute(
        "DELETE FROM personal_memories WHERE id = ?1",
        params![request.id],
    )?;
    if sqlite_vec_available_on_connection(&conn) {
        let _ = conn.execute(
            "DELETE FROM personal_memories_vec WHERE rowid = ?1",
            params![request.id],
        );
    }
    Ok(())
}

async fn build_recall_context(
    embedding_client: Arc<dyn EmbeddingClient>,
    workspace_root: &Path,
    query: &str,
) -> Result<Option<String>, StorageError> {
    let normalized = query.trim();
    if normalized.is_empty() {
        return Ok(None);
    }

    let secret_intent = detect_secret_intent(normalized);
    let response = search_memory(
        embedding_client,
        workspace_root,
        MemorySearchRequest {
            query: normalized.to_string(),
            scope: Some(MemorySearchScope::Both),
            limit: Some(8),
            include_secrets: true,
        },
    )
    .await?;
    if response.results.is_empty() {
        return Ok(None);
    }

    let mut lines = vec!["Memory recall context (internal only):".to_string()];
    let mut added = false;

    for entry in response.results.iter().filter(|item| !item.secret).take(3) {
        lines.push(format!(
            "- {} [{}]: {}",
            entry.label, entry.source, entry.snippet
        ));
        added = true;
    }

    if secret_intent {
        for entry in response
            .results
            .iter()
            .filter(|item| item.secret && item.score >= 0.78)
            .take(2)
        {
            if let Some(memory_id) = entry.memory_id {
                if let Ok(secret_value) = load_decrypted_secret(memory_id) {
                    lines.push(format!("- {} [credential]: {}", entry.label, secret_value));
                    added = true;
                    let _ = audit_secret_injection(memory_id, &entry.label, "intent+similarity");
                }
            }
        }
    }

    if !added {
        return Ok(None);
    }
    Ok(Some(lines.join("\n")))
}

fn load_decrypted_secret(memory_id: i64) -> Result<String, StorageError> {
    let conn = open_global_memory_connection()?;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT cipher_text, nonce FROM personal_secrets WHERE memory_id = ?1",
            params![memory_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((cipher_text, nonce)) = row else {
        return Err(StorageError::NotFound(format!(
            "secret memory {} not found",
            memory_id
        )));
    };
    decrypt_secret_text(&cipher_text, &nonce)
}

fn audit_secret_injection(memory_id: i64, label: &str, reason: &str) -> Result<(), StorageError> {
    let conn = open_global_memory_connection()?;
    conn.execute(
        "INSERT INTO secret_injection_audit(id, memory_id, label, reason, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5)",
        params![
            Uuid::new_v4().to_string(),
            memory_id,
            label,
            reason,
            now_unix_ms()
        ],
    )?;
    Ok(())
}

async fn process_workspace_index_jobs(
    embedding_client: Arc<dyn EmbeddingClient>,
    workspace_root: &Path,
    max_jobs: usize,
) -> Result<(), StorageError> {
    let limit = max_jobs.clamp(1, 128);
    let jobs = {
        let conn = open_workspace_connection(workspace_root)?;
        let mut stmt = conn.prepare(
            "SELECT mij.id, mij.chunk_id, mc.chunk_text
             FROM memory_index_jobs mij
             JOIN memory_chunks mc ON mc.id = mij.chunk_id
             WHERE mij.status IN ('pending', 'retry')
             ORDER BY mij.scheduled_at ASC, mij.id ASC
             LIMIT ?1",
        )?;
        let mut rows = stmt.query(params![limit as i64])?;
        let mut jobs = Vec::new();
        while let Some(row) = rows.next()? {
            jobs.push((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ));
        }
        jobs
    };

    for (job_id, chunk_id, chunk_text) in jobs {
        let embedding = embedding_client.embed(&chunk_text).await;
        let conn = open_workspace_connection(workspace_root)?;
        match embedding {
            Ok(vector) => {
                conn.execute(
                    "UPDATE memory_index_jobs SET status = 'processing' WHERE id = ?1",
                    params![job_id],
                )?;
                if sqlite_vec_available_on_connection(&conn) {
                    let vector_json =
                        serde_json::to_string(&normalize_embedding_dimension(vector))?;
                    if let Err(err) = conn.execute(
                        "INSERT OR REPLACE INTO memory_chunks_vec(rowid, embedding) VALUES(?1, ?2)",
                        params![chunk_id, vector_json],
                    ) {
                        conn.execute(
                            "UPDATE memory_index_jobs
                             SET status = CASE WHEN retry_count >= 5 THEN 'failed' ELSE 'retry' END,
                                 retry_count = retry_count + 1,
                                 last_error = ?2,
                                 scheduled_at = ?3
                             WHERE id = ?1",
                            params![job_id, err.to_string(), now_unix_ms()],
                        )?;
                        continue;
                    }
                }
                conn.execute(
                    "UPDATE memory_chunks SET status = 'indexed', updated_at = ?2 WHERE id = ?1",
                    params![chunk_id, now_unix_ms()],
                )?;
                conn.execute(
                    "UPDATE memory_index_jobs SET status = 'done', last_error = NULL WHERE id = ?1",
                    params![job_id],
                )?;
            }
            Err(err) => {
                conn.execute(
                    "UPDATE memory_index_jobs
                     SET status = CASE WHEN retry_count >= 5 THEN 'failed' ELSE 'retry' END,
                         retry_count = retry_count + 1,
                         last_error = ?2,
                         scheduled_at = ?3
                     WHERE id = ?1",
                    params![job_id, err.to_string(), now_unix_ms()],
                )?;
            }
        }
    }
    Ok(())
}

fn register_sqlite_vec_auto_extension() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

pub fn sqlite_vec_available() -> bool {
    register_sqlite_vec_auto_extension();
    let Ok(conn) = Connection::open_in_memory() else {
        return false;
    };
    sqlite_vec_available_on_connection(&conn)
}

fn sqlite_vec_available_on_connection(conn: &Connection) -> bool {
    conn.query_row::<String, _, _>("SELECT vec_version()", [], |row| row.get(0))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct MockEmbeddingClient;

    #[async_trait]
    impl EmbeddingClient for MockEmbeddingClient {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, StorageError> {
            Ok(vec![0.01; EMBEDDING_DIMENSION])
        }
    }

    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join("ai-helper-storage-tests")
                .join(format!("{}-{}", name, Uuid::new_v4()));
            fs::create_dir_all(&root).expect("failed to create temp workspace");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn mock_service() -> StorageService {
        StorageService::with_embedding_client(Arc::new(MockEmbeddingClient))
    }

    fn sample_conversation(conversation_id: &str) -> ConversationSnapshot {
        ConversationSnapshot {
            id: conversation_id.to_string(),
            title: "Test Conversation".to_string(),
            pinned: false,
            messages: vec![MessageSnapshot {
                id: "msg-1".to_string(),
                role: "user".to_string(),
                content: "hello world".to_string(),
                images: Vec::new(),
                timestamp: 1_700_000_000_000,
                is_streaming: false,
            }],
            protocol_cards: Vec::new(),
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_100,
        }
    }

    fn sample_image_conversation(conversation_id: &str) -> ConversationSnapshot {
        ConversationSnapshot {
            id: conversation_id.to_string(),
            title: "Image Conversation".to_string(),
            pinned: false,
            messages: vec![MessageSnapshot {
                id: "msg-image-1".to_string(),
                role: "user".to_string(),
                content: "image".to_string(),
                images: vec![InputImageAttachmentSnapshot {
                    id: "img-1".to_string(),
                    name: "test.png".to_string(),
                    mime_type: "image/png".to_string(),
                    data_url: "data:image/png;base64,aGVsbG8=".to_string(),
                    size_bytes: 5,
                }],
                timestamp: 1_700_000_100_000,
                is_streaming: false,
            }],
            protocol_cards: Vec::new(),
            created_at: 1_700_000_100_000,
            updated_at: 1_700_000_100_500,
        }
    }

    #[tokio::test]
    async fn bootstrap_imports_legacy_only_once() {
        let service = mock_service();
        let workspace = TempWorkspace::new("legacy-import");
        let legacy = LegacySessionSnapshot {
            conversations: vec![sample_conversation("conv-legacy-1")],
            current_conversation_id: Some("conv-legacy-1".to_string()),
        };

        let first = service
            .bootstrap_workspace(
                &workspace.root,
                StorageBootstrapRequest {
                    legacy: Some(legacy.clone()),
                },
            )
            .await
            .expect("first bootstrap failed");
        assert!(first.migrated_legacy);
        assert_eq!(first.conversations.len(), 1);
        assert_eq!(
            first.current_conversation_id.as_deref(),
            Some("conv-legacy-1")
        );

        let second = service
            .bootstrap_workspace(
                &workspace.root,
                StorageBootstrapRequest {
                    legacy: Some(legacy),
                },
            )
            .await
            .expect("second bootstrap failed");
        assert!(!second.migrated_legacy);
        assert_eq!(second.conversations.len(), 1);
        assert_eq!(
            second.current_conversation_id.as_deref(),
            Some("conv-legacy-1")
        );
    }

    #[tokio::test]
    async fn workspace_conversations_are_isolated() {
        let service = mock_service();
        let workspace_a = TempWorkspace::new("workspace-a");
        let workspace_b = TempWorkspace::new("workspace-b");

        service
            .upsert_conversation(&workspace_a.root, sample_conversation("conv-a-1"))
            .await
            .expect("failed to upsert workspace A conversation");

        let boot_a = service
            .bootstrap_workspace(&workspace_a.root, StorageBootstrapRequest::default())
            .await
            .expect("failed to bootstrap workspace A");
        let boot_b = service
            .bootstrap_workspace(&workspace_b.root, StorageBootstrapRequest::default())
            .await
            .expect("failed to bootstrap workspace B");

        assert_eq!(boot_a.conversations.len(), 1);
        assert_eq!(boot_a.conversations[0].id, "conv-a-1");
        assert!(boot_b.conversations.is_empty());
    }

    #[tokio::test]
    async fn current_conversation_persists_in_workspace_state() {
        let service = mock_service();
        let workspace = TempWorkspace::new("current-conversation");

        service
            .upsert_conversation(&workspace.root, sample_conversation("conv-current-1"))
            .await
            .expect("failed to upsert conversation");
        service
            .set_current_conversation(&workspace.root, Some("conv-current-1".to_string()))
            .await
            .expect("failed to set current conversation");

        let boot = service
            .bootstrap_workspace(&workspace.root, StorageBootstrapRequest::default())
            .await
            .expect("failed to bootstrap workspace");
        assert_eq!(
            boot.current_conversation_id.as_deref(),
            Some("conv-current-1")
        );
    }

    #[tokio::test]
    async fn image_is_saved_under_workspace_assets_tree() {
        let service = mock_service();
        let workspace = TempWorkspace::new("image-assets");
        let conversation = sample_image_conversation("conv-img-1");

        service
            .upsert_conversation(&workspace.root, conversation.clone())
            .await
            .expect("failed to upsert image conversation");

        let expected = workspace
            .root
            .join(".ah")
            .join("assets")
            .join("conversations")
            .join("conv-img-1")
            .join("msg-image-1")
            .join("img-1.png");
        assert!(
            expected.exists(),
            "expected image file to exist: {:?}",
            expected
        );

        let exported = service
            .export_conversation(&workspace.root, "conv-img-1")
            .await
            .expect("failed to export conversation");
        let image = exported
            .messages
            .first()
            .and_then(|message| message.images.first())
            .expect("expected exported image");
        assert!(image.data_url.starts_with("data:image/png;base64,"));
    }
}
