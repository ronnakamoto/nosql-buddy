//! MongoDB collection sink for import / restore jobs.
//!
//! Two modes:
//! - `Insert` — batch `insert_many` (fast, fails on duplicates).
//! - `Upsert` — `replace_one` with `upsert:true` per document (slower,
//!   overwrites existing documents by `_id`).

use async_trait::async_trait;
use bson::Document;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::core::DocumentSink;
use crate::error::AppResult;

#[derive(Debug, Clone, Copy)]
pub enum InsertMode {
    Insert,
    Upsert,
}

pub struct CollectionSink {
    collection: mongodb::Collection<Document>,
    batch: Vec<Document>,
    batch_size: usize,
    inserted: Arc<AtomicU64>,
    mode: InsertMode,
}

impl CollectionSink {
    pub fn new(
        collection: mongodb::Collection<Document>,
        batch_size: usize,
        inserted: Arc<AtomicU64>,
    ) -> Self {
        Self {
            collection,
            batch: Vec::with_capacity(batch_size.clamp(1, 10_000)),
            batch_size: batch_size.max(1),
            inserted,
            mode: InsertMode::Insert,
        }
    }

    pub fn with_mode(mut self, mode: InsertMode) -> Self {
        self.mode = mode;
        self
    }

    async fn flush(&mut self) -> AppResult<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let docs = std::mem::take(&mut self.batch);
        match self.mode {
            InsertMode::Insert => {
                let count = docs.len() as u64;
                self.collection.insert_many(docs).await?;
                self.inserted.fetch_add(count, Ordering::Relaxed);
            }
            InsertMode::Upsert => {
                for doc in docs {
                    let filter = match doc.get("_id") {
                        Some(id) => bson::doc! { "_id": id },
                        None => bson::doc! {}, // no _id → always insert
                    };
                    let opts = mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build();
                    self.collection.replace_one(filter, doc).with_options(opts).await?;
                    self.inserted.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl DocumentSink for CollectionSink {
    async fn start(&mut self) -> AppResult<()> {
        Ok(())
    }

    async fn write(&mut self, doc: Document) -> AppResult<()> {
        self.batch.push(doc);
        if self.batch.len() >= self.batch_size {
            self.flush().await?;
        }
        Ok(())
    }

    async fn finish(mut self: Box<Self>) -> AppResult<()> {
        self.flush().await
    }

    async fn abort(self: Box<Self>) -> AppResult<()> {
        Ok(())
    }
}
