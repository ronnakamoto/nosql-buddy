//! Document source that pulls from a MongoDB Cursor.

use async_trait::async_trait;
use bson::Document;
use futures_util::stream::StreamExt;
use mongodb::Cursor;

use super::core::{DocumentSource, RowResult};
use crate::error::AppResult;

pub struct CursorSource {
    cursor: Cursor<Document>,
    total_count: Option<u64>,
}

impl CursorSource {
    pub fn new(cursor: Cursor<Document>, total_count: Option<u64>) -> Self {
        Self {
            cursor,
            total_count,
        }
    }
}

#[async_trait]
impl DocumentSource for CursorSource {
    fn size_hint(&self) -> Option<u64> {
        self.total_count
    }

    async fn next_doc(&mut self) -> AppResult<Option<RowResult>> {
        match self.cursor.next().await {
            Some(Ok(doc)) => Ok(Some(RowResult::Doc(doc))),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }
}
