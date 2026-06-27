//! BSON file source: reads a sequence of length-prefixed BSON documents
//! from a file. Compatible with `mongodump` `.bson` output.
//!
//! Automatically decompresses `.bson.gz` (gzip) and `.bson.zst` (zstd) files.

use async_trait::async_trait;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::error::{AppError, AppResult};
use super::core::{DocumentSource, RowResult};

enum BsonReader {
    Plain(BufReader<File>),
    Gzip(flate2::read::GzDecoder<BufReader<File>>),
    Zstd(zstd::stream::read::Decoder<'static, BufReader<File>>),
}

impl Read for BsonReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            BsonReader::Plain(r) => r.read(buf),
            BsonReader::Gzip(r) => r.read(buf),
            BsonReader::Zstd(r) => r.read(buf),
        }
    }
}

pub struct BsonSource {
    reader: BsonReader,
}

impl BsonSource {
    pub fn from_path(path: &Path) -> AppResult<Self> {
        let file = File::open(path)?;
        let buffered = BufReader::with_capacity(64 * 1024, file);
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        let reader = if ext == "gz" && stem.ends_with(".bson") {
            BsonReader::Gzip(flate2::read::GzDecoder::new(buffered))
        } else if ext == "zst" && stem.ends_with(".bson") {
            BsonReader::Zstd(zstd::stream::read::Decoder::new(File::open(path)?).map_err(|e| {
                AppError::Io(format!("zstd decoder init failed: {e}"))
            })?)
        } else {
            BsonReader::Plain(buffered)
        };

        Ok(Self { reader })
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

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{doc, oid::ObjectId};
    use std::str::FromStr;

    fn sample_docs() -> Vec<bson::Document> {
        vec![
            doc! { "_id": ObjectId::from_str("64f1a2b3c4d5e6f789012345").unwrap(), "name": "Alice", "age": 30 },
            doc! { "_id": ObjectId::from_str("64f1a2b3c4d5e6f789012346").unwrap(), "name": "Bob", "age": 25 },
        ]
    }

    fn write_plain_bson(path: &Path, docs: &[bson::Document]) {
        use std::io::Write;
        let mut file = std::fs::File::create(path).unwrap();
        for doc in docs {
            let bytes = bson::to_vec(doc).unwrap();
            file.write_all(&bytes).unwrap();
        }
    }

    fn write_gzip_bson(path: &Path, docs: &[bson::Document]) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        for doc in docs {
            let bytes = bson::to_vec(doc).unwrap();
            encoder.write_all(&bytes).unwrap();
        }
        encoder.finish().unwrap();
    }

    fn write_zstd_bson(path: &Path, docs: &[bson::Document]) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(file, 3).unwrap();
        for doc in docs {
            let bytes = bson::to_vec(doc).unwrap();
            encoder.write_all(&bytes).unwrap();
        }
        encoder.finish().unwrap();
    }

    async fn assert_source_reads_docs(source: &mut BsonSource, expected: &[bson::Document]) {
        for expected_doc in expected {
            let result = source.next_doc().await.unwrap().unwrap();
            if let RowResult::Doc(doc) = result {
                assert_eq!(doc.get_str("name").unwrap(), expected_doc.get_str("name").unwrap());
                assert_eq!(doc.get_i32("age").unwrap(), expected_doc.get_i32("age").unwrap());
            } else {
                panic!("expected doc");
            }
        }
        assert!(source.next_doc().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn plain_bson_round_trips() {
        let path = std::env::temp_dir().join(format!("mongo-buddy-bson-source-plain-{}", uuid::Uuid::new_v4()));
        let docs = sample_docs();
        write_plain_bson(&path, &docs);

        let mut source = BsonSource::from_path(&path).unwrap();
        assert_source_reads_docs(&mut source, &docs).await;

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn gzip_bson_round_trips() {
        let path = std::env::temp_dir().join(format!("test.{}.bson.gz", uuid::Uuid::new_v4()));
        let docs = sample_docs();
        write_gzip_bson(&path, &docs);

        let mut source = BsonSource::from_path(&path).unwrap();
        assert_source_reads_docs(&mut source, &docs).await;

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn zstd_bson_round_trips() {
        let path = std::env::temp_dir().join(format!("test.{}.bson.zst", uuid::Uuid::new_v4()));
        let docs = sample_docs();
        write_zstd_bson(&path, &docs);

        let mut source = BsonSource::from_path(&path).unwrap();
        assert_source_reads_docs(&mut source, &docs).await;

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn compressed_with_bson_extension_round_trips() {
        // Files named like mongodump output: users.bson.gz
        let path = std::env::temp_dir().join(format!("users.{}.bson.gz", uuid::Uuid::new_v4()));
        let docs = sample_docs();
        write_gzip_bson(&path, &docs);

        let mut source = BsonSource::from_path(&path).unwrap();
        assert_source_reads_docs(&mut source, &docs).await;

        let _ = std::fs::remove_file(&path);
    }
}
