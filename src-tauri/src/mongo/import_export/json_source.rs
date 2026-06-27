//! JSON import source for a single object, JSON arrays, and NDJSON.

use async_trait::async_trait;
use bson::{Bson, Document};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Cursor, Read};
use std::path::Path;

use super::core::{DocumentSource, RowError, RowResult};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JsonImportShape {
    Object,
    Array,
    Ndjson,
}

pub struct JsonSource {
    inner: JsonSourceInner,
    total: Option<u64>,
}

enum JsonSourceInner {
    Rows(VecDeque<RowResult>),
    Array {
        reader: JsonArrayReader,
        row: u64,
    },
    Ndjson {
        reader: Box<dyn BufRead + Send>,
        line: String,
        row: u64,
    },
}

impl JsonSource {
    pub fn from_path(path: &Path, shape: JsonImportShape) -> AppResult<Self> {
        let file = std::fs::File::open(path)?;
        Self::from_reader(Box::new(file), shape)
    }

    pub fn from_text(text: String, shape: JsonImportShape) -> AppResult<Self> {
        Self::from_reader(Box::new(Cursor::new(text.into_bytes())), shape)
    }

    fn from_reader(reader: Box<dyn Read + Send>, shape: JsonImportShape) -> AppResult<Self> {
        match shape {
            JsonImportShape::Object => {
                let value: serde_json::Value = serde_json::from_reader(reader)?;
                let row = doc_from_json_value(value)
                    .map(RowResult::Doc)
                    .unwrap_or_else(|e| row_error(1, e.to_string()));
                Ok(Self {
                    inner: JsonSourceInner::Rows(VecDeque::from([row])),
                    total: Some(1),
                })
            }
            JsonImportShape::Array => Ok(Self {
                inner: JsonSourceInner::Array {
                    reader: JsonArrayReader::new(Box::new(BufReader::new(reader))),
                    row: 0,
                },
                total: None,
            }),
            JsonImportShape::Ndjson => Ok(Self {
                inner: JsonSourceInner::Ndjson {
                    reader: Box::new(BufReader::new(reader)),
                    line: String::new(),
                    row: 0,
                },
                total: None,
            }),
        }
    }
}

#[async_trait]
impl DocumentSource for JsonSource {
    fn size_hint(&self) -> Option<u64> {
        self.total
    }

    async fn next_doc(&mut self) -> AppResult<Option<RowResult>> {
        match &mut self.inner {
            JsonSourceInner::Rows(rows) => Ok(rows.pop_front()),
            JsonSourceInner::Array { reader, row } => {
                let raw = match reader.next_raw()? {
                    Some(raw) => raw,
                    None => return Ok(None),
                };
                *row += 1;
                let value = match serde_json::from_slice::<serde_json::Value>(&raw) {
                    Ok(value) => value,
                    Err(e) => return Ok(Some(row_error(*row, e.to_string()))),
                };
                Ok(Some(
                    doc_from_json_value(value)
                        .map(RowResult::Doc)
                        .unwrap_or_else(|e| row_error(*row, e.to_string())),
                ))
            }
            JsonSourceInner::Ndjson { reader, line, row } => loop {
                line.clear();
                let read = reader.read_line(line)?;
                if read == 0 {
                    return Ok(None);
                }
                *row += 1;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let value = match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(value) => value,
                    Err(e) => return Ok(Some(row_error(*row, e.to_string()))),
                };
                return Ok(Some(
                    doc_from_json_value(value)
                        .map(RowResult::Doc)
                        .unwrap_or_else(|e| row_error(*row, e.to_string())),
                ));
            },
        }
    }
}

/// Streams a top-level JSON array element-by-element, holding only one element's
/// raw bytes in memory at a time. Each element is returned as raw UTF-8 bytes for
/// the caller to parse, so structural malformations inside a single element are
/// reported as row errors rather than aborting the whole import.
struct JsonArrayReader {
    reader: Box<dyn BufRead + Send>,
    peeked: Option<u8>,
    opened: bool,
    finished: bool,
}

impl JsonArrayReader {
    fn new(reader: Box<dyn BufRead + Send>) -> Self {
        Self {
            reader,
            peeked: None,
            opened: false,
            finished: false,
        }
    }

    fn read_byte(&mut self) -> AppResult<Option<u8>> {
        if let Some(b) = self.peeked.take() {
            return Ok(Some(b));
        }
        let mut buf = [0u8; 1];
        let n = self.reader.read(&mut buf)?;
        Ok(if n == 0 { None } else { Some(buf[0]) })
    }

    fn peek_byte(&mut self) -> AppResult<Option<u8>> {
        if self.peeked.is_none() {
            let mut buf = [0u8; 1];
            if self.reader.read(&mut buf)? != 0 {
                self.peeked = Some(buf[0]);
            }
        }
        Ok(self.peeked)
    }

    fn skip_ws(&mut self) -> AppResult<()> {
        while let Some(b) = self.peek_byte()? {
            if b.is_ascii_whitespace() {
                self.read_byte()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Returns the raw bytes of the next array element, or `None` at end of array.
    fn next_raw(&mut self) -> AppResult<Option<Vec<u8>>> {
        if self.finished {
            return Ok(None);
        }
        if !self.opened {
            self.skip_ws()?;
            match self.read_byte()? {
                Some(b'[') => self.opened = true,
                Some(other) => {
                    return Err(AppError::Validation(format!(
                        "expected JSON array, found '{}'",
                        other as char
                    )))
                }
                None => {
                    return Err(AppError::Validation(
                        "expected JSON array, found empty input".into(),
                    ))
                }
            }
        } else {
            self.skip_ws()?;
            match self.peek_byte()? {
                Some(b',') => {
                    self.read_byte()?;
                }
                Some(b']') => {
                    self.read_byte()?;
                    self.finished = true;
                    return Ok(None);
                }
                Some(_) => {}
                None => {
                    return Err(AppError::Validation("unterminated JSON array".into()));
                }
            }
        }
        self.skip_ws()?;
        if let Some(b']') = self.peek_byte()? {
            self.read_byte()?;
            self.finished = true;
            return Ok(None);
        }
        self.scan_value().map(Some)
    }

    /// Scans a single JSON value at the current position into raw bytes, tracking
    /// brace/bracket depth and string state. Leaves the trailing `,` or `]` for
    /// the next call.
    fn scan_value(&mut self) -> AppResult<Vec<u8>> {
        let mut buf = Vec::new();
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escaped = false;
        loop {
            let b = match self.peek_byte()? {
                Some(b) => b,
                None => {
                    if depth == 0 && !in_string && !buf.is_empty() {
                        return Ok(buf);
                    }
                    return Err(AppError::Validation(
                        "unterminated JSON value in array".into(),
                    ));
                }
            };
            if in_string {
                buf.push(b);
                self.read_byte()?;
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                    if depth == 0 {
                        return Ok(buf);
                    }
                }
                continue;
            }
            match b {
                b'"' => {
                    in_string = true;
                    buf.push(b);
                    self.read_byte()?;
                }
                b'{' | b'[' => {
                    depth += 1;
                    buf.push(b);
                    self.read_byte()?;
                }
                b'}' | b']' => {
                    if depth == 0 {
                        return Ok(buf);
                    }
                    depth -= 1;
                    buf.push(b);
                    self.read_byte()?;
                    if depth == 0 {
                        return Ok(buf);
                    }
                }
                b',' => {
                    if depth == 0 {
                        return Ok(buf);
                    }
                    buf.push(b);
                    self.read_byte()?;
                }
                b' ' | b'\t' | b'\n' | b'\r' => {
                    if depth == 0 {
                        return Ok(buf);
                    }
                    buf.push(b);
                    self.read_byte()?;
                }
                _ => {
                    buf.push(b);
                    self.read_byte()?;
                }
            }
        }
    }
}

pub fn doc_from_json_value(value: serde_json::Value) -> AppResult<Document> {
    let bson = bson::to_bson(&value)?;
    match bson {
        Bson::Document(doc) => Ok(doc),
        other => Err(AppError::InvalidBson(format!(
            "import row must be a JSON object, got {}",
            bson_type_name(&other)
        ))),
    }
}

fn bson_type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Array(_) => "array",
        Bson::Boolean(_) => "bool",
        Bson::Double(_) => "double",
        Bson::Int32(_) => "int32",
        Bson::Int64(_) => "int64",
        Bson::Null => "null",
        Bson::String(_) => "string",
        _ => "bson",
    }
}

fn row_error(row: u64, message: String) -> RowResult {
    RowResult::Error(RowError {
        row: Some(row),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Bson, DateTime};

    #[tokio::test]
    async fn parses_object_array_and_ndjson() {
        let mut object =
            JsonSource::from_text(r#"{"a":1}"#.into(), JsonImportShape::Object).unwrap();
        assert!(matches!(
            object.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));

        let mut array =
            JsonSource::from_text(r#"[{"a":1},{"a":2}]"#.into(), JsonImportShape::Array).unwrap();
        assert!(matches!(
            array.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
        assert!(matches!(
            array.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
        assert!(array.next_doc().await.unwrap().is_none());

        let mut ndjson =
            JsonSource::from_text("{\"a\":1}\n{\"a\":2}\n".into(), JsonImportShape::Ndjson)
                .unwrap();
        assert!(matches!(
            ndjson.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
        assert!(matches!(
            ndjson.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
    }

    #[tokio::test]
    async fn reports_malformed_ndjson_rows_without_aborting_source() {
        let mut src = JsonSource::from_text(
            "{\"a\":1}\nnope\n{\"a\":2}\n".into(),
            JsonImportShape::Ndjson,
        )
        .unwrap();
        assert!(matches!(
            src.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
        assert!(matches!(
            src.next_doc().await.unwrap(),
            Some(RowResult::Error(_))
        ));
        assert!(matches!(
            src.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
    }

    #[tokio::test]
    async fn streams_array_with_nested_docs_and_reports_bad_elements() {
        let mut src = JsonSource::from_text(
            r#"[ {"a": {"b": [1,2]}} , 5 , {"c": "x,]y"} ]"#.into(),
            JsonImportShape::Array,
        )
        .unwrap();
        // nested object/array element parses fine
        assert!(matches!(
            src.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
        // scalar element is not a JSON object -> row error, source continues
        assert!(matches!(
            src.next_doc().await.unwrap(),
            Some(RowResult::Error(_))
        ));
        // string containing comma and bracket must not break element splitting
        assert!(matches!(
            src.next_doc().await.unwrap(),
            Some(RowResult::Doc(_))
        ));
        assert!(src.next_doc().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn empty_array_yields_no_rows() {
        let mut src = JsonSource::from_text("[]".into(), JsonImportShape::Array).unwrap();
        assert!(src.next_doc().await.unwrap().is_none());
    }

    #[test]
    fn extended_json_preserves_bson_types() {
        let doc = doc_from_json_value(serde_json::json!({
            "_id": {"$oid": "64f1a2b3c4d5e6f789012345"},
            "createdAt": {"$date": {"$numberLong": "1700000000123"}}
        }))
        .unwrap();
        assert!(matches!(doc.get("_id"), Some(Bson::ObjectId(_))));
        assert!(matches!(doc.get("createdAt"), Some(Bson::DateTime(_))));
        assert_eq!(
            doc.get_datetime("createdAt").unwrap(),
            &DateTime::from_millis(1_700_000_000_123)
        );
    }
}
