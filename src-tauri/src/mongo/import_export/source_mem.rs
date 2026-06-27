//! In-memory document source, backed by a pre-parsed vector of documents.
//! Used for exporting an explicit set of documents (e.g. the user's current
//! selection) where the docs are supplied by the caller rather than streamed
//! from a cursor.

use async_trait::async_trait;
use bson::Document;
use std::collections::VecDeque;

use super::core::{DocumentSource, RowResult};
use crate::error::AppResult;

pub struct VecSource {
    docs: VecDeque<Document>,
    total: u64,
}

impl VecSource {
    pub fn new(docs: Vec<Document>) -> Self {
        let total = docs.len() as u64;
        Self {
            docs: docs.into(),
            total,
        }
    }
}

#[async_trait]
impl DocumentSource for VecSource {
    fn size_hint(&self) -> Option<u64> {
        Some(self.total)
    }

    async fn next_doc(&mut self) -> AppResult<Option<RowResult>> {
        Ok(self.docs.pop_front().map(RowResult::Doc))
    }
}
