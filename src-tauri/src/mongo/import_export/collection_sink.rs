//! MongoDB collection sink for import jobs.

use async_trait::async_trait;
use bson::Document;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::core::DocumentSink;
use crate::error::AppResult;

pub struct CollectionSink {
    collection: mongodb::Collection<Document>,
    batch: Vec<Document>,
    batch_size: usize,
    inserted: Arc<AtomicU64>,
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
        }
    }

    async fn flush(&mut self) -> AppResult<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let docs = std::mem::take(&mut self.batch);
        let count = docs.len() as u64;
        self.collection.insert_many(docs).await?;
        self.inserted.fetch_add(count, Ordering::Relaxed);
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
