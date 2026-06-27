//! CSV import source with header/no-header modes and light type inference.

use async_trait::async_trait;
use bson::{Bson, Document};
use csv::StringRecord;
use std::io::{Cursor, Read};
use std::path::Path;

use super::core::{DocumentSource, RowError, RowResult};
use crate::error::{AppError, AppResult};

pub struct CsvSource {
    reader: csv::Reader<Box<dyn Read + Send>>,
    columns: Vec<String>,
    pending: Option<StringRecord>,
    row: u64,
}

impl CsvSource {
    pub fn from_path(path: &Path, delimiter: u8, headers: bool) -> AppResult<Self> {
        let file = std::fs::File::open(path)?;
        Self::from_reader(Box::new(file), delimiter, headers)
    }

    pub fn from_text(text: String, delimiter: u8, headers: bool) -> AppResult<Self> {
        Self::from_reader(Box::new(Cursor::new(text.into_bytes())), delimiter, headers)
    }

    fn from_reader(reader: Box<dyn Read + Send>, delimiter: u8, headers: bool) -> AppResult<Self> {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(headers)
            .from_reader(reader);

        let (columns, pending, row) = if headers {
            let columns = reader
                .headers()
                .map_err(|e| AppError::Validation(format!("invalid CSV headers: {e}")))?
                .iter()
                .enumerate()
                .map(|(idx, name)| {
                    if name.trim().is_empty() {
                        format!("field_{}", idx + 1)
                    } else {
                        name.trim().to_string()
                    }
                })
                .collect();
            (columns_dedup(columns), None, 1)
        } else {
            let mut first = StringRecord::new();
            if reader
                .read_record(&mut first)
                .map_err(|e| AppError::Validation(format!("invalid CSV row: {e}")))?
            {
                let columns = (0..first.len())
                    .map(|idx| format!("field_{}", idx + 1))
                    .collect();
                (columns, Some(first), 0)
            } else {
                (Vec::new(), None, 0)
            }
        };

        Ok(Self {
            reader,
            columns,
            pending,
            row,
        })
    }
}

#[async_trait]
impl DocumentSource for CsvSource {
    fn size_hint(&self) -> Option<u64> {
        None
    }

    async fn next_doc(&mut self) -> AppResult<Option<RowResult>> {
        let record = if let Some(record) = self.pending.take() {
            record
        } else {
            let mut record = StringRecord::new();
            match self.reader.read_record(&mut record) {
                Ok(true) => record,
                Ok(false) => return Ok(None),
                Err(e) => {
                    self.row += 1;
                    return Ok(Some(RowResult::Error(RowError {
                        row: Some(self.row),
                        message: e.to_string(),
                    })));
                }
            }
        };

        self.row += 1;
        if self.columns.is_empty() {
            self.columns = (0..record.len())
                .map(|idx| format!("field_{}", idx + 1))
                .collect();
        }

        let mut doc = Document::new();
        for (idx, cell) in record.iter().enumerate() {
            let key = self
                .columns
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("field_{}", idx + 1));
            doc.insert(key, bson_value_from_cell(cell));
        }
        Ok(Some(RowResult::Doc(doc)))
    }
}

pub fn bson_value_from_cell(cell: &str) -> Bson {
    let trimmed = cell.trim();
    if trimmed.is_empty() {
        return Bson::Null;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Bson::Boolean(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Bson::Boolean(false);
    }
    if let Ok(n) = trimmed.parse::<i32>() {
        return Bson::Int32(n);
    }
    if let Ok(n) = trimmed.parse::<i64>() {
        return Bson::Int64(n);
    }
    if (trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E'))
        && trimmed.parse::<f64>().is_ok()
    {
        return Bson::Double(trimmed.parse::<f64>().unwrap());
    }
    if matches!(trimmed.as_bytes().first(), Some(b'{') | Some(b'[')) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Ok(bson) = bson::to_bson(&value) {
                return bson;
            }
        }
    }
    Bson::String(cell.to_string())
}

fn columns_dedup(columns: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    columns
        .into_iter()
        .map(|name| {
            let count = seen.entry(name.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                name
            } else {
                format!("{name}_{count}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::Bson;

    #[tokio::test]
    async fn parses_csv_with_headers_and_types() {
        let mut src = CsvSource::from_text(
            "name,n,ok,nested\nAda,42,true,\"{\"\"city\"\":\"\"NYC\"\"}\"\n".into(),
            b',',
            true,
        )
        .unwrap();
        let row = src.next_doc().await.unwrap().unwrap();
        let RowResult::Doc(doc) = row else {
            panic!("expected doc");
        };
        assert_eq!(doc.get_str("name").unwrap(), "Ada");
        assert_eq!(doc.get_i32("n").unwrap(), 42);
        assert!(doc.get_bool("ok").unwrap());
        assert!(matches!(doc.get("nested"), Some(Bson::Document(_))));
    }

    #[tokio::test]
    async fn parses_csv_without_headers() {
        let mut src = CsvSource::from_text("Ada,42\n".into(), b',', false).unwrap();
        let RowResult::Doc(doc) = src.next_doc().await.unwrap().unwrap() else {
            panic!("expected doc");
        };
        assert_eq!(doc.get_str("field_1").unwrap(), "Ada");
        assert_eq!(doc.get_i32("field_2").unwrap(), 42);
    }
}
