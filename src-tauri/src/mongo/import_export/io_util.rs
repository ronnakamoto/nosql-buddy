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

/// Validate an import source path. The file must already exist and live under
/// the same user roots as export targets.
pub fn validate_source_path(path: &str) -> AppResult<PathBuf> {
    validate_user_path(path, "import", true)
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

fn part_path_for(final_path: &Path) -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
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
}
