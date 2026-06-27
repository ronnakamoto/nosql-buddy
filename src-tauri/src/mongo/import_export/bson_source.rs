//! BSON file source: reads a sequence of length-prefixed BSON documents
//! from a file. Compatible with `mongodump` `.bson` output.

use async_trait::async_trait;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::error::{AppError, AppResult};
use super::core::{DocumentSource, RowResult};

pub struct BsonSource {
    reader: BufReader<File>,
}

impl BsonSource {
    pub fn from_path(path: &Path) -> AppResult<Self> {
        let file = File::open(path)?;
        Ok(Self {
            reader: BufReader::with_capacity(64 * 1024, file),
        })
    }
}

#[async_trait]
impl DocumentSource for BsonSource {
    fn size_hint(&self) -> Option<u64> {
        None
    }

    async fn next_doc(&mut self) -> AppResult<Option<RowResult>> {
        let mut len_bytes = [0u8; 4];
        match self.reader.read_exact(&mut len_bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(AppError::Io(e.to_string())),
        }
        let len = i32::from_le_bytes(len_bytes) as usize;
        if len < 5 {
            return Err(AppError::Validation("invalid BSON document length".into()));
        }
        let mut buf = vec![0u8; len];
        buf[0..4].copy_from_slice(&len_bytes);
        self.reader.read_exact(&mut buf[4..])?;
        let doc = bson::from_slice(&buf)?;
        Ok(Some(RowResult::Doc(doc)))
    }
}
