//! CSV document sink. Columns are fixed up front (provided by the caller or
//! derived from a schema sample) so output is deterministic and streamable.
//!
//! CSV cannot represent all BSON types losslessly. The cell-encoding rules are
//! explicit: scalars render natively; ObjectId/Date/Decimal128 render as their
//! display string; nested objects/arrays and binary render as compact relaxed
//! Extended JSON. The export wizard surfaces this tradeoff to the user.

use async_trait::async_trait;
use bson::{Bson, Document};
use std::sync::{Arc, Mutex};

use super::core::DocumentSink;
use super::io_util::WriteSink;
use crate::error::{AppError, AppResult};

pub struct CsvSink {
    target: Option<WriteSink>,
    output_slot: Arc<Mutex<Option<String>>>,
    columns: Vec<String>,
    delimiter: u8,
    write_headers: bool,
}

impl CsvSink {
    pub fn new(
        target: WriteSink,
        output_slot: Arc<Mutex<Option<String>>>,
        columns: Vec<String>,
        delimiter: u8,
        write_headers: bool,
    ) -> Self {
        Self {
            target: Some(target),
            output_slot,
            columns,
            delimiter,
            write_headers,
        }
    }

    fn target_mut(&mut self) -> AppResult<&mut WriteSink> {
        self.target
            .as_mut()
            .ok_or_else(|| AppError::Internal("csv sink already finalized".into()))
    }

    fn encode_record(delimiter: u8, fields: &[String]) -> AppResult<Vec<u8>> {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(delimiter)
            .from_writer(Vec::with_capacity(256));
        wtr.write_record(fields)
            .map_err(|e| AppError::Internal(format!("csv encode error: {e}")))?;
        wtr.flush()
            .map_err(|e| AppError::Internal(format!("csv flush error: {e}")))?;
        wtr.into_inner()
            .map_err(|e| AppError::Internal(format!("csv writer error: {e}")))
    }
}

#[async_trait]
impl DocumentSink for CsvSink {
    async fn start(&mut self) -> AppResult<()> {
        if self.write_headers {
            let delimiter = self.delimiter;
            let headers = self.columns.clone();
            let bytes = Self::encode_record(delimiter, &headers)?;
            self.target_mut()?.write_all(&bytes)?;
        }
        Ok(())
    }

    async fn write(&mut self, doc: Document) -> AppResult<()> {
        let delimiter = self.delimiter;
        let fields: Vec<String> = self
            .columns
            .iter()
            .map(|col| csv_cell(get_bson_path(&doc, col)))
            .collect();
        let bytes = Self::encode_record(delimiter, &fields)?;
        self.target_mut()?.write_all(&bytes)?;
        Ok(())
    }

    async fn finish(mut self: Box<Self>) -> AppResult<()> {
        let target = self
            .target
            .take()
            .ok_or_else(|| AppError::Internal("csv sink already finalized".into()))?;
        if let Some(text) = target.finish()? {
            *self
                .output_slot
                .lock()
                .map_err(|_| AppError::Internal("export output slot mutex poisoned".into()))? =
                Some(text);
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

/// Walk a dotted path (`a.b.c`) into a document, returning the leaf value.
fn get_bson_path<'a>(doc: &'a Document, path: &str) -> Option<&'a Bson> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = doc.get(first)?;
    for part in parts {
        match current {
            Bson::Document(d) => current = d.get(part)?,
            _ => return None,
        }
    }
    Some(current)
}

/// Render a BSON value as a CSV cell. Lossy by design for complex types.
fn csv_cell(value: Option<&Bson>) -> String {
    match value {
        None | Some(Bson::Null) => String::new(),
        Some(Bson::String(s)) => s.clone(),
        Some(Bson::Int32(n)) => n.to_string(),
        Some(Bson::Int64(n)) => n.to_string(),
        Some(Bson::Double(n)) => n.to_string(),
        Some(Bson::Boolean(b)) => b.to_string(),
        Some(Bson::ObjectId(oid)) => oid.to_hex(),
        Some(Bson::DateTime(dt)) => dt
            .try_to_rfc3339_string()
            .unwrap_or_else(|_| dt.timestamp_millis().to_string()),
        Some(Bson::Decimal128(d)) => d.to_string(),
        Some(other) => {
            let json = other.clone().into_relaxed_extjson();
            serde_json::to_string(&json).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::spec::BinarySubtype;
    use bson::{doc, Binary, Bson, DateTime, Decimal128};
    use std::str::FromStr;

    #[test]
    fn cell_renders_scalars_and_objectid() {
        let oid = bson::oid::ObjectId::new();
        let d = doc! { "n": 42i32, "s": "hi", "b": true, "_id": oid };
        assert_eq!(csv_cell(get_bson_path(&d, "n")), "42");
        assert_eq!(csv_cell(get_bson_path(&d, "s")), "hi");
        assert_eq!(csv_cell(get_bson_path(&d, "b")), "true");
        assert_eq!(csv_cell(get_bson_path(&d, "_id")), oid.to_hex());
        assert_eq!(csv_cell(get_bson_path(&d, "missing")), "");
    }

    #[test]
    fn cell_renders_nested_as_json() {
        let d = doc! { "addr": { "city": "NYC" }, "tags": ["a", "b"] };
        assert_eq!(csv_cell(get_bson_path(&d, "addr.city")), "NYC");
        let obj = csv_cell(get_bson_path(&d, "addr"));
        assert!(obj.contains("city") && obj.contains("NYC"));
        let arr = csv_cell(get_bson_path(&d, "tags"));
        assert!(arr.contains('a') && arr.contains('b'));
    }

    #[test]
    fn encode_record_escapes_delimiters() {
        let bytes = CsvSink::encode_record(b',', &["a,b".to_string(), "c".to_string()]).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("\"a,b\","));
    }

    #[tokio::test]
    async fn csv_sink_round_trips_declared_lossy_cells() {
        let output = Arc::new(Mutex::new(None));
        let oid = bson::oid::ObjectId::parse_str("64f1a2b3c4d5e6f789012345").unwrap();
        let dt = DateTime::from_millis(1_700_000_000_123);
        let decimal = Decimal128::from_str("12.34").unwrap();
        let doc = doc! {
            "_id": oid,
            "name": "Ada",
            "createdAt": dt,
            "price": Bson::Decimal128(decimal),
            "nested": { "city": "NYC" },
            "payload": Bson::Binary(Binary {
                subtype: BinarySubtype::Generic,
                bytes: vec![1, 2, 3],
            }),
        };
        let mut sink = CsvSink::new(
            WriteSink::Plain(crate::mongo::import_export::io_util::WriteTarget::Buffer(Vec::new())),
            output.clone(),
            vec![
                "_id".into(),
                "name".into(),
                "createdAt".into(),
                "price".into(),
                "nested".into(),
                "payload".into(),
            ],
            b',',
            true,
        );

        sink.start().await.unwrap();
        sink.write(doc).await.unwrap();
        Box::new(sink).finish().await.unwrap();

        let text = output.lock().unwrap().take().unwrap();
        let mut reader = csv::Reader::from_reader(text.as_bytes());
        let headers = reader.headers().unwrap().clone();
        assert_eq!(
            headers.iter().collect::<Vec<_>>(),
            vec!["_id", "name", "createdAt", "price", "nested", "payload"]
        );

        let row = reader.records().next().unwrap().unwrap();
        let oid_text = oid.to_hex();
        let date_text = dt.try_to_rfc3339_string().unwrap();
        let decimal_text = decimal.to_string();
        assert_eq!(row.get(0), Some(oid_text.as_str()));
        assert_eq!(row.get(1), Some("Ada"));
        assert_eq!(row.get(2), Some(date_text.as_str()));
        assert_eq!(row.get(3), Some(decimal_text.as_str()));
        assert!(row.get(4).unwrap().contains("\"city\":\"NYC\""));
        assert!(row.get(5).unwrap().contains("$binary"));
    }
}
