use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use chrono::Utc;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::{rngs::OsRng, RngCore};
use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub const PROTOCOL_VERSION: u32 = 2;
// Keep the original network root so existing Workmsg/Trixy identities and workspaces
// continue to sync after the product rename.
pub const FIREBASE_ROOT: &str = "workmsg_v1";
pub const LEGACY_ATTACHMENT_CHUNK_SIZE: usize = 512 * 1024;
pub const ATTACHMENT_CHUNK_SIZE: usize = 32 * 1024;
pub const ATTACHMENT_STORAGE_VERSION: u8 = 2;
pub const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;
const ATTACHMENT_RETRY_ATTEMPTS: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub user_id: String,
    pub name: String,
    pub firebase_url: String,
    pub network_id: String,
    pub mailbox_id: String,
    pub sign_secret_b64: String,
    pub sign_public_b64: String,
    pub box_secret_b64: String,
    pub box_public_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicUser {
    pub user_id: String,
    pub name: String,
    pub network_id: String,
    pub mailbox_id: String,
    pub sign_public_b64: String,
    pub box_public_b64: String,
}

#[derive(Clone, Debug)]
pub struct NetworkSummary {
    pub network_id: String,
    pub label: String,
    pub firebase_url: String,
}

#[derive(Clone, Debug)]
pub struct ContactSummary {
    pub user_id: String,
    pub name: String,
    pub routes: Vec<NetworkSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContactInvite {
    pub version: u32,
    pub firebase_url: String,
    #[serde(default)]
    pub network_label: String,
    pub user: PublicUser,
}

#[derive(Clone, Debug)]
pub struct WorkspaceSummary {
    pub id: String,
    pub name: String,
    pub network_id: String,
    pub network_label: String,
}

#[derive(Clone, Debug)]
pub struct MessageView {
    pub id: String,
    pub workspace_id: String,
    pub author_id: String,
    pub author_name: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub body: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub id: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256_b64: String,
    pub remote_token: String,
    pub chunk_count: u32,
    #[serde(default = "legacy_attachment_chunk_size")]
    pub chunk_size: u32,
    #[serde(default = "legacy_attachment_storage_version")]
    pub storage_version: u8,
}

#[derive(Clone, Debug)]
pub struct AttachmentView {
    pub id: String,
    pub workspace_id: String,
    pub message_id: String,
    pub author_id: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256_b64: String,
    pub remote_token: String,
    pub chunk_count: u32,
    pub chunk_size: u32,
    pub storage_version: u8,
    pub local_path: Option<PathBuf>,
    pub upload_pending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AttachmentChunk {
    nonce_b64: String,
    ciphertext_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceAction {
    MemberAdded { user: PublicUser },
    MemberJoined { user: PublicUser, permit: SignedJoinPermit },
    MessageCreated {
        message_id: String,
        body: String,
        #[serde(default)]
        attachments: Vec<AttachmentMeta>,
    },
    MessageEdited { message_id: String, body: String },
    MessageDeleted { message_id: String },
}

impl WorkspaceAction {
    fn kind(&self) -> &'static str {
        match self {
            Self::MemberAdded { .. } => "member_added",
            Self::MemberJoined { .. } => "member_joined",
            Self::MessageCreated { .. } => "message_created",
            Self::MessageEdited { .. } => "message_edited",
            Self::MessageDeleted { .. } => "message_deleted",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedEvent {
    pub version: u32,
    pub id: String,
    pub workspace_id: String,
    pub author_id: String,
    pub seq: i64,
    pub created_at: String,
    pub kind: String,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
    pub signature_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteBody {
    pub version: u32,
    pub created_at: String,
    pub network_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_key_b64: String,
    pub members: Vec<PublicUser>,
    pub events: Vec<SignedEvent>,
    pub inviter: PublicUser,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedInvite {
    pub body: InviteBody,
    pub signature_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinPermitBody {
    pub version: u32,
    pub created_at: String,
    pub workspace_id: String,
    pub network_id: String,
    pub share_id: String,
    pub inviter: PublicUser,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedJoinPermit {
    pub body: JoinPermitBody,
    pub signature_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceSharePackage {
    pub invite: SignedInvite,
    pub permit: SignedJoinPermit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncryptedWorkspaceShare {
    pub nonce_b64: String,
    pub ciphertext_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceShareLink {
    pub version: u32,
    pub firebase_url: String,
    #[serde(default)]
    pub network_label: String,
    pub workspace_id: String,
    pub token: String,
    pub key_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SealedInvite {
    pub sender_box_public_b64: String,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "packet_type", rename_all = "snake_case")]
pub enum TransportPacket {
    Event { event: SignedEvent },
    WorkspaceInvite { sealed: SealedInvite },
}

#[derive(Clone, Debug)]
pub struct MessageAlert {
    pub workspace_id: String,
    pub workspace_name: String,
    pub author_name: String,
    pub body: String,
    pub has_attachments: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SyncReport {
    pub sent: usize,
    pub received: usize,
    pub alerts: Vec<MessageAlert>,
    pub errors: Vec<String>,
}

#[derive(Clone)]
pub struct AppDb {
    path: PathBuf,
}

fn legacy_attachment_chunk_size() -> u32 {
    LEGACY_ATTACHMENT_CHUNK_SIZE as u32
}

fn legacy_attachment_storage_version() -> u8 {
    1
}

fn ensure_attachment_columns(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(attachments)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row?);
    }
    drop(stmt);

    if !columns.contains("chunk_size") {
        conn.execute(
            "ALTER TABLE attachments ADD COLUMN chunk_size INTEGER NOT NULL DEFAULT 524288",
            [],
        )?;
    }
    if !columns.contains("storage_version") {
        conn.execute(
            "ALTER TABLE attachments ADD COLUMN storage_version INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row?);
    }
    Ok(columns)
}

fn ensure_v06_schema(conn: &Connection) -> Result<()> {
    let workspace_columns = table_columns(conn, "workspaces")?;
    if !workspace_columns.contains("network_id") {
        conn.execute(
            "ALTER TABLE workspaces ADD COLUMN network_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

    let outbox_columns = table_columns(conn, "outbox")?;
    if !outbox_columns.contains("network_id") {
        conn.execute(
            "ALTER TABLE outbox ADD COLUMN network_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

    let identity_json: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key='identity'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(json) = identity_json {
        if let Ok(identity) = serde_json::from_str::<Identity>(&json) {
            let label = firebase_label(&identity.firebase_url);
            conn.execute(
                r#"INSERT INTO networks(network_id,label,firebase_url,created_at)
                   VALUES(?1,?2,?3,?4)
                   ON CONFLICT(network_id) DO UPDATE SET firebase_url=excluded.firebase_url"#,
                params![
                    identity.network_id,
                    label,
                    identity.firebase_url,
                    Utc::now().to_rfc3339()
                ],
            )?;
            conn.execute(
                "UPDATE workspaces SET network_id=?1 WHERE network_id=''",
                params![identity.network_id],
            )?;
            conn.execute(
                "UPDATE outbox SET network_id=?1 WHERE network_id=''",
                params![identity.network_id],
            )?;
        }
    }

    conn.execute(
        r#"INSERT OR IGNORE INTO contact_routes(user_id,network_id,mailbox_id)
           SELECT user_id,network_id,mailbox_id FROM contacts
           WHERE network_id<>'' AND mailbox_id<>''"#,
        [],
    )?;
    Ok(())
}

impl AppDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let db = Self { path };
        db.init_schema()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(conn)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS networks (
                network_id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                firebase_url TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS contacts (
                user_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                network_id TEXT NOT NULL,
                mailbox_id TEXT NOT NULL,
                sign_public_b64 TEXT NOT NULL,
                box_public_b64 TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS contact_routes (
                user_id TEXT NOT NULL,
                network_id TEXT NOT NULL,
                mailbox_id TEXT NOT NULL,
                PRIMARY KEY (user_id, network_id)
            );

            CREATE TABLE IF NOT EXISTS workspaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                workspace_key_b64 TEXT NOT NULL,
                created_at TEXT NOT NULL,
                network_id TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS members (
                workspace_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                name TEXT NOT NULL,
                network_id TEXT NOT NULL,
                mailbox_id TEXT NOT NULL,
                sign_public_b64 TEXT NOT NULL,
                box_public_b64 TEXT NOT NULL,
                PRIMARY KEY (workspace_id, user_id)
            );

            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                workspace_id TEXT NOT NULL,
                author_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                kind TEXT NOT NULL,
                nonce_b64 TEXT NOT NULL,
                ciphertext_b64 TEXT NOT NULL,
                signature_b64 TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_workspace
                ON events(workspace_id, created_at, id);

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                author_id TEXT NOT NULL,
                author_name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                edited_at TEXT,
                body TEXT NOT NULL,
                deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_messages_workspace
                ON messages(workspace_id, created_at, id);

            CREATE TABLE IF NOT EXISTS attachments (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                author_id TEXT NOT NULL,
                file_name TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                sha256_b64 TEXT NOT NULL,
                remote_token TEXT NOT NULL,
                chunk_count INTEGER NOT NULL,
                chunk_size INTEGER NOT NULL DEFAULT 524288,
                storage_version INTEGER NOT NULL DEFAULT 1,
                local_path TEXT,
                upload_pending INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_attachments_message
                ON attachments(message_id, id);

            CREATE TABLE IF NOT EXISTS outbox (
                packet_id TEXT PRIMARY KEY,
                network_id TEXT NOT NULL DEFAULT '',
                recipient_mailbox_id TEXT NOT NULL,
                packet_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS inbox_seen (
                packet_id TEXT PRIMARY KEY,
                received_at TEXT NOT NULL
            );
            "#,
        )?;
        ensure_attachment_columns(&conn)?;
        ensure_v06_schema(&conn)?;
        Ok(())
    }

    pub fn has_identity(&self) -> Result<bool> {
        let conn = self.conn()?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key='identity'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.is_some())
    }

    pub fn create_identity(&self, name: &str, firebase_url: &str) -> Result<Identity> {
        if self.has_identity()? {
            bail!("Identity already exists");
        }
        let name = name.trim();
        if name.is_empty() {
            bail!("Display name cannot be empty");
        }
        let firebase_url = normalize_firebase_url(firebase_url)?;

        let mut sign_secret = [0u8; 32];
        OsRng.fill_bytes(&mut sign_secret);
        let signing_key = SigningKey::from_bytes(&sign_secret);

        let mut box_secret = [0u8; 32];
        OsRng.fill_bytes(&mut box_secret);
        let box_secret_key = StaticSecret::from(box_secret);
        let box_public = X25519PublicKey::from(&box_secret_key);

        let mut mailbox = [0u8; 32];
        OsRng.fill_bytes(&mut mailbox);

        let identity = Identity {
            user_id: Uuid::now_v7().to_string(),
            name: name.to_string(),
            network_id: network_id_for_url(&firebase_url),
            firebase_url,
            mailbox_id: b64(&mailbox),
            sign_secret_b64: b64(&sign_secret),
            sign_public_b64: b64(&signing_key.verifying_key().to_bytes()),
            box_secret_b64: b64(&box_secret),
            box_public_b64: b64(box_public.as_bytes()),
        };

        self.conn()?.execute(
            "INSERT INTO meta(key,value) VALUES('identity',?1)",
            params![serde_json::to_string(&identity)?],
        )?;
        self.ensure_network(&identity.firebase_url, Some("Primary"))?;
        self.upsert_contact(&identity.public_user())?;
        Ok(identity)
    }

    pub fn identity(&self) -> Result<Identity> {
        let json: String = self
            .conn()?
            .query_row(
                "SELECT value FROM meta WHERE key='identity'",
                [],
                |row| row.get(0),
            )
            .context("No Trixy identity has been created")?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn networks(&self) -> Result<Vec<NetworkSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT network_id,label,firebase_url FROM networks ORDER BY lower(label),network_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NetworkSummary {
                network_id: row.get(0)?,
                label: row.get(1)?,
                firebase_url: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn add_network(&self, label: &str, firebase_url: &str) -> Result<NetworkSummary> {
        self.ensure_network(firebase_url, Some(label))
    }

    fn ensure_network(&self, firebase_url: &str, label: Option<&str>) -> Result<NetworkSummary> {
        let firebase_url = normalize_firebase_url(firebase_url)?;
        let network_id = network_id_for_url(&firebase_url);
        let requested_label = label.map(str::trim).filter(|s| !s.is_empty());
        let display_label = requested_label
            .map(str::to_string)
            .unwrap_or_else(|| firebase_label(&firebase_url));
        self.conn()?.execute(
            r#"INSERT INTO networks(network_id,label,firebase_url,created_at)
               VALUES(?1,?2,?3,?4)
               ON CONFLICT(network_id) DO UPDATE SET
                 firebase_url=excluded.firebase_url,
                 label=CASE WHEN ?5<>'' THEN ?5 ELSE networks.label END"#,
            params![
                network_id,
                display_label,
                firebase_url,
                Utc::now().to_rfc3339(),
                requested_label.unwrap_or("")
            ],
        )?;
        self.network_by_id(&network_id)?.context("Network was not saved")
    }

    pub fn network_by_id(&self, network_id: &str) -> Result<Option<NetworkSummary>> {
        self.conn()?
            .query_row(
                "SELECT network_id,label,firebase_url FROM networks WHERE network_id=?1",
                params![network_id],
                |row| {
                    Ok(NetworkSummary {
                        network_id: row.get(0)?,
                        label: row.get(1)?,
                        firebase_url: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn network_url(&self, network_id: &str) -> Result<String> {
        Ok(self
            .network_by_id(network_id)?
            .with_context(|| format!("Firebase connection {} is not configured", short_id(network_id)))?
            .firebase_url)
    }

    pub fn default_network_id(&self) -> Result<String> {
        let identity = self.identity()?;
        if self.network_by_id(&identity.network_id)?.is_some() {
            return Ok(identity.network_id);
        }
        self.networks()?
            .first()
            .map(|network| network.network_id.clone())
            .context("No Firebase connections are configured")
    }

    pub fn contact_invite_code(&self) -> Result<String> {
        let network_id = self.default_network_id()?;
        self.contact_invite_code_for_network(&network_id)
    }

    pub fn contact_invite_code_for_network(&self, network_id: &str) -> Result<String> {
        let network = self
            .network_by_id(network_id)?
            .context("Firebase connection not found")?;
        let user = self.identity()?.public_user_for(network_id);
        let invite = ContactInvite {
            version: PROTOCOL_VERSION,
            firebase_url: network.firebase_url,
            network_label: network.label,
            user,
        };
        Ok(format!(
            "TRIXY-CONTACT2-{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&invite)?)
        ))
    }

    pub fn import_contact_code(&self, code: &str) -> Result<PublicUser> {
        let trimmed = code.trim();
        if let Some(encoded) = trimmed.strip_prefix("TRIXY-CONTACT2-") {
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .context("Invalid Trixy contact code")?;
            let invite: ContactInvite =
                serde_json::from_slice(&bytes).context("Invalid Trixy contact payload")?;
            if invite.version != PROTOCOL_VERSION {
                bail!("Unsupported Trixy contact code version");
            }
            let network = self.ensure_network(
                &invite.firebase_url,
                if invite.network_label.trim().is_empty() {
                    None
                } else {
                    Some(invite.network_label.as_str())
                },
            )?;
            if invite.user.network_id != network.network_id {
                bail!("Contact code Firebase connection does not match its user route");
            }
            self.finish_contact_import(invite.user)
        } else {
            let encoded = trimmed
                .strip_prefix("TRIXY-CONTACT-")
                .or_else(|| trimmed.strip_prefix("WORKMSG-CONTACT-"))
                .unwrap_or(trimmed);
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .context("Invalid Trixy contact code")?;
            let user: PublicUser =
                serde_json::from_slice(&bytes).context("Invalid Trixy contact payload")?;
            if self.network_by_id(&user.network_id)?.is_none() {
                bail!("This older contact code uses a Firebase connection you have not added yet");
            }
            self.finish_contact_import(user)
        }
    }

    fn finish_contact_import(&self, user: PublicUser) -> Result<PublicUser> {
        validate_public_user(&user)?;
        let me = self.identity()?;
        if user.user_id == me.user_id {
            bail!("That is your own contact code");
        }
        self.upsert_contact(&user)?;
        Ok(user)
    }

    pub fn contacts(&self) -> Result<Vec<PublicUser>> {
        let mut out = Vec::new();
        for network in self.networks()? {
            out.extend(self.contacts_for_network(&network.network_id)?);
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.network_id.cmp(&b.network_id)));
        Ok(out)
    }

    pub fn contacts_for_network(&self, network_id: &str) -> Result<Vec<PublicUser>> {
        let me = self.identity().ok().map(|i| i.user_id);
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT c.user_id,c.name,r.network_id,r.mailbox_id,c.sign_public_b64,c.box_public_b64
               FROM contacts c
               JOIN contact_routes r ON r.user_id=c.user_id
               WHERE r.network_id=?1
               ORDER BY lower(c.name),c.user_id"#,
        )?;
        let rows = stmt.query_map(params![network_id], row_to_public_user)?;
        let mut users = Vec::new();
        for row in rows {
            let user = row?;
            if me.as_deref() != Some(user.user_id.as_str()) {
                users.push(user);
            }
        }
        Ok(users)
    }

    pub fn contact_summaries(&self) -> Result<Vec<ContactSummary>> {
        let me = self.identity().ok().map(|i| i.user_id);
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT c.user_id,c.name,n.network_id,n.label,n.firebase_url
               FROM contacts c
               LEFT JOIN contact_routes r ON r.user_id=c.user_id
               LEFT JOIN networks n ON n.network_id=r.network_id
               ORDER BY lower(c.name),c.user_id,lower(n.label)"#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut grouped: Vec<ContactSummary> = Vec::new();
        for row in rows {
            let (user_id, name, network_id, label, firebase_url) = row?;
            if me.as_deref() == Some(user_id.as_str()) {
                continue;
            }
            let index = grouped.iter().position(|item| item.user_id == user_id);
            let idx = if let Some(index) = index {
                index
            } else {
                grouped.push(ContactSummary {
                    user_id: user_id.clone(),
                    name: name.clone(),
                    routes: Vec::new(),
                });
                grouped.len() - 1
            };
            if let (Some(network_id), Some(label), Some(firebase_url)) = (network_id, label, firebase_url) {
                grouped[idx].routes.push(NetworkSummary {
                    network_id,
                    label,
                    firebase_url,
                });
            }
        }
        Ok(grouped)
    }

    fn upsert_contact(&self, user: &PublicUser) -> Result<()> {
        validate_public_user(user)?;
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO contacts(user_id,name,network_id,mailbox_id,sign_public_b64,box_public_b64)
               VALUES(?1,?2,?3,?4,?5,?6)
               ON CONFLICT(user_id) DO UPDATE SET
                 name=excluded.name,
                 network_id=excluded.network_id,
                 mailbox_id=excluded.mailbox_id,
                 sign_public_b64=excluded.sign_public_b64,
                 box_public_b64=excluded.box_public_b64"#,
            params![
                user.user_id,
                user.name,
                user.network_id,
                user.mailbox_id,
                user.sign_public_b64,
                user.box_public_b64
            ],
        )?;
        conn.execute(
            r#"INSERT INTO contact_routes(user_id,network_id,mailbox_id)
               VALUES(?1,?2,?3)
               ON CONFLICT(user_id,network_id) DO UPDATE SET mailbox_id=excluded.mailbox_id"#,
            params![user.user_id, user.network_id, user.mailbox_id],
        )?;
        Ok(())
    }

    pub fn create_workspace(&self, name: &str) -> Result<String> {
        let network_id = self.default_network_id()?;
        self.create_workspace_on_network(name, &network_id)
    }

    pub fn create_workspace_on_network(&self, name: &str, network_id: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Workspace name cannot be empty");
        }
        self.network_by_id(network_id)?.context("Firebase connection not found")?;
        let identity = self.identity()?;
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let id = Uuid::now_v7().to_string();
        let created_at = Utc::now().to_rfc3339();
        self.conn()?.execute(
            "INSERT INTO workspaces(id,name,workspace_key_b64,created_at,network_id) VALUES(?1,?2,?3,?4,?5)",
            params![id, name, b64(&key), created_at, network_id],
        )?;
        self.upsert_member(&id, &identity.public_user_for(network_id))?;
        Ok(id)
    }

    pub fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"SELECT w.id,w.name,w.network_id,COALESCE(n.label,'Firebase')
               FROM workspaces w LEFT JOIN networks n ON n.network_id=w.network_id
               ORDER BY lower(w.name),w.id"#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkspaceSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                network_id: row.get(2)?,
                network_label: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn workspace_name(&self, workspace_id: &str) -> Result<String> {
        Ok(self.conn()?.query_row(
            "SELECT name FROM workspaces WHERE id=?1",
            params![workspace_id],
            |row| row.get(0),
        )?)
    }

    pub fn workspace_network_id(&self, workspace_id: &str) -> Result<String> {
        Ok(self.conn()?.query_row(
            "SELECT network_id FROM workspaces WHERE id=?1",
            params![workspace_id],
            |row| row.get(0),
        )?)
    }

    pub fn workspace_network(&self, workspace_id: &str) -> Result<NetworkSummary> {
        let network_id = self.workspace_network_id(workspace_id)?;
        self.network_by_id(&network_id)?.context("Workspace Firebase connection is not configured")
    }

    pub fn members(&self, workspace_id: &str) -> Result<Vec<PublicUser>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT user_id,name,network_id,mailbox_id,sign_public_b64,box_public_b64
             FROM members WHERE workspace_id=?1 ORDER BY lower(name),user_id",
        )?;
        let rows = stmt.query_map(params![workspace_id], row_to_public_user)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn member_by_id(&self, workspace_id: &str, user_id: &str) -> Result<Option<PublicUser>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT user_id,name,network_id,mailbox_id,sign_public_b64,box_public_b64
             FROM members WHERE workspace_id=?1 AND user_id=?2",
            params![workspace_id, user_id],
            row_to_public_user,
        )
        .optional()
        .map_err(Into::into)
    }

    fn upsert_member(&self, workspace_id: &str, user: &PublicUser) -> Result<()> {
        validate_public_user(user)?;
        let workspace_network_id = self.workspace_network_id(workspace_id)?;
        if user.network_id != workspace_network_id {
            bail!("Member route does not match the workspace Firebase connection");
        }
        self.conn()?.execute(
            r#"INSERT INTO members(workspace_id,user_id,name,network_id,mailbox_id,sign_public_b64,box_public_b64)
               VALUES(?1,?2,?3,?4,?5,?6,?7)
               ON CONFLICT(workspace_id,user_id) DO UPDATE SET
                 name=excluded.name,
                 network_id=excluded.network_id,
                 mailbox_id=excluded.mailbox_id,
                 sign_public_b64=excluded.sign_public_b64,
                 box_public_b64=excluded.box_public_b64"#,
            params![
                workspace_id,
                user.user_id,
                user.name,
                user.network_id,
                user.mailbox_id,
                user.sign_public_b64,
                user.box_public_b64
            ],
        )?;
        Ok(())
    }

    pub fn add_member(&self, workspace_id: &str, user_id: &str) -> Result<()> {
        let network_id = self.workspace_network_id(workspace_id)?;
        let contact = self
            .contact_by_id_on_network(user_id, &network_id)?
            .context("That contact is not connected to this workspace's Firebase database")?;
        if self.member_by_id(workspace_id, user_id)?.is_some() {
            bail!("That person is already in the workspace");
        }

        let previous_members = self.members(workspace_id)?;
        let action = WorkspaceAction::MemberAdded {
            user: contact.clone(),
        };
        let event = self.create_signed_event(workspace_id, action)?;
        self.store_and_apply_event(&event)?;
        let me = self.identity()?;

        for member in previous_members {
            if member.user_id != me.user_id {
                self.queue_packet(
                    &network_id,
                    &member.mailbox_id,
                    &TransportPacket::Event {
                        event: event.clone(),
                    },
                )?;
            }
        }

        let sealed = self.build_sealed_invite(workspace_id, &contact)?;
        self.queue_packet(
            &network_id,
            &contact.mailbox_id,
            &TransportPacket::WorkspaceInvite { sealed },
        )?;
        Ok(())
    }

    fn contact_by_id_on_network(&self, user_id: &str, network_id: &str) -> Result<Option<PublicUser>> {
        self.conn()?
            .query_row(
                r#"SELECT c.user_id,c.name,r.network_id,r.mailbox_id,c.sign_public_b64,c.box_public_b64
                   FROM contacts c JOIN contact_routes r ON r.user_id=c.user_id
                   WHERE c.user_id=?1 AND r.network_id=?2"#,
                params![user_id, network_id],
                row_to_public_user,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn send_message(&self, workspace_id: &str, body: &str) -> Result<String> {
        self.send_message_with_files(workspace_id, body, &[])
    }

    pub fn send_message_with_files(
        &self,
        workspace_id: &str,
        body: &str,
        files: &[PathBuf],
    ) -> Result<String> {
        let body = body.trim_end();
        if body.trim().is_empty() && files.is_empty() {
            bail!("Message cannot be empty");
        }

        let message_id = Uuid::now_v7().to_string();
        let mut staged = Vec::new();
        for file in files {
            staged.push(self.stage_attachment(workspace_id, &message_id, file)?);
        }
        let attachments = staged
            .iter()
            .map(|(meta, _)| meta.clone())
            .collect::<Vec<_>>();

        let event = self.create_signed_event(
            workspace_id,
            WorkspaceAction::MessageCreated {
                message_id: message_id.clone(),
                body: body.to_string(),
                attachments,
            },
        )?;
        self.store_and_apply_event(&event)?;

        let conn = self.conn()?;
        for (meta, local_path) in staged {
            conn.execute(
                "UPDATE attachments SET local_path=?1,upload_pending=1 WHERE id=?2",
                params![local_path.to_string_lossy().to_string(), meta.id],
            )?;
        }

        self.broadcast_event(workspace_id, &event)?;
        Ok(message_id)
    }

    pub fn edit_message(&self, workspace_id: &str, message_id: &str, body: &str) -> Result<()> {
        let body = body.trim();
        if body.is_empty() {
            bail!("Message cannot be empty");
        }
        self.ensure_own_message(workspace_id, message_id)?;
        let event = self.create_signed_event(
            workspace_id,
            WorkspaceAction::MessageEdited {
                message_id: message_id.to_string(),
                body: body.to_string(),
            },
        )?;
        self.store_and_apply_event(&event)?;
        self.broadcast_event(workspace_id, &event)?;
        Ok(())
    }

    pub fn delete_message(&self, workspace_id: &str, message_id: &str) -> Result<()> {
        self.ensure_own_message(workspace_id, message_id)?;
        let event = self.create_signed_event(
            workspace_id,
            WorkspaceAction::MessageDeleted {
                message_id: message_id.to_string(),
            },
        )?;
        self.store_and_apply_event(&event)?;
        self.broadcast_event(workspace_id, &event)?;
        Ok(())
    }

    fn ensure_own_message(&self, workspace_id: &str, message_id: &str) -> Result<()> {
        let me = self.identity()?;
        let author: Option<String> = self
            .conn()?
            .query_row(
                "SELECT author_id FROM messages WHERE id=?1 AND workspace_id=?2",
                params![message_id, workspace_id],
                |row| row.get(0),
            )
            .optional()?;
        match author {
            Some(author) if author == me.user_id => Ok(()),
            Some(_) => bail!("You can only modify your own messages"),
            None => bail!("Message not found"),
        }
    }

    pub fn messages(&self, workspace_id: &str) -> Result<Vec<MessageView>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,workspace_id,author_id,author_name,created_at,edited_at,body,deleted
             FROM messages WHERE workspace_id=?1 ORDER BY created_at,id",
        )?;
        let rows = stmt.query_map(params![workspace_id], |row| {
            Ok(MessageView {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                author_id: row.get(2)?,
                author_name: row.get(3)?,
                created_at: row.get(4)?,
                edited_at: row.get(5)?,
                body: row.get(6)?,
                deleted: row.get::<_, i64>(7)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn attachments_for_message(&self, message_id: &str) -> Result<Vec<AttachmentView>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,workspace_id,message_id,author_id,file_name,size_bytes,sha256_b64,remote_token,chunk_count,chunk_size,storage_version,local_path,upload_pending
             FROM attachments WHERE message_id=?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![message_id], row_to_attachment_view)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn attachment_by_id(&self, attachment_id: &str) -> Result<AttachmentView> {
        self.conn()?
            .query_row(
                "SELECT id,workspace_id,message_id,author_id,file_name,size_bytes,sha256_b64,remote_token,chunk_count,chunk_size,storage_version,local_path,upload_pending
                 FROM attachments WHERE id=?1",
                params![attachment_id],
                row_to_attachment_view,
            )
            .context("Attachment not found")
    }

    fn attachments_dir(&self) -> Result<PathBuf> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        let stem = self
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("trixy");
        let dir = parent.join(format!("{stem}-files"));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn stage_attachment(
        &self,
        workspace_id: &str,
        message_id: &str,
        source: &Path,
    ) -> Result<(AttachmentMeta, PathBuf)> {
        let metadata = fs::metadata(source)
            .with_context(|| format!("Could not read attachment {}", source.display()))?;
        if !metadata.is_file() {
            bail!("Attachments must be files");
        }
        if metadata.len() > MAX_ATTACHMENT_BYTES {
            bail!(
                "Attachment {} is larger than the {} MB test limit",
                source.display(),
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            );
        }
        let file_name = source
            .file_name()
            .and_then(|s| s.to_str())
            .context("Attachment filename is not valid UTF-8")?
            .to_string();
        if file_name.trim().is_empty() {
            bail!("Attachment filename cannot be empty");
        }

        let id = Uuid::now_v7().to_string();
        let remote_token = b64(&random_32());
        let sha256_b64 = hash_file_b64(source)?;
        let size_bytes = metadata.len();
        let chunk_size = ATTACHMENT_CHUNK_SIZE as u32;
        let chunk_count = std::cmp::max(
            1,
            ((size_bytes + chunk_size as u64 - 1) / chunk_size as u64) as u32,
        );
        let safe_name = safe_file_name(&file_name);
        let local_path = self
            .attachments_dir()?
            .join(format!("{id}-{safe_name}"));
        fs::copy(source, &local_path).with_context(|| {
            format!(
                "Could not copy attachment into Trixy storage: {}",
                source.display()
            )
        })?;

        let meta = AttachmentMeta {
            id,
            file_name,
            size_bytes,
            sha256_b64,
            remote_token,
            chunk_count,
            chunk_size,
            storage_version: ATTACHMENT_STORAGE_VERSION,
        };
        validate_attachment_meta(&meta)?;

        // The event application creates the attachment row. Keep the workspace/message
        // arguments here so the caller is forced to stage against the same message.
        let _ = (workspace_id, message_id);
        Ok((meta, local_path))
    }

    fn pending_attachment_uploads(&self) -> Result<Vec<AttachmentView>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,workspace_id,message_id,author_id,file_name,size_bytes,sha256_b64,remote_token,chunk_count,chunk_size,storage_version,local_path,upload_pending
             FROM attachments WHERE upload_pending=1 AND local_path IS NOT NULL ORDER BY created_at,id",
        )?;
        let rows = stmt.query_map([], row_to_attachment_view)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn mark_attachment_uploaded(&self, attachment_id: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE attachments SET upload_pending=0 WHERE id=?1",
            params![attachment_id],
        )?;
        Ok(())
    }

    pub fn download_attachment(&self, attachment_id: &str) -> Result<PathBuf> {
        let attachment = self.attachment_by_id(attachment_id)?;
        if let Some(path) = &attachment.local_path {
            if path.exists() {
                return Ok(path.clone());
            }
        }
        validate_attachment_view(&attachment)?;
        let firebase_url = self.workspace_network(&attachment.workspace_id)?.firebase_url;
        let key = self.workspace_key(&attachment.workspace_id)?;
        let client = http_client()?;
        let safe_name = safe_file_name(&attachment.file_name);
        let final_path = self
            .attachments_dir()?
            .join(format!("{}-{safe_name}", attachment.id));
        let temp_path = final_path.with_extension("trixy-part");
        let mut output = File::create(&temp_path)?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;

        for index in 0..attachment.chunk_count {
            let chunk = download_attachment_chunk(
                &client,
                &firebase_url,
                &attachment.remote_token,
                attachment.storage_version,
                index,
            )
            .with_context(|| format!("Could not download file chunk {}/{}", index + 1, attachment.chunk_count))?;
            let plaintext = decrypt(
                &key,
                &unb64(&chunk.nonce_b64)?,
                &unb64(&chunk.ciphertext_b64)?,
            )?;
            if plaintext.len() > attachment.chunk_size as usize {
                let _ = fs::remove_file(&temp_path);
                bail!("Attachment chunk is larger than the signed chunk size");
            }
            written += plaintext.len() as u64;
            if written > attachment.size_bytes {
                let _ = fs::remove_file(&temp_path);
                bail!("Attachment is larger than the signed metadata");
            }
            hasher.update(&plaintext);
            output.write_all(&plaintext)?;
        }
        output.flush()?;
        drop(output);

        if written != attachment.size_bytes {
            let _ = fs::remove_file(&temp_path);
            bail!("Attachment size check failed");
        }
        let digest = hasher.finalize();
        if b64(&digest) != attachment.sha256_b64 {
            let _ = fs::remove_file(&temp_path);
            bail!("Attachment integrity check failed");
        }
        fs::rename(&temp_path, &final_path)?;
        self.conn()?.execute(
            "UPDATE attachments SET local_path=?1 WHERE id=?2",
            params![final_path.to_string_lossy().to_string(), attachment.id],
        )?;
        Ok(final_path)
    }

    fn workspace_key(&self, workspace_id: &str) -> Result<[u8; 32]> {
        let encoded: String = self.conn()?.query_row(
            "SELECT workspace_key_b64 FROM workspaces WHERE id=?1",
            params![workspace_id],
            |row| row.get(0),
        )?;
        to_32(&unb64(&encoded)?)
    }

    fn next_seq(&self, workspace_id: &str, author_id: &str) -> Result<i64> {
        Ok(self.conn()?.query_row(
            "SELECT COALESCE(MAX(seq),0)+1 FROM events WHERE workspace_id=?1 AND author_id=?2",
            params![workspace_id, author_id],
            |row| row.get(0),
        )?)
    }

    fn create_signed_event(&self, workspace_id: &str, action: WorkspaceAction) -> Result<SignedEvent> {
        let identity = self.identity()?;
        if self.member_by_id(workspace_id, &identity.user_id)?.is_none() {
            bail!("You are not a member of this workspace");
        }
        let key = self.workspace_key(workspace_id)?;
        let plaintext = serde_json::to_vec(&action)?;
        let (nonce, ciphertext) = encrypt(&key, &plaintext)?;
        let mut event = SignedEvent {
            version: PROTOCOL_VERSION,
            id: Uuid::now_v7().to_string(),
            workspace_id: workspace_id.to_string(),
            author_id: identity.user_id.clone(),
            seq: self.next_seq(workspace_id, &identity.user_id)?,
            created_at: Utc::now().to_rfc3339(),
            kind: action.kind().to_string(),
            nonce_b64: b64(&nonce),
            ciphertext_b64: b64(&ciphertext),
            signature_b64: String::new(),
        };
        event.signature_b64 = sign_event(&event, &identity)?;
        Ok(event)
    }

    fn broadcast_event(&self, workspace_id: &str, event: &SignedEvent) -> Result<()> {
        let me = self.identity()?;
        let network_id = self.workspace_network_id(workspace_id)?;
        for member in self.members(workspace_id)? {
            if member.user_id != me.user_id {
                self.queue_packet(
                    &network_id,
                    &member.mailbox_id,
                    &TransportPacket::Event {
                        event: event.clone(),
                    },
                )?;
            }
        }
        Ok(())
    }

    fn queue_packet(
        &self,
        network_id: &str,
        recipient_mailbox_id: &str,
        packet: &TransportPacket,
    ) -> Result<()> {
        validate_firebase_key(recipient_mailbox_id)?;
        self.network_by_id(network_id)?.context("Firebase connection not found")?;
        let packet_id = Uuid::now_v7().to_string();
        self.conn()?.execute(
            "INSERT INTO outbox(packet_id,network_id,recipient_mailbox_id,packet_json,created_at)
             VALUES(?1,?2,?3,?4,?5)",
            params![
                packet_id,
                network_id,
                recipient_mailbox_id,
                serde_json::to_string(packet)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    fn outbox_rows(&self) -> Result<Vec<(String, String, String, TransportPacket)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT packet_id,network_id,recipient_mailbox_id,packet_json FROM outbox ORDER BY packet_id LIMIT 500",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (packet_id, network_id, mailbox_id, json) = row?;
            out.push((packet_id, network_id, mailbox_id, serde_json::from_str(&json)?));
        }
        Ok(out)
    }

    fn delete_outbox(&self, packet_id: &str) -> Result<()> {
        self.conn()?.execute(
            "DELETE FROM outbox WHERE packet_id=?1",
            params![packet_id],
        )?;
        Ok(())
    }

    pub fn pending_outbox_count(&self) -> Result<i64> {
        Ok(self
            .conn()?
            .query_row("SELECT COUNT(*) FROM outbox", [], |row| row.get(0))?)
    }

    fn seen_packet(&self, packet_id: &str) -> Result<bool> {
        let exists: Option<String> = self
            .conn()?
            .query_row(
                "SELECT packet_id FROM inbox_seen WHERE packet_id=?1",
                params![packet_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    fn mark_packet_seen(&self, packet_id: &str) -> Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO inbox_seen(packet_id,received_at) VALUES(?1,?2)",
            params![packet_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn process_packet(&self, packet_id: &str, packet: &TransportPacket) -> Result<bool> {
        if self.seen_packet(packet_id)? {
            return Ok(false);
        }
        let changed = match packet {
            TransportPacket::Event { event } => self.store_and_apply_event(event)?,
            TransportPacket::WorkspaceInvite { sealed } => {
                self.import_sealed_invite(sealed)?;
                true
            }
        };

        // Membership changes are gossiped once by every client that sees them for
        // the first time. That lets several people join from the same org link and
        // still converge on the complete member/contact directory even when those
        // joiners were not present in the original link snapshot. Existing-event
        // de-duplication prevents this from turning into an infinite loop.
        if changed {
            if let TransportPacket::Event { event } = packet {
                if matches!(event.kind.as_str(), "member_added" | "member_joined") {
                    self.broadcast_event(&event.workspace_id, event)?;
                }
                if event.kind == "member_joined" {
                    self.refresh_joiner_from_share(event)?;
                }
            }
        }

        self.mark_packet_seen(packet_id)?;
        Ok(changed)
    }

    fn refresh_joiner_from_share(&self, event: &SignedEvent) -> Result<()> {
        // A reusable org link is a snapshot at the moment it is created. When the
        // original sharer later sees a new member join through that link, send that
        // member a fresh sealed invite. This makes the complete member directory
        // (and therefore Contacts) converge even when the same link is reused over time.
        if event.kind != "member_joined" {
            return Ok(());
        }
        let key = self.workspace_key(&event.workspace_id)?;
        let plaintext = decrypt(
            &key,
            &unb64(&event.nonce_b64)?,
            &unb64(&event.ciphertext_b64)?,
        )?;
        let action: WorkspaceAction = serde_json::from_slice(&plaintext)?;
        let WorkspaceAction::MemberJoined { user, permit } = action else {
            return Ok(());
        };
        let me = self.identity()?;
        if permit.body.inviter.user_id != me.user_id || user.user_id == me.user_id {
            return Ok(());
        }
        let sealed = self.build_sealed_invite(&event.workspace_id, &user)?;
        let network_id = self.workspace_network_id(&event.workspace_id)?;
        self.queue_packet(
            &network_id,
            &user.mailbox_id,
            &TransportPacket::WorkspaceInvite { sealed },
        )?;
        Ok(())
    }

    fn verify_join_permit(&self, workspace_id: &str, permit: &SignedJoinPermit) -> Result<()> {
        if permit.body.version != PROTOCOL_VERSION {
            bail!("Unsupported workspace join permit version");
        }
        if permit.body.workspace_id != workspace_id {
            bail!("Workspace join permit does not match this workspace");
        }
        let workspace_network_id = self.workspace_network_id(workspace_id)?;
        if permit.body.network_id != workspace_network_id {
            bail!("Workspace join permit uses a different Firebase connection");
        }
        if permit.body.share_id.trim().is_empty() {
            bail!("Workspace join permit is missing its share identifier");
        }
        validate_public_user(&permit.body.inviter)?;
        let inviter = self
            .member_by_id(workspace_id, &permit.body.inviter.user_id)?
            .context("Workspace join permit inviter is not a current member")?;
        if inviter.sign_public_b64 != permit.body.inviter.sign_public_b64
            || inviter.box_public_b64 != permit.body.inviter.box_public_b64
        {
            bail!("Workspace join permit inviter identity does not match the workspace directory");
        }
        let verify_key = verifying_key(&permit.body.inviter.sign_public_b64)?;
        let signature = signature_from_b64(&permit.signature_b64)?;
        verify_key
            .verify(&serde_json::to_vec(&permit.body)?, &signature)
            .context("Invalid workspace join permit signature")?;
        Ok(())
    }

    fn store_and_apply_event(&self, event: &SignedEvent) -> Result<bool> {
        if event.version != PROTOCOL_VERSION {
            bail!("Unsupported event protocol version {}", event.version);
        }
        let already: Option<String> = self
            .conn()?
            .query_row(
                "SELECT id FROM events WHERE id=?1",
                params![event.id],
                |row| row.get(0),
            )
            .optional()?;
        if already.is_some() {
            return Ok(false);
        }

        let workspace_network_id = self.workspace_network_id(&event.workspace_id)?;
        let key = self.workspace_key(&event.workspace_id)?;
        let plaintext = decrypt(
            &key,
            &unb64(&event.nonce_b64)?,
            &unb64(&event.ciphertext_b64)?,
        )?;
        let action: WorkspaceAction = serde_json::from_slice(&plaintext)?;
        if action.kind() != event.kind {
            bail!("Event kind does not match encrypted payload");
        }
        if let WorkspaceAction::MemberJoined { permit, .. } = &action {
            self.verify_join_permit(&event.workspace_id, permit)?;
        }

        let existing_author = self.member_by_id(&event.workspace_id, &event.author_id)?;
        let author = if let Some(author) = existing_author {
            verify_event(event, &author)?;
            author
        } else {
            match &action {
                WorkspaceAction::MemberJoined { user, permit } => {
                    validate_public_user(user)?;
                    if user.user_id != event.author_id {
                        bail!("Workspace join event author does not match the joining user");
                    }
                    if user.network_id != workspace_network_id {
                        bail!("Joining user belongs to a different Firebase connection");
                    }
                    verify_event(event, user)?;
                    let _ = permit;
                    user.clone()
                }
                _ => bail!("Event author is not a workspace member"),
            }
        };

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO events(id,version,workspace_id,author_id,seq,created_at,kind,nonce_b64,ciphertext_b64,signature_b64)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                event.id,
                event.version as i64,
                event.workspace_id,
                event.author_id,
                event.seq,
                event.created_at,
                event.kind,
                event.nonce_b64,
                event.ciphertext_b64,
                event.signature_b64
            ],
        )?;

        match action {
            WorkspaceAction::MemberAdded { user } => {
                validate_public_user(&user)?;
                if user.network_id != workspace_network_id {
                    bail!("New member belongs to a different Trixy network");
                }
                upsert_member_and_contact_tx(&tx, &event.workspace_id, &user)?;
            }
            WorkspaceAction::MemberJoined { user, permit } => {
                validate_public_user(&user)?;
                if user.user_id != event.author_id || user.network_id != workspace_network_id {
                    bail!("Invalid workspace join event");
                }
                let _ = permit;
                upsert_member_and_contact_tx(&tx, &event.workspace_id, &user)?;
            }
            WorkspaceAction::MessageCreated {
                message_id,
                body,
                attachments,
            } => {
                tx.execute(
                    "INSERT OR IGNORE INTO messages(id,workspace_id,author_id,author_name,created_at,edited_at,body,deleted)
                     VALUES(?1,?2,?3,?4,?5,NULL,?6,0)",
                    params![
                        message_id,
                        event.workspace_id,
                        event.author_id,
                        author.name,
                        event.created_at,
                        body
                    ],
                )?;
                for attachment in attachments {
                    validate_attachment_meta(&attachment)?;
                    tx.execute(
                        "INSERT OR IGNORE INTO attachments(
                            id,workspace_id,message_id,author_id,file_name,size_bytes,sha256_b64,remote_token,chunk_count,chunk_size,storage_version,local_path,upload_pending,created_at
                         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,0,?12)",
                        params![
                            attachment.id,
                            event.workspace_id,
                            message_id,
                            event.author_id,
                            attachment.file_name,
                            attachment.size_bytes as i64,
                            attachment.sha256_b64,
                            attachment.remote_token,
                            attachment.chunk_count as i64,
                            attachment.chunk_size as i64,
                            attachment.storage_version as i64,
                            event.created_at
                        ],
                    )?;
                }
            }
            WorkspaceAction::MessageEdited { message_id, body } => {
                let original_author: Option<String> = tx
                    .query_row(
                        "SELECT author_id FROM messages WHERE id=?1 AND workspace_id=?2",
                        params![message_id, event.workspace_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if original_author.as_deref() != Some(event.author_id.as_str()) {
                    bail!("Message edit author mismatch or message is missing");
                }
                tx.execute(
                    "UPDATE messages SET body=?1,edited_at=?2 WHERE id=?3 AND workspace_id=?4",
                    params![body, event.created_at, message_id, event.workspace_id],
                )?;
            }
            WorkspaceAction::MessageDeleted { message_id } => {
                let original_author: Option<String> = tx
                    .query_row(
                        "SELECT author_id FROM messages WHERE id=?1 AND workspace_id=?2",
                        params![message_id, event.workspace_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if original_author.as_deref() != Some(event.author_id.as_str()) {
                    bail!("Message delete author mismatch or message is missing");
                }
                tx.execute(
                    "UPDATE messages SET deleted=1,body='' WHERE id=?1 AND workspace_id=?2",
                    params![message_id, event.workspace_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(true)
    }

    fn message_alert_for_event(&self, event: &SignedEvent) -> Result<Option<MessageAlert>> {
        if event.kind != "message_created" {
            return Ok(None);
        }
        let author = self
            .member_by_id(&event.workspace_id, &event.author_id)?
            .context("Message author is not a workspace member")?;
        verify_event(event, &author)?;
        let key = self.workspace_key(&event.workspace_id)?;
        let plaintext = decrypt(
            &key,
            &unb64(&event.nonce_b64)?,
            &unb64(&event.ciphertext_b64)?,
        )?;
        let action: WorkspaceAction = serde_json::from_slice(&plaintext)?;
        let WorkspaceAction::MessageCreated { body, attachments, .. } = action else {
            return Ok(None);
        };
        Ok(Some(MessageAlert {
            workspace_id: event.workspace_id.clone(),
            workspace_name: self.workspace_name(&event.workspace_id)?,
            author_name: author.name,
            body,
            has_attachments: !attachments.is_empty(),
        }))
    }

    fn events(&self, workspace_id: &str) -> Result<Vec<SignedEvent>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT version,id,workspace_id,author_id,seq,created_at,kind,nonce_b64,ciphertext_b64,signature_b64
             FROM events WHERE workspace_id=?1 ORDER BY created_at,id",
        )?;
        let rows = stmt.query_map(params![workspace_id], |row| {
            Ok(SignedEvent {
                version: row.get::<_, i64>(0)? as u32,
                id: row.get(1)?,
                workspace_id: row.get(2)?,
                author_id: row.get(3)?,
                seq: row.get(4)?,
                created_at: row.get(5)?,
                kind: row.get(6)?,
                nonce_b64: row.get(7)?,
                ciphertext_b64: row.get(8)?,
                signature_b64: row.get(9)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn build_signed_invite(&self, workspace_id: &str) -> Result<SignedInvite> {
        let identity = self.identity()?;
        let network_id = self.workspace_network_id(workspace_id)?;
        let workspace_name = self.workspace_name(workspace_id)?;
        let workspace_key_b64 = b64(&self.workspace_key(workspace_id)?);
        let body = InviteBody {
            version: PROTOCOL_VERSION,
            created_at: Utc::now().to_rfc3339(),
            network_id: network_id.clone(),
            workspace_id: workspace_id.to_string(),
            workspace_name,
            workspace_key_b64,
            members: self.members(workspace_id)?,
            events: self.events(workspace_id)?,
            inviter: identity.public_user_for(&network_id),
        };
        let signing = signing_key(&identity)?;
        let signature_b64 = b64(&signing.sign(&serde_json::to_vec(&body)?).to_bytes());
        Ok(SignedInvite { body, signature_b64 })
    }

    fn verify_signed_invite(&self, invite: &SignedInvite) -> Result<()> {
        if invite.body.version != PROTOCOL_VERSION {
            bail!("Unsupported workspace invitation version");
        }
        validate_public_user(&invite.body.inviter)?;
        if invite.body.inviter.network_id != invite.body.network_id {
            bail!("Workspace invitation inviter route is inconsistent");
        }
        let verify_key = verifying_key(&invite.body.inviter.sign_public_b64)?;
        let signature = signature_from_b64(&invite.signature_b64)?;
        verify_key
            .verify(&serde_json::to_vec(&invite.body)?, &signature)
            .context("Invalid workspace invitation signature")?;
        for member in &invite.body.members {
            validate_public_user(member)?;
            if member.network_id != invite.body.network_id {
                bail!("Workspace invitation contains a member from another Firebase connection");
            }
        }
        Ok(())
    }

    fn import_signed_invite(&self, invite: &SignedInvite, require_me: bool) -> Result<()> {
        self.verify_signed_invite(invite)?;
        self.network_by_id(&invite.body.network_id)?
            .context("Workspace Firebase connection is not configured")?;
        let identity = self.identity()?;
        let member_ids: HashSet<&str> = invite
            .body
            .members
            .iter()
            .map(|member| member.user_id.as_str())
            .collect();
        if require_me && !member_ids.contains(identity.user_id.as_str()) {
            bail!("Workspace invitation does not include this user");
        }
        let workspace_key = to_32(&unb64(&invite.body.workspace_key_b64)?)?;
        self.conn()?.execute(
            r#"INSERT INTO workspaces(id,name,workspace_key_b64,created_at,network_id)
               VALUES(?1,?2,?3,?4,?5)
               ON CONFLICT(id) DO UPDATE SET
                 name=excluded.name,
                 network_id=excluded.network_id"#,
            params![
                invite.body.workspace_id,
                invite.body.workspace_name,
                b64(&workspace_key),
                invite.body.created_at,
                invite.body.network_id
            ],
        )?;

        for member in &invite.body.members {
            self.upsert_member(&invite.body.workspace_id, member)?;
            self.upsert_contact(member)?;
        }

        for event in &invite.body.events {
            if let Err(err) = self.store_and_apply_event(event) {
                eprintln!("Skipping imported event {}: {err:#}", event.id);
            }
        }
        Ok(())
    }

    fn build_sealed_invite(&self, workspace_id: &str, recipient: &PublicUser) -> Result<SealedInvite> {
        let identity = self.identity()?;
        let signed = self.build_signed_invite(workspace_id)?;
        let my_secret = StaticSecret::from(to_32(&unb64(&identity.box_secret_b64)?)?);
        let recipient_public = X25519PublicKey::from(to_32(&unb64(&recipient.box_public_b64)?)?);
        let shared = my_secret.diffie_hellman(&recipient_public);
        let key = derive_invite_key(shared.as_bytes());
        let (nonce, ciphertext) = encrypt(&key, &serde_json::to_vec(&signed)?)?;
        Ok(SealedInvite {
            sender_box_public_b64: identity.box_public_b64,
            nonce_b64: b64(&nonce),
            ciphertext_b64: b64(&ciphertext),
        })
    }

    fn import_sealed_invite(&self, sealed: &SealedInvite) -> Result<()> {
        let identity = self.identity()?;
        let my_secret = StaticSecret::from(to_32(&unb64(&identity.box_secret_b64)?)?);
        let sender_public = X25519PublicKey::from(to_32(&unb64(&sealed.sender_box_public_b64)?)?);
        let shared = my_secret.diffie_hellman(&sender_public);
        let key = derive_invite_key(shared.as_bytes());
        let plaintext = decrypt(
            &key,
            &unb64(&sealed.nonce_b64)?,
            &unb64(&sealed.ciphertext_b64)?,
        )?;
        let invite: SignedInvite = serde_json::from_slice(&plaintext)?;
        if invite.body.inviter.box_public_b64 != sealed.sender_box_public_b64 {
            bail!("Workspace invitation sender mismatch");
        }
        self.import_signed_invite(&invite, true)
    }

    pub fn create_workspace_share_link(&self, workspace_id: &str) -> Result<String> {
        let network = self.workspace_network(workspace_id)?;
        let identity = self.identity()?;
        let invite = self.build_signed_invite(workspace_id)?;
        let share_id = Uuid::now_v7().to_string();
        let permit_body = JoinPermitBody {
            version: PROTOCOL_VERSION,
            created_at: Utc::now().to_rfc3339(),
            workspace_id: workspace_id.to_string(),
            network_id: network.network_id.clone(),
            share_id: share_id.clone(),
            inviter: identity.public_user_for(&network.network_id),
        };
        let permit_signature = b64(
            &signing_key(&identity)?
                .sign(&serde_json::to_vec(&permit_body)?)
                .to_bytes(),
        );
        let package = WorkspaceSharePackage {
            invite,
            permit: SignedJoinPermit {
                body: permit_body,
                signature_b64: permit_signature,
            },
        };
        let token = b64(&random_32());
        let share_key = random_32();
        let (nonce, ciphertext) = encrypt(&share_key, &serde_json::to_vec(&package)?)?;
        let encrypted = EncryptedWorkspaceShare {
            nonce_b64: b64(&nonce),
            ciphertext_b64: b64(&ciphertext),
        };
        let client = http_client()?;
        client
            .put(firebase_workspace_share_url(&network.firebase_url, &token))
            .json(&encrypted)
            .send()
            .context("Could not publish the workspace link")?
            .error_for_status()
            .context("Firebase rejected the workspace link. Publish the current firebase-rules.json")?;

        let link = WorkspaceShareLink {
            version: 1,
            firebase_url: network.firebase_url,
            network_label: network.label,
            workspace_id: workspace_id.to_string(),
            token,
            key_b64: b64(&share_key),
        };
        Ok(format!(
            "trixy://join/{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&link)?)
        ))
    }

    pub fn import_workspace_share_link(&self, value: &str) -> Result<String> {
        let trimmed = value.trim();
        let encoded = trimmed
            .strip_prefix("trixy://join/")
            .or_else(|| trimmed.strip_prefix("TRIXY-WORKSPACE-"))
            .unwrap_or(trimmed);
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .context("Invalid Trixy workspace link")?;
        let link: WorkspaceShareLink =
            serde_json::from_slice(&bytes).context("Invalid Trixy workspace link payload")?;
        if link.version != 1 {
            bail!("Unsupported Trixy workspace link version");
        }
        validate_firebase_key(&link.token)?;
        let share_key = to_32(&unb64(&link.key_b64)?)?;
        let network = self.ensure_network(
            &link.firebase_url,
            if link.network_label.trim().is_empty() {
                None
            } else {
                Some(link.network_label.as_str())
            },
        )?;
        let client = http_client()?;
        let response = client
            .get(firebase_workspace_share_url(&network.firebase_url, &link.token))
            .send()
            .context("Could not open the workspace link")?
            .error_for_status()
            .context("Firebase rejected the workspace link")?;
        let encrypted: Option<EncryptedWorkspaceShare> = response.json()?;
        let encrypted = encrypted.context("This workspace link no longer exists")?;
        let plaintext = decrypt(
            &share_key,
            &unb64(&encrypted.nonce_b64)?,
            &unb64(&encrypted.ciphertext_b64)?,
        )?;
        let package: WorkspaceSharePackage =
            serde_json::from_slice(&plaintext).context("Invalid encrypted workspace package")?;
        if package.invite.body.workspace_id != link.workspace_id
            || package.invite.body.network_id != network.network_id
            || package.permit.body.workspace_id != link.workspace_id
            || package.permit.body.network_id != network.network_id
        {
            bail!("Workspace link metadata does not match its encrypted package");
        }
        self.verify_signed_invite(&package.invite)?;
        let verify_key = verifying_key(&package.permit.body.inviter.sign_public_b64)?;
        let signature = signature_from_b64(&package.permit.signature_b64)?;
        verify_key
            .verify(&serde_json::to_vec(&package.permit.body)?, &signature)
            .context("Invalid workspace join permit")?;
        if package.permit.body.inviter.user_id != package.invite.body.inviter.user_id
            || package.permit.body.share_id.trim().is_empty()
        {
            bail!("Workspace link inviter information is inconsistent");
        }

        self.import_signed_invite(&package.invite, false)?;
        let identity = self.identity()?;
        if self
            .member_by_id(&link.workspace_id, &identity.user_id)?
            .is_none()
        {
            let me = identity.public_user_for(&network.network_id);
            self.upsert_member(&link.workspace_id, &me)?;
            self.upsert_contact(&me)?;
            let event = self.create_signed_event(
                &link.workspace_id,
                WorkspaceAction::MemberJoined {
                    user: me,
                    permit: package.permit,
                },
            )?;
            self.store_and_apply_event(&event)?;
            self.broadcast_event(&link.workspace_id, &event)?;
        }
        Ok(link.workspace_id)
    }

}

impl Identity {
    pub fn public_user(&self) -> PublicUser {
        self.public_user_for(&self.network_id)
    }

    pub fn public_user_for(&self, network_id: &str) -> PublicUser {
        PublicUser {
            user_id: self.user_id.clone(),
            name: self.name.clone(),
            network_id: network_id.to_string(),
            mailbox_id: self.mailbox_id.clone(),
            sign_public_b64: self.sign_public_b64.clone(),
            box_public_b64: self.box_public_b64.clone(),
        }
    }
}

pub fn default_db_path() -> PathBuf {
    // Prefer the legacy database when it already exists so the rename does not
    // make users recreate identities or workspaces. New installs use Trixy paths.
    #[cfg(target_os = "windows")]
    {
        if let Ok(base) = std::env::var("APPDATA") {
            let base = PathBuf::from(base);
            let legacy = base.join("Workmsg").join("workmsg-firebase-v02.db");
            let current = base.join("Trixy").join("trixy.db");
            if legacy.exists() && !current.exists() { return legacy; }
            return current;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            let home = PathBuf::from(home);
            let legacy = home.join("Library/Application Support/Workmsg").join("workmsg-firebase-v02.db");
            let current = home.join("Library/Application Support/Trixy").join("trixy.db");
            if legacy.exists() && !current.exists() { return legacy; }
            return current;
        }
    }
    if let Ok(base) = std::env::var("XDG_DATA_HOME") {
        let base = PathBuf::from(base);
        let legacy = base.join("workmsg").join("workmsg-firebase-v02.db");
        let current = base.join("trixy").join("trixy.db");
        if legacy.exists() && !current.exists() { return legacy; }
        return current;
    }
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        let legacy = home.join(".local/share/workmsg").join("workmsg-firebase-v02.db");
        let current = home.join(".local/share/trixy").join("trixy.db");
        if legacy.exists() && !current.exists() { return legacy; }
        return current;
    }
    PathBuf::from("trixy.db")
}

pub fn firebase_probe(firebase_url: &str) -> Result<()> {
    let firebase_url = normalize_firebase_url(firebase_url)?;
    let probe_mailbox = b64(&random_32());
    let client = http_client()?;

    client
        .get(firebase_mailbox_url(&firebase_url, &probe_mailbox))
        .send()
        .context("Could not reach Firebase")?
        .error_for_status()
        .context("Firebase rejected the mailbox read. Check the database URL and Rules tab")?;

    // File transfer uses a random mailbox capability too. Exercise the exact nested
    // path here so setup catches corporate proxy / Firebase rule problems immediately.
    let chunk_url = firebase_attachment_mailbox_chunk_url(&firebase_url, &probe_mailbox, 0);
    let probe = serde_json::json!({"probe": true});
    client
        .put(&chunk_url)
        .json(&probe)
        .send()
        .context("Could not test Firebase file transfer")?
        .error_for_status()
        .context("Firebase rejected the file-transfer test. Publish firebase-rules.json")?;
    client
        .get(&chunk_url)
        .send()
        .context("Could not read the Firebase file-transfer test")?
        .error_for_status()
        .context("Firebase rejected the file-transfer read")?;
    let _ = client.delete(&chunk_url).send();
    Ok(())
}

pub fn sync_once(db_path: impl AsRef<Path>) -> Result<SyncReport> {
    let db = AppDb::open(db_path)?;
    let identity = db.identity()?;
    let client = http_client()?;
    let mut report = SyncReport::default();

    for attachment in db.pending_attachment_uploads()? {
        match upload_attachment(&db, &client, &attachment) {
            Ok(()) => {
                db.mark_attachment_uploaded(&attachment.id)?;
            }
            Err(err) => report.errors.push(format!(
                "Could not upload attachment {}: {err:#}",
                attachment.file_name
            )),
        }
    }

    for (packet_id, network_id, mailbox_id, packet) in db.outbox_rows()? {
        let firebase_url = match db.network_url(&network_id) {
            Ok(url) => url,
            Err(err) => {
                report.errors.push(format!(
                    "Could not route packet {}: {err:#}",
                    short_id(&packet_id)
                ));
                continue;
            }
        };
        let url = firebase_packet_url(&firebase_url, &mailbox_id, &packet_id);
        match client
            .put(url)
            .json(&packet)
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(_) => {
                db.delete_outbox(&packet_id)?;
                report.sent += 1;
            }
            Err(err) => report.errors.push(format!(
                "Could not send packet {} on {}: {}",
                short_id(&packet_id),
                db.network_by_id(&network_id)?
                    .map(|network| network.label)
                    .unwrap_or_else(|| "Firebase".to_string()),
                err
            )),
        }
    }

    for network in db.networks()? {
        let response = match client
            .get(firebase_mailbox_url(
                &network.firebase_url,
                &identity.mailbox_id,
            ))
            .send()
            .and_then(|response| response.error_for_status())
        {
            Ok(response) => response,
            Err(err) => {
                report.errors.push(format!(
                    "{} sync unavailable: {}",
                    network.label, err
                ));
                continue;
            }
        };
        let packets: Option<HashMap<String, TransportPacket>> = response.json()?;
        let mut packets: Vec<(String, TransportPacket)> = packets
            .unwrap_or_default()
            .into_iter()
            .filter(|(key, _)| key != "file_chunks" && key != "workspace_share")
            .collect();
        packets.sort_by(|a, b| a.0.cmp(&b.0));

        for (packet_id, packet) in packets {
            match db.process_packet(&packet_id, &packet) {
                Ok(changed) => {
                    if changed {
                        report.received += 1;
                        if let TransportPacket::Event { event } = &packet {
                            if let Ok(Some(alert)) = db.message_alert_for_event(event) {
                                report.alerts.push(alert);
                            }
                        }
                    }

                    let delete_url = firebase_packet_url(
                        &network.firebase_url,
                        &identity.mailbox_id,
                        &packet_id,
                    );
                    if let Err(err) = client
                        .delete(delete_url)
                        .send()
                        .and_then(|response| response.error_for_status())
                    {
                        report.errors.push(format!(
                            "Could not clear {} mailbox packet {}: {}",
                            network.label,
                            short_id(&packet_id),
                            err
                        ));
                    }
                }
                Err(err) => {
                    report.errors.push(format!(
                        "Deferred {} mailbox packet {}: {err:#}",
                        network.label,
                        short_id(&packet_id)
                    ));
                }
            }
        }
    }

    Ok(report)
}

fn upload_attachment(
    db: &AppDb,
    client: &Client,
    attachment: &AttachmentView,
) -> Result<()> {
    validate_attachment_view(attachment)?;
    let path = attachment
        .local_path
        .as_ref()
        .context("Attachment file is missing locally")?;
    let metadata = fs::metadata(path)?;
    if metadata.len() != attachment.size_bytes {
        bail!("Local attachment size changed before upload");
    }
    if hash_file_b64(path)? != attachment.sha256_b64 {
        bail!("Local attachment contents changed before upload");
    }

    let firebase_url = db.workspace_network(&attachment.workspace_id)?.firebase_url;
    let key = db.workspace_key(&attachment.workspace_id)?;
    let mut file = File::open(path)?;
    let mut buffer = vec![0u8; attachment.chunk_size as usize];
    let mut total_read = 0u64;

    for index in 0..attachment.chunk_count {
        let mut read = 0usize;
        while read < buffer.len() {
            let n = file.read(&mut buffer[read..])?;
            if n == 0 {
                break;
            }
            read += n;
        }
        total_read += read as u64;
        let (nonce, ciphertext) = encrypt(&key, &buffer[..read])?;
        let chunk = AttachmentChunk {
            nonce_b64: b64(&nonce),
            ciphertext_b64: b64(&ciphertext),
        };
        upload_attachment_chunk(
            client,
            &firebase_url,
            &attachment.remote_token,
            attachment.storage_version,
            index,
            &chunk,
        )
        .with_context(|| format!("Could not upload file chunk {}/{}", index + 1, attachment.chunk_count))?;
    }

    if total_read != attachment.size_bytes {
        bail!("Attachment size changed during upload");
    }
    Ok(())
}

fn retry_pause(attempt: usize) {
    let millis = match attempt {
        0 => 80,
        1 => 180,
        2 => 420,
        _ => 900,
    };
    std::thread::sleep(Duration::from_millis(millis));
}

fn upload_attachment_chunk(
    client: &Client,
    firebase_url: &str,
    remote_token: &str,
    storage_version: u8,
    chunk_index: u32,
    chunk: &AttachmentChunk,
) -> Result<()> {
    let urls = attachment_chunk_write_urls(firebase_url, remote_token, storage_version, chunk_index);
    let mut last_error = None;
    for url in urls {
        for attempt in 0..ATTACHMENT_RETRY_ATTEMPTS {
            match client.put(&url).json(chunk).send() {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    last_error = Some(anyhow!("Firebase returned HTTP {}", response.status()));
                    // Permission failures on the legacy /attachments path should immediately
                    // fall through to the mailbox-compatible path.
                    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
                        break;
                    }
                }
                Err(err) => last_error = Some(anyhow!(err)),
            }
            retry_pause(attempt);
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Attachment chunk upload failed")))
}

fn download_attachment_chunk(
    client: &Client,
    firebase_url: &str,
    remote_token: &str,
    storage_version: u8,
    chunk_index: u32,
) -> Result<AttachmentChunk> {
    let urls = attachment_chunk_read_urls(firebase_url, remote_token, storage_version, chunk_index);
    let mut last_error = None;
    for url in urls {
        for attempt in 0..ATTACHMENT_RETRY_ATTEMPTS {
            match client.get(&url).send() {
                Ok(response) if response.status().is_success() => {
                    let chunk: Option<AttachmentChunk> = response.json()?;
                    if let Some(chunk) = chunk {
                        return Ok(chunk);
                    }
                    last_error = Some(anyhow!("File chunk has not arrived yet"));
                }
                Ok(response) => {
                    last_error = Some(anyhow!("Firebase returned HTTP {}", response.status()));
                    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
                        break;
                    }
                }
                Err(err) => last_error = Some(anyhow!(err)),
            }
            retry_pause(attempt);
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Attachment chunk download failed")))
}

pub fn format_time(timestamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|_| timestamp.to_string())
}

pub fn member_names(members: &[PublicUser]) -> String {
    members
        .iter()
        .map(|member| member.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn upsert_member_and_contact_tx(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    user: &PublicUser,
) -> Result<()> {
    tx.execute(
        r#"INSERT INTO members(workspace_id,user_id,name,network_id,mailbox_id,sign_public_b64,box_public_b64)
           VALUES(?1,?2,?3,?4,?5,?6,?7)
           ON CONFLICT(workspace_id,user_id) DO UPDATE SET
             name=excluded.name,
             network_id=excluded.network_id,
             mailbox_id=excluded.mailbox_id,
             sign_public_b64=excluded.sign_public_b64,
             box_public_b64=excluded.box_public_b64"#,
        params![
            workspace_id,
            user.user_id,
            user.name,
            user.network_id,
            user.mailbox_id,
            user.sign_public_b64,
            user.box_public_b64
        ],
    )?;
    tx.execute(
        r#"INSERT INTO contacts(user_id,name,network_id,mailbox_id,sign_public_b64,box_public_b64)
           VALUES(?1,?2,?3,?4,?5,?6)
           ON CONFLICT(user_id) DO UPDATE SET
             name=excluded.name,
             network_id=excluded.network_id,
             mailbox_id=excluded.mailbox_id,
             sign_public_b64=excluded.sign_public_b64,
             box_public_b64=excluded.box_public_b64"#,
        params![
            user.user_id,
            user.name,
            user.network_id,
            user.mailbox_id,
            user.sign_public_b64,
            user.box_public_b64
        ],
    )?;
    tx.execute(
        r#"INSERT INTO contact_routes(user_id,network_id,mailbox_id)
           VALUES(?1,?2,?3)
           ON CONFLICT(user_id,network_id) DO UPDATE SET mailbox_id=excluded.mailbox_id"#,
        params![user.user_id, user.network_id, user.mailbox_id],
    )?;
    Ok(())
}

fn row_to_public_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublicUser> {
    Ok(PublicUser {
        user_id: row.get(0)?,
        name: row.get(1)?,
        network_id: row.get(2)?,
        mailbox_id: row.get(3)?,
        sign_public_b64: row.get(4)?,
        box_public_b64: row.get(5)?,
    })
}

fn row_to_attachment_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<AttachmentView> {
    let local_path: Option<String> = row.get(11)?;
    Ok(AttachmentView {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        message_id: row.get(2)?,
        author_id: row.get(3)?,
        file_name: row.get(4)?,
        size_bytes: row.get::<_, i64>(5)? as u64,
        sha256_b64: row.get(6)?,
        remote_token: row.get(7)?,
        chunk_count: row.get::<_, i64>(8)? as u32,
        chunk_size: row.get::<_, i64>(9)? as u32,
        storage_version: row.get::<_, i64>(10)? as u8,
        local_path: local_path.map(PathBuf::from),
        upload_pending: row.get::<_, i64>(12)? != 0,
    })
}

fn hash_file_b64(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(b64(&hasher.finalize()))
}

fn safe_file_name(file_name: &str) -> String {
    let mut out = String::with_capacity(file_name.len());
    for c in file_name.chars() {
        if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    let trimmed = out.trim().trim_matches('.');
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.chars().take(160).collect()
    }
}

fn validate_attachment_meta(meta: &AttachmentMeta) -> Result<()> {
    if meta.id.trim().is_empty() || meta.file_name.trim().is_empty() {
        bail!("Incomplete attachment metadata");
    }
    if meta.size_bytes > MAX_ATTACHMENT_BYTES {
        bail!("Attachment exceeds the configured size limit");
    }
    if meta.chunk_count == 0 || meta.chunk_size == 0 {
        bail!("Attachment chunk metadata is invalid");
    }
    if meta.chunk_size as usize > LEGACY_ATTACHMENT_CHUNK_SIZE {
        bail!("Attachment chunk size is too large");
    }
    if meta.storage_version == 0 || meta.storage_version > ATTACHMENT_STORAGE_VERSION {
        bail!("Unsupported attachment storage version");
    }
    let expected_chunks = std::cmp::max(
        1,
        ((meta.size_bytes + meta.chunk_size as u64 - 1) / meta.chunk_size as u64) as u32,
    );
    if meta.chunk_count != expected_chunks {
        bail!("Attachment chunk count does not match its size");
    }
    validate_firebase_key(&meta.remote_token)?;
    if unb64(&meta.sha256_b64)?.len() != 32 {
        bail!("Invalid attachment hash");
    }
    Ok(())
}

fn validate_attachment_view(attachment: &AttachmentView) -> Result<()> {
    validate_attachment_meta(&AttachmentMeta {
        id: attachment.id.clone(),
        file_name: attachment.file_name.clone(),
        size_bytes: attachment.size_bytes,
        sha256_b64: attachment.sha256_b64.clone(),
        remote_token: attachment.remote_token.clone(),
        chunk_count: attachment.chunk_count,
        chunk_size: attachment.chunk_size,
        storage_version: attachment.storage_version,
    })
}

fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("Trixy/0.6")
        .build()?)
}

fn firebase_label(firebase_url: &str) -> String {
    let trimmed = firebase_url
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = trimmed.split('/').next().unwrap_or(trimmed);
    host.split('.').next().unwrap_or(host).replace("-default-rtdb", "")
}

fn normalize_firebase_url(value: &str) -> Result<String> {
    let mut url = value.trim().trim_end_matches('/').to_string();
    if url.ends_with(".json") {
        url.truncate(url.len() - 5);
    }
    if url.is_empty() {
        bail!("Firebase database URL cannot be empty");
    }
    if !(url.starts_with("https://")
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost"))
    {
        bail!("Use an https:// Firebase Realtime Database URL");
    }
    Ok(url)
}

fn network_id_for_url(firebase_url: &str) -> String {
    let digest = Sha256::digest(firebase_url.as_bytes());
    b64(&digest[..16])
}

fn firebase_mailbox_url(firebase_url: &str, mailbox_id: &str) -> String {
    format!(
        "{}/{}/mailboxes/{}.json",
        firebase_url.trim_end_matches('/'),
        FIREBASE_ROOT,
        mailbox_id
    )
}

fn firebase_packet_url(firebase_url: &str, mailbox_id: &str, packet_id: &str) -> String {
    format!(
        "{}/{}/mailboxes/{}/{}.json",
        firebase_url.trim_end_matches('/'),
        FIREBASE_ROOT,
        mailbox_id,
        packet_id
    )
}

fn firebase_workspace_share_url(firebase_url: &str, token: &str) -> String {
    format!(
        "{}/{}/mailboxes/{}/workspace_share.json",
        firebase_url.trim_end_matches('/'),
        FIREBASE_ROOT,
        token
    )
}

fn firebase_attachment_mailbox_chunk_url(
    firebase_url: &str,
    remote_token: &str,
    chunk_index: u32,
) -> String {
    // A random attachment token acts as a synthetic mailbox capability. This reuses
    // the exact Firebase rule path that already carries messages, so installations
    // with working chat do not need a second permission namespace for files.
    format!(
        "{}/{}/mailboxes/{}/file_chunks/{}.json",
        firebase_url.trim_end_matches('/'),
        FIREBASE_ROOT,
        remote_token,
        chunk_index
    )
}

fn firebase_legacy_attachment_chunk_url(
    firebase_url: &str,
    remote_token: &str,
    chunk_index: u32,
) -> String {
    format!(
        "{}/{}/attachments/{}/{}.json",
        firebase_url.trim_end_matches('/'),
        FIREBASE_ROOT,
        remote_token,
        chunk_index
    )
}

fn attachment_chunk_write_urls(
    firebase_url: &str,
    remote_token: &str,
    storage_version: u8,
    chunk_index: u32,
) -> Vec<String> {
    if storage_version >= 2 {
        vec![firebase_attachment_mailbox_chunk_url(firebase_url, remote_token, chunk_index)]
    } else {
        // Old pending v0.3 transfers try the old path first, then the new mailbox
        // path so a previously rejected file can still recover after upgrading.
        vec![
            firebase_legacy_attachment_chunk_url(firebase_url, remote_token, chunk_index),
            firebase_attachment_mailbox_chunk_url(firebase_url, remote_token, chunk_index),
        ]
    }
}

fn attachment_chunk_read_urls(
    firebase_url: &str,
    remote_token: &str,
    storage_version: u8,
    chunk_index: u32,
) -> Vec<String> {
    if storage_version >= 2 {
        vec![firebase_attachment_mailbox_chunk_url(firebase_url, remote_token, chunk_index)]
    } else {
        vec![
            firebase_legacy_attachment_chunk_url(firebase_url, remote_token, chunk_index),
            firebase_attachment_mailbox_chunk_url(firebase_url, remote_token, chunk_index),
        ]
    }
}

fn validate_public_user(user: &PublicUser) -> Result<()> {
    if user.user_id.trim().is_empty()
        || user.name.trim().is_empty()
        || user.network_id.trim().is_empty()
        || user.mailbox_id.trim().is_empty()
    {
        bail!("Incomplete Trixy user profile");
    }
    validate_firebase_key(&user.mailbox_id)?;
    if unb64(&user.sign_public_b64)?.len() != 32 || unb64(&user.box_public_b64)?.len() != 32 {
        bail!("Invalid public key length");
    }
    Ok(())
}

fn validate_firebase_key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 768
        || value
            .chars()
            .any(|c| matches!(c, '.' | '$' | '#' | '[' | ']' | '/') || c.is_control())
    {
        bail!("Invalid Firebase mailbox identifier");
    }
    Ok(())
}

fn random_32() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(value: &str) -> Result<Vec<u8>> {
    Ok(URL_SAFE_NO_PAD.decode(value)?)
}

fn to_32(bytes: &[u8]) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| anyhow!("Expected a 32-byte key"))
}

fn signing_key(identity: &Identity) -> Result<SigningKey> {
    Ok(SigningKey::from_bytes(&to_32(&unb64(
        &identity.sign_secret_b64,
    )?)?))
}

fn verifying_key(public_b64: &str) -> Result<VerifyingKey> {
    VerifyingKey::from_bytes(&to_32(&unb64(public_b64)?)?).map_err(Into::into)
}

fn signature_from_b64(value: &str) -> Result<Signature> {
    let bytes = unb64(value)?;
    let array: [u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("Expected a 64-byte signature"))?;
    Ok(Signature::from_bytes(&array))
}

fn event_signing_bytes(event: &SignedEvent) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        event.version,
        &event.id,
        &event.workspace_id,
        &event.author_id,
        event.seq,
        &event.created_at,
        &event.kind,
        &event.nonce_b64,
        &event.ciphertext_b64,
    ))?)
}

fn sign_event(event: &SignedEvent, identity: &Identity) -> Result<String> {
    let key = signing_key(identity)?;
    Ok(b64(&key.sign(&event_signing_bytes(event)?).to_bytes()))
}

fn verify_event(event: &SignedEvent, author: &PublicUser) -> Result<()> {
    if event.author_id != author.user_id {
        bail!("Event author mismatch");
    }
    let key = verifying_key(&author.sign_public_b64)?;
    let signature = signature_from_b64(&event.signature_b64)?;
    key.verify(&event_signing_bytes(event)?, &signature)
        .context("Invalid event signature")?;
    Ok(())
}

fn encrypt(key_bytes: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key_bytes));
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| anyhow!("Encryption failed"))?;
    Ok((nonce.to_vec(), ciphertext))
}

fn decrypt(key_bytes: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if nonce.len() != 12 {
        bail!("Invalid nonce length");
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key_bytes));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("Decryption failed"))
}

fn derive_invite_key(shared: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"trixy-invite-v2");
    hasher.update(shared);
    hasher.finalize().into()
}

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}
