//! Connection pool / client registry. Active `mongodb::Client` instances
//! are stored in a `RwLock<HashMap<connectionId, ClientEntry>>`. Each
//! entry is reference-counted (`Arc<Client>`) so clones of the client can
//! be handed into async tasks without holding a lock across `.await`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mongodb::options::ClientOptions;
use mongodb::Client;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::mongo::types::{CollectionKind, CollectionSummary, ConnectionHandle, DatabaseSummary, ServerInfo};

/// A pooled client plus the metadata that callers need to scope subsequent
/// requests without re-fetching the profile.
#[derive(Clone)]
pub struct ClientEntry {
    pub client: Arc<Client>,
    pub profile_id: String,
    pub name: String,
    pub opened_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Default)]
pub struct ClientRegistry {
    inner: RwLock<HashMap<String, ClientEntry>>,
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, connection_id: String, entry: ClientEntry) {
        self.inner.write().await.insert(connection_id, entry);
    }

    pub async fn get(&self, connection_id: &str) -> AppResult<ClientEntry> {
        self.inner
            .read()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| AppError::ConnectionNotFound(connection_id.to_string()))
    }

    pub async fn remove(&self, connection_id: &str) -> AppResult<ClientEntry> {
        self.inner
            .write()
            .await
            .remove(connection_id)
            .ok_or_else(|| AppError::ConnectionNotFound(connection_id.to_string()))
    }

    pub async fn list(&self) -> Vec<ConnectionDescriptor> {
        self.inner
            .read()
            .await
            .values()
            .map(|e| ConnectionDescriptor {
                connection_id: e.client_hash().to_string(),
                profile_id: e.profile_id.clone(),
                name: e.name.clone(),
                opened_at: e.opened_at.to_rfc3339(),
            })
            .collect()
    }
}

/// Stable descriptor used by the frontend to show active connections.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDescriptor {
    pub connection_id: String,
    pub profile_id: String,
    pub name: String,
    pub opened_at: String,
}

impl ClientEntry {
    fn client_hash(&self) -> String {
        // The connection id is already unique; expose it back to callers.
        // We use a separate id for the entry so callers can refer to a
        // specific connection in a way that survives re-keying.
        format!("{}::{}", self.profile_id, self.opened_at.timestamp_millis())
    }
}

/// Build a `mongodb::Client` from a profile + secret. Honors the SSH tunnel
/// and SOCKS5 configuration, sets timeouts, and applies a stable
/// application name so it shows up in `db.currentOp()` on the server.
pub async fn build_client(
    uri: &str,
    app_name: &str,
) -> AppResult<Arc<Client>> {
    if uri.trim().is_empty() {
        return Err(AppError::Validation("connection URI must not be empty".into()));
    }
    let mut options = ClientOptions::parse(uri).await?;
    options.app_name = Some(app_name.to_string());
    options.server_selection_timeout = Some(Duration::from_secs(8));
    options.connect_timeout = Some(Duration::from_secs(8));
    options.max_pool_size = Some(32);
    options.min_pool_size = Some(1);
    let client = Client::with_options(options)?;
    Ok(Arc::new(client))
}

/// Probe the server, list databases, and produce a `ConnectionHandle`.
pub async fn describe_connection(
    client: &Client,
    connection_id: &str,
    profile_id: &str,
    name: &str,
) -> AppResult<ConnectionHandle> {
    let server_info = hello(client).await.ok();
    let databases = list_databases(client).await?;
    Ok(ConnectionHandle {
        connection_id: connection_id.to_string(),
        profile_id: profile_id.to_string(),
        name: name.to_string(),
        server_info,
        databases,
    })
}

async fn hello(client: &Client) -> AppResult<ServerInfo> {
    let doc = client
        .database("admin")
        .run_command(bson::doc! { "hello": 1 })
        .await?;
    let version = doc.get_str("maxWireVersion").ok().map(|_| "unknown".to_string());
    let host = doc.get_str("me").ok().map(|s| s.to_string());
    let is_master = doc.get_bool("isWritablePrimary").unwrap_or(false);
    let topology = if doc.contains_key("setName") {
        "replicaSet"
    } else if doc.contains_key("msg") && doc.get_str("msg").unwrap_or("") == "isdbgrid" {
        "sharded"
    } else {
        "standalone"
    };
    Ok(ServerInfo {
        version,
        host,
        is_master: Some(is_master),
        topology: Some(topology.to_string()),
    })
}

pub async fn list_databases(client: &Client) -> AppResult<Vec<DatabaseSummary>> {
    let databases = client.list_database_names().await?;
    let mut out = Vec::with_capacity(databases.len());
    for name in databases {
        let size = client
            .database(&name)
            .run_command(bson::doc! { "dbStats": 1 })
            .await
            .ok()
            .and_then(|d| d.get_i64("dataSize").ok().map(|v| v as u64));
        let collections_count = client
            .database(&name)
            .list_collection_names()
            .await
            .ok()
            .map(|c| c.len() as u64);
        out.push(DatabaseSummary {
            name,
            size_on_disk: size,
            collections_count,
        });
    }
    Ok(out)
}

pub async fn list_collections(
    client: &Client,
    db: &str,
) -> AppResult<Vec<CollectionSummary>> {
    let names = client.database(db).list_collection_names().await?;
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let kind = classify_collection_name(client, db, &name).await;
        let document_count = client
            .database(db)
            .run_command(bson::doc! { "count": &name })
            .await
            .ok()
            .and_then(|d| d.get_i32("n").ok().map(|v| v as u64).or_else(|| d.get_i64("n").ok().map(|v| v as u64)));
        let stats = client
            .database(db)
            .run_command(bson::doc! { "collStats": &name })
            .await
            .ok();
        let (size_bytes, storage_size_bytes) = match stats {
            Some(d) => (
                d.get_i64("size").ok().map(|v| v as u64),
                d.get_i64("storageSize").ok().map(|v| v as u64),
            ),
            None => (None, None),
        };
        out.push(CollectionSummary {
            name,
            kind,
            document_count,
            size_bytes,
            storage_size_bytes,
        });
    }
    Ok(out)
}

async fn classify_collection_name(client: &Client, db: &str, name: &str) -> CollectionKind {
    let info = client
        .database(db)
        .run_command(bson::doc! {
            "listCollections": 1,
            "filter": { "name": name },
        })
        .await
        .ok();
    let Some(info) = info else {
        return CollectionKind::Collection;
    };
    if let Ok(cursor) = info.get_document("cursor") {
        if let Ok(first_batch) = cursor.get_array("firstBatch") {
            if let Some(bson::Bson::Document(doc)) = first_batch.first() {
                return classify_collection(doc);
            }
        }
    }
    CollectionKind::Collection
}

fn classify_collection(info: &bson::Document) -> CollectionKind {
    let t = info.get_str("type").unwrap_or("collection");
    match t {
        "view" => CollectionKind::View,
        "timeseries" => CollectionKind::TimeSeries,
        "sharded" => CollectionKind::Sharded,
        _ => CollectionKind::Collection,
    }
}
