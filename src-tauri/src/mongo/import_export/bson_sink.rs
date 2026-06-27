//! BSON document sink: writes length-prefixed BSON documents sequentially.
//! Compatible with `mongodump` `.bson` output.

use async_trait::async_trait;
use bson::Document;

use super::core::DocumentSink;
use super::io_util::WriteTarget;
use crate::error::{AppError, AppResult};

pub struct BsonSink {
    target: Option<WriteTarget>,
}

impl BsonSink {
    pub fn new(target: WriteTarget) -> Self {
        Self { target: Some(target) }
    }

    fn target_mut(&mut self) -> AppResult<&mut WriteTarget> {
        self.target
            .as_mut()
            .ok_or_else(|| AppError::Internal("bson sink already finalized".into()))
    }
}

#[async_trait]
impl DocumentSink for BsonSink {
    async fn start(&mut self) -> AppResult<()> {
        Ok(())
    }

    async fn write(&mut self, doc: Document) -> AppResult<()> {
        let bytes = bson::to_vec(&doc)?;
        self.target_mut()?.write_all(&bytes)?;
        Ok(())
    }

    async fn finish(mut self: Box<Self>) -> AppResult<()> {
        if let Some(target) = self.target.take() {
            target.commit()?;
        }
        Ok(())
    }

    async fn abort(mut self: Box<Self>) -> AppResult<()> {
        if let Some(target) = self.target.take() {
            target.abort();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::import_export::core::{DocumentSource, RowResult};
    use bson::{doc, oid::ObjectId};
    use std::str::FromStr;

    #[tokio::test]
    async fn bson_sink_writes_and_file_round_trips() {
        let path = std::env::temp_dir().join(format!("mongo-buddy-bson-sink-test-{}", uuid::Uuid::new_v4()));
        let writer = crate::mongo::import_export::io_util::AtomicFileWriter::create(path.clone()).unwrap();
        let mut sink = BsonSink::new(WriteTarget::File(writer));

        sink.start().await.unwrap();
        sink.write(doc! { "_id": ObjectId::from_str("64f1a2b3c4d5e6f789012345").unwrap(), "name": "Alice", "age": 30 }).await.unwrap();
        sink.write(doc! { "_id": ObjectId::from_str("64f1a2b3c4d5e6f789012346").unwrap(), "name": "Bob", "age": 25 }).await.unwrap();
        Box::new(sink).finish().await.unwrap();

        // Read back via BsonSource
        let mut source = crate::mongo::import_export::bson_source::BsonSource::from_path(&path).unwrap();
        let doc1 = source.next_doc().await.unwrap().unwrap();
        let doc2 = source.next_doc().await.unwrap().unwrap();
        let eof = source.next_doc().await.unwrap();

        if let RowResult::Doc(d1) = doc1 {
            assert_eq!(d1.get_str("name").unwrap(), "Alice");
            assert_eq!(d1.get_i32("age").unwrap(), 30);
        } else { panic!("expected doc"); }

        if let RowResult::Doc(d2) = doc2 {
            assert_eq!(d2.get_str("name").unwrap(), "Bob");
        } else { panic!("expected doc"); }

        assert!(eof.is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn bson_sink_abort_does_not_leave_file() {
        let path = std::env::temp_dir().join(format!("mongo-buddy-bson-abort-test-{}", uuid::Uuid::new_v4()));
        let writer = crate::mongo::import_export::io_util::AtomicFileWriter::create(path.clone()).unwrap();
        let sink = BsonSink::new(WriteTarget::File(writer));
        Box::new(sink).abort().await.unwrap();
        // The .part file should be cleaned up by AtomicFileWriter::abort.
        let part = crate::mongo::import_export::io_util::part_path_for(&path);
        assert!(!part.exists());
        assert!(!path.exists());
    }
}
