//! JSON document sink: writes Extended JSON, either as a JSON array or as
//! newline-delimited JSON (NDJSON). Targets a file or an in-memory buffer
//! (for clipboard exports).

use async_trait::async_trait;
use bson::{Bson, Document};
use std::sync::{Arc, Mutex};

use super::core::DocumentSink;
use super::io_util::WriteSink;
use crate::error::{AppError, AppResult};

#[derive(Clone, Copy)]
pub enum JsonShape {
    /// One JSON array containing every document.
    Array,
    /// One JSON document per line (NDJSON / JSON Lines).
    Ndjson,
}

pub struct JsonSink {
    target: Option<WriteSink>,
    output_slot: Arc<Mutex<Option<String>>>,
    shape: JsonShape,
    canonical: bool,
    wrote_first: bool,
}

impl JsonSink {
    pub fn new(
        target: WriteSink,
        output_slot: Arc<Mutex<Option<String>>>,
        shape: JsonShape,
        canonical: bool,
    ) -> Self {
        Self {
            target: Some(target),
            output_slot,
            shape,
            canonical,
            wrote_first: false,
        }
    }

    fn target_mut(&mut self) -> AppResult<&mut WriteSink> {
        self.target
            .as_mut()
            .ok_or_else(|| AppError::Internal("json sink already finalized".into()))
    }

    fn encode(&self, doc: Document) -> Vec<u8> {
        let value = if self.canonical {
            Bson::Document(doc).into_canonical_extjson()
        } else {
            Bson::Document(doc).into_relaxed_extjson()
        };
        serde_json::to_vec(&value).unwrap_or_default()
    }
}

#[async_trait]
impl DocumentSink for JsonSink {
    async fn start(&mut self) -> AppResult<()> {
        if let JsonShape::Array = self.shape {
            self.target_mut()?.write_all(b"[")?;
        }
        Ok(())
    }

    async fn write(&mut self, doc: Document) -> AppResult<()> {
        let bytes = self.encode(doc);
        let shape = self.shape;
        let first = !self.wrote_first;
        let target = self.target_mut()?;
        match shape {
            JsonShape::Array => {
                if first {
                    target.write_all(b"\n  ")?;
                } else {
                    target.write_all(b",\n  ")?;
                }
                target.write_all(&bytes)?;
            }
            JsonShape::Ndjson => {
                target.write_all(&bytes)?;
                target.write_all(b"\n")?;
            }
        }
        self.wrote_first = true;
        Ok(())
    }

    async fn finish(mut self: Box<Self>) -> AppResult<()> {
        if let JsonShape::Array = self.shape {
            let suffix: &[u8] = if self.wrote_first { b"\n]\n" } else { b"]\n" };
            self.target_mut()?.write_all(suffix)?;
        }
        let target = self
            .target
            .take()
            .ok_or_else(|| AppError::Internal("json sink already finalized".into()))?;
        let text = target.finish()?;
        if let Some(text) = text {
            *self.output_slot.lock().map_err(lock_err)? = Some(text);
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

fn lock_err<T>(_: std::sync::PoisonError<T>) -> AppError {
    AppError::Internal("export output slot mutex poisoned".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::oid::ObjectId;
    use bson::spec::BinarySubtype;
    use bson::{doc, Binary, Bson, DateTime, Decimal128};
    use std::str::FromStr;

    fn output_slot() -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(None))
    }

    fn sample_doc() -> Document {
        doc! {
            "_id": ObjectId::parse_str("64f1a2b3c4d5e6f789012345").unwrap(),
            "createdAt": DateTime::from_millis(1_700_000_000_123),
            "price": Bson::Decimal128(Decimal128::from_str("12.34").unwrap()),
            "payload": Bson::Binary(Binary {
                subtype: BinarySubtype::Generic,
                bytes: vec![1, 2, 3, 4],
            }),
        }
    }

    fn parse_doc(value: &serde_json::Value) -> Document {
        let bson = bson::to_bson(value).unwrap();
        match bson {
            Bson::Document(doc) => doc,
            other => panic!("expected document, got {other:?}"),
        }
    }

    fn assert_bson_types_round_trip(doc: &Document) {
        assert!(matches!(doc.get("_id"), Some(Bson::ObjectId(_))));
        assert!(matches!(doc.get("createdAt"), Some(Bson::DateTime(_))));
        assert!(matches!(doc.get("price"), Some(Bson::Decimal128(_))));
        assert!(matches!(doc.get("payload"), Some(Bson::Binary(_))));
        assert_eq!(doc.get("_id"), sample_doc().get("_id"));
        assert_eq!(doc.get("createdAt"), sample_doc().get("createdAt"));
        assert_eq!(doc.get("price"), sample_doc().get("price"));
        assert_eq!(doc.get("payload"), sample_doc().get("payload"));
    }

    #[tokio::test]
    async fn canonical_json_array_round_trips_bson_types() {
        let slot = output_slot();
        let mut sink = JsonSink::new(
            WriteSink::Plain(crate::mongo::import_export::io_util::WriteTarget::Buffer(Vec::new())),
            slot.clone(),
            JsonShape::Array,
            true,
        );

        sink.start().await.unwrap();
        sink.write(sample_doc()).await.unwrap();
        Box::new(sink).finish().await.unwrap();

        let text = slot.lock().unwrap().take().unwrap();
        let values: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(values.len(), 1);
        assert_bson_types_round_trip(&parse_doc(&values[0]));
    }

    #[tokio::test]
    async fn relaxed_ndjson_round_trips_bson_types() {
        let slot = output_slot();
        let mut sink = JsonSink::new(
            WriteSink::Plain(crate::mongo::import_export::io_util::WriteTarget::Buffer(Vec::new())),
            slot.clone(),
            JsonShape::Ndjson,
            false,
        );

        sink.start().await.unwrap();
        sink.write(sample_doc()).await.unwrap();
        Box::new(sink).finish().await.unwrap();

        let text = slot.lock().unwrap().take().unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1);
        let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_bson_types_round_trip(&parse_doc(&value));
    }
}
