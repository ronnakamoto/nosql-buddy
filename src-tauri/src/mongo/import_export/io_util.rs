//! Atomic file writes and target-path validation for exports.
//!
//! All export bytes are written by Rust (never via the JS fs plugin), so the
//! frontend only ever passes a path string chosen via the native dialog. To
//! keep the same security posture as the app's `fs:scope`, the chosen path
//! must resolve under one of the user's standard directories. Output is
//! written to a sibling `.part` file and atomically renamed on success, so a
//! cancelled or failed export never leaves a half-written file behind.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

/// Validate that `path` resolves under an allowed root (Documents, Downloads,
/// Desktop, or Home). Returns the validated absolute path.
pub fn validate_target_path(path: &str) -> AppResult<PathBuf> {
    validate_user_path(path, "export", false)
}

/// Validate an import source file path. The file must already exist and live under
/// the same user roots as export targets.
pub fn validate_source_path(path: &str) -> AppResult<PathBuf> {
    validate_user_path(path, "import", true)
}

/// Validate a restore source directory path. The directory must already exist and live under
/// the same user roots as export targets.
pub fn validate_source_dir(path: &str) -> AppResult<PathBuf> {
    let target = PathBuf::from(path);
    let parent = target
        .parent()
        .ok_or_else(|| AppError::Validation("restore path has no parent directory".into()))?;
    let parent_canon = parent
        .canonicalize()
        .map_err(|e| AppError::Validation(format!("invalid restore directory: {e}")))?;

    let allowed: Vec<PathBuf> = [
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::desktop_dir(),
        dirs::home_dir(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|p| p.canonicalize().ok())
    .collect();

    let permitted = allowed.iter().any(|root| parent_canon.starts_with(root));
    if !permitted {
        return Err(AppError::Validation(
            "restore path must be under your Documents, Downloads, Desktop, or Home directory".into(),
        ));
    }
    if !target.is_dir() {
        return Err(AppError::Validation(
            "restore source directory does not exist".into(),
        ));
    }
    Ok(target)
}

fn validate_user_path(path: &str, operation: &str, must_exist: bool) -> AppResult<PathBuf> {
    let target = PathBuf::from(path);
    let parent = target
        .parent()
        .ok_or_else(|| AppError::Validation(format!("{operation} path has no parent directory")))?;
    // The parent must exist so we can canonicalize it; the file itself need not.
    let parent_canon = parent
        .canonicalize()
        .map_err(|e| AppError::Validation(format!("invalid {operation} directory: {e}")))?;

    let allowed: Vec<PathBuf> = [
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::desktop_dir(),
        dirs::home_dir(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|p| p.canonicalize().ok())
    .collect();

    let permitted = allowed.iter().any(|root| parent_canon.starts_with(root));
    if !permitted {
        return Err(AppError::Validation(format!(
            "{operation} path must be under your Documents, Downloads, Desktop, or Home directory"
        )));
    }
    if must_exist && !target.is_file() {
        return Err(AppError::Validation(format!(
            "{operation} source file does not exist"
        )));
    }
    Ok(target)
}

/// A buffered writer that writes to `<final>.part` and atomically renames to
/// `<final>` on commit. Dropping without committing leaves the `.part` file;
/// call [`AtomicFileWriter::abort`] to remove it.
pub struct AtomicFileWriter {
    final_path: PathBuf,
    part_path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl AtomicFileWriter {
    pub fn create(final_path: PathBuf) -> AppResult<Self> {
        let part_path = part_path_for(&final_path);
        let file = File::create(&part_path)?;
        Ok(Self {
            final_path,
            part_path,
            writer: Some(BufWriter::with_capacity(64 * 1024, file)),
        })
    }

    /// Borrow the underlying writer for synchronous writes.
    pub fn writer(&mut self) -> AppResult<&mut BufWriter<File>> {
        self.writer
            .as_mut()
            .ok_or_else(|| AppError::Internal("export writer already finalized".into()))
    }

    /// Flush, sync, and atomically rename `.part` to the final path.
    pub fn commit(mut self) -> AppResult<()> {
        use std::io::Write;
        if let Some(mut w) = self.writer.take() {
            w.flush()?;
            let file = w
                .into_inner()
                .map_err(|e| AppError::Io(format!("flush failed: {e}")))?;
            file.sync_all()?;
        }
        std::fs::rename(&self.part_path, &self.final_path)?;
        Ok(())
    }

    /// Discard the partial file.
    pub fn abort(mut self) {
        self.writer.take();
        let _ = std::fs::remove_file(&self.part_path);
    }
}

pub(crate) fn part_path_for(final_path: &Path) -> PathBuf {
    let mut os = final_path.as_os_str().to_owned();
    os.push(".part");
    PathBuf::from(os)
}

/// A write destination for a sink: either an on-disk atomic file or an
/// in-memory buffer (used for clipboard exports). Both expose synchronous
/// `write_all`; `commit`/`abort` are no-ops for the buffer.
pub enum WriteTarget {
    File(AtomicFileWriter),
    Buffer(Vec<u8>),
}

impl WriteTarget {
    pub fn write_all(&mut self, bytes: &[u8]) -> AppResult<()> {
        use std::io::Write;
        match self {
            WriteTarget::File(f) => {
                f.writer()?.write_all(bytes)?;
                Ok(())
            }
            WriteTarget::Buffer(buf) => {
                buf.extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    /// Commit the target. For a file, performs the atomic rename and returns
    /// `None`. For a buffer, returns the accumulated UTF-8 text.
    pub fn commit(self) -> AppResult<Option<String>> {
        match self {
            WriteTarget::File(f) => {
                f.commit()?;
                Ok(None)
            }
            WriteTarget::Buffer(buf) => {
                let text = String::from_utf8(buf)
                    .map_err(|e| AppError::Internal(format!("invalid utf-8 in export: {e}")))?;
                Ok(Some(text))
            }
        }
    }

    pub fn abort(self) {
        if let WriteTarget::File(f) = self {
            f.abort();
        }
    }
}

/// Compression kind for archives and file exports.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CompressionKind {
    None,
    Gzip,
    Zstd,
}

impl CompressionKind {
    pub fn extension(&self) -> &'static str {
        match self {
            CompressionKind::None => "",
            CompressionKind::Gzip => ".gz",
            CompressionKind::Zstd => ".zst",
        }
    }
}

/// A transparent [`std::io::Write`] adapter around [`WriteTarget`] so it can be
/// fed to `flate2` / `zstd` encoders.
struct DirectWriter(WriteTarget);

impl std::io::Write for DirectWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write_all(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.0 {
            WriteTarget::File(f) => {
                f.writer().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?.flush()
            }
            WriteTarget::Buffer(_) => Ok(()),
        }
    }
}

/// A wrapper that applies optional compression on top of a [`WriteTarget`].
/// All writes go through the compressor; `finish` flushes and returns the
/// underlying [`WriteTarget`] so it can be committed / aborted as usual.
pub struct CompressedWriter {
    encoder: Encoder,
}

enum Encoder {
    None(DirectWriter),
    Gzip(flate2::write::GzEncoder<DirectWriter>),
    Zstd(zstd::stream::write::Encoder<'static, DirectWriter>),
}

impl std::io::Write for Encoder {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Encoder::None(w) => w.write(buf),
            Encoder::Gzip(w) => w.write(buf),
            Encoder::Zstd(w) => w.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Encoder::None(w) => w.flush(),
            Encoder::Gzip(w) => w.flush(),
            Encoder::Zstd(w) => w.flush(),
        }
    }
}

impl CompressedWriter {
    pub fn new(target: WriteTarget, kind: CompressionKind) -> AppResult<Self> {
        let direct = DirectWriter(target);
        let encoder = match kind {
            CompressionKind::None => Encoder::None(direct),
            CompressionKind::Gzip => Encoder::Gzip(flate2::write::GzEncoder::new(
                direct,
                flate2::Compression::default(),
            )),
            CompressionKind::Zstd => Encoder::Zstd(
                zstd::stream::write::Encoder::new(direct, 3).map_err(|e| {
                    AppError::Io(format!("zstd encoder init failed: {e}"))
                })?,
            ),
        };
        Ok(Self { encoder })
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> AppResult<()> {
        use std::io::Write;
        self.encoder
            .write_all(bytes)
            .map_err(|e| AppError::Io(format!("compression write failed: {e}")))
    }

    /// Finish compression and return the inner [`WriteTarget`].
    pub fn finish(self) -> AppResult<WriteTarget> {
        match self.encoder {
            Encoder::None(direct) => Ok(direct.0),
            Encoder::Gzip(gz) => {
                let direct = gz.finish().map_err(|e| AppError::Io(format!("gzip finish: {e}")))?;
                Ok(direct.0)
            }
            Encoder::Zstd(zst) => {
                let direct = zst.finish().map_err(|e| AppError::Io(format!("zstd finish: {e}")))?;
                Ok(direct.0)
            }
        }
    }

    /// Abort without flushing. Returns the inner [`WriteTarget`] so it can be
    /// aborted in turn (removing the partial file).
    pub fn abort(self) -> WriteTarget {
        match self.encoder {
            Encoder::None(direct) => direct.0,
            Encoder::Gzip(gz) => {
                // Best-effort: try to finish; if it fails, drop the encoder
                // and return the inner target so the caller can clean up.
                match gz.finish() {
                    Ok(direct) => direct.0,
                    Err(_) => WriteTarget::Buffer(Vec::new()),
                }
            }
            Encoder::Zstd(zst) => {
                match zst.finish() {
                    Ok(direct) => direct.0,
                    Err(_) => WriteTarget::Buffer(Vec::new()),
                }
            }
        }
    }
}

/// A unified write target that may be plain or compressed.
/// Sinks use this so they do not need to branch on compression themselves.
pub enum WriteSink {
    Plain(WriteTarget),
    Compressed(CompressedWriter),
}

impl WriteSink {
    pub fn write_all(&mut self, bytes: &[u8]) -> AppResult<()> {
        match self {
            WriteSink::Plain(t) => t.write_all(bytes),
            WriteSink::Compressed(c) => c.write_all(bytes),
        }
    }

    pub fn finish(self) -> AppResult<Option<String>> {
        match self {
            WriteSink::Plain(t) => t.commit(),
            WriteSink::Compressed(c) => {
                let target = c.finish()?;
                target.commit()
            }
        }
    }

    pub fn abort(self) {
        match self {
            WriteSink::Plain(t) => t.abort(),
            WriteSink::Compressed(c) => c.abort().abort(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mongo-buddy-export-test-{nonce}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn atomic_writer_commits_part_file_to_final_path() {
        let path = unique_test_path("out.json");
        let part = part_path_for(&path);
        let mut writer = AtomicFileWriter::create(path.clone()).unwrap();
        writer.writer().unwrap().write_all(b"hello").unwrap();

        assert!(part.exists());
        writer.commit().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert!(!part.exists());
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn atomic_writer_abort_removes_part_file() {
        let path = unique_test_path("out.json");
        let part = part_path_for(&path);
        let mut writer = AtomicFileWriter::create(path.clone()).unwrap();
        writer.writer().unwrap().write_all(b"partial").unwrap();

        assert!(part.exists());
        writer.abort();

        assert!(!part.exists());
        assert!(!path.exists());
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn target_path_validation_allows_home_file_and_rejects_system_file() {
        let home = dirs::home_dir().unwrap();
        let allowed = home.join("mongo-buddy-export-validation.json");
        assert_eq!(
            validate_target_path(&allowed.to_string_lossy()).unwrap(),
            allowed
        );

        let denied = validate_target_path("/etc/mongo-buddy-export-validation.json");
        assert!(matches!(denied, Err(AppError::Validation(_))));
    }

    #[test]
    fn compressed_writer_none_round_trips() {
        let path = unique_test_path("plain.txt");
        let target = WriteTarget::File(AtomicFileWriter::create(path.clone()).unwrap());
        let mut writer = CompressedWriter::new(target, CompressionKind::None).unwrap();
        writer.write_all(b"hello plain").unwrap();
        let finished = writer.finish().unwrap();
        finished.commit().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "hello plain");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn compressed_writer_gzip_round_trips() {
        let path = unique_test_path("compressed.gz");
        let target = WriteTarget::File(AtomicFileWriter::create(path.clone()).unwrap());
        let mut writer = CompressedWriter::new(target, CompressionKind::Gzip).unwrap();
        writer.write_all(b"hello gzip").unwrap();
        let finished = writer.finish().unwrap();
        finished.commit().unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "hello gzip");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn compressed_writer_zstd_round_trips() {
        let path = unique_test_path("compressed.zst");
        let target = WriteTarget::File(AtomicFileWriter::create(path.clone()).unwrap());
        let mut writer = CompressedWriter::new(target, CompressionKind::Zstd).unwrap();
        writer.write_all(b"hello zstd").unwrap();
        let finished = writer.finish().unwrap();
        finished.commit().unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let mut decoder = zstd::stream::read::Decoder::new(file).unwrap();
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "hello zstd");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn source_dir_validation_allows_home_dir_and_rejects_system_dir() {
        let home = dirs::home_dir().unwrap();
        let allowed = home.join("mongo-buddy-restore-test-dir");
        fs::create_dir_all(&allowed).unwrap();
        assert_eq!(
            validate_source_dir(&allowed.to_string_lossy()).unwrap(),
            allowed
        );
        let _ = fs::remove_dir(&allowed);

        let denied = validate_source_dir("/etc");
        assert!(matches!(denied, Err(AppError::Validation(_))));
    }

    #[test]
    fn source_dir_validation_rejects_file() {
        let home = dirs::home_dir().unwrap();
        let file = home.join("mongo-buddy-restore-test-file.txt");
        fs::write(&file, b"x").unwrap();
        let result = validate_source_dir(&file.to_string_lossy());
        assert!(matches!(result, Err(AppError::Validation(_))));
        let _ = fs::remove_file(&file);
    }
}
