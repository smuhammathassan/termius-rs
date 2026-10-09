//! sftp — see crate docs.
//!
//! A small, functional wrapper over `russh-sftp`'s client session covering the
//! operations the Termius SFTP browser performs: list / read / write / remove /
//! mkdir / rename (plus `stat`). Result types are plain Rust structs so
//! `termius-ssh` stays independent of `russh-sftp` protocol types.

use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

use crate::error::{Result, SshError};

/// Map any `russh-sftp` error (its exact type varies across 2.x/3.x) into
/// [`SshError::Sftp`] without pinning a concrete error type.
fn sftp_err<E: std::fmt::Display>(err: E) -> SshError {
    SshError::sftp(err.to_string())
}

/// Join a remote directory and entry name using POSIX semantics
/// (`"/"` handling is pure logic, unit-tested here).
pub fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// One directory entry as returned by [`SftpClient::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpEntry {
    pub name: String,
    /// `None` when the server did not return permissions for the entry.
    pub is_dir: Option<bool>,
    pub size: Option<u64>,
}

/// Metadata for a remote path.
///
/// (russh-sftp 3.0.1: `metadata()` returns `FileAttributes`, whose `size` /
/// `permissions` fields are `Option<u64>` / `Option<u32>`.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpStat {
    pub size: Option<u64>,
    pub is_dir: Option<bool>,
    pub permissions: Option<u32>,
}

impl SftpStat {
    const DIRECTORY_MODE: u32 = 0o040000;
    const MODE_MASK: u32 = 0o170000;

    pub fn is_dir(&self) -> bool {
        self.is_dir.unwrap_or(false)
    }

    pub fn is_file(&self) -> bool {
        self.is_dir == Some(false)
    }
}

/// SFTP client bound to one session channel of an established SSH connection.
///
/// Backed by russh-sftp 3.0.1's `russh_sftp::client::SftpSession`; operation
/// names inside the impl (`open` / `create` / `remove_file` / `create_dir` /
/// `remove_dir` / `rename` / `metadata` / `read_dir`) are the verified 3.0.1
/// surface. All errors are mapped through [`sftp_err`] so the concrete error
/// type is never named.
pub struct SftpClient {
    sftp: SftpSession,
}

impl SftpClient {
    /// Wrap an already-opened SFTP session. The caller (see
    /// [`crate::client::SshClient::sftp`]) opens a session channel and
    /// requests the `sftp` subsystem first.
    pub fn new(sftp: SftpSession) -> Self {
        Self { sftp }
    }

    /// List a remote directory (skips `.` / `..`), resolving each entry's
    /// type/size with a follow-up `stat` (best effort — special files may
    /// not carry attributes).
    #[instrument(skip_all, fields(path = %path))]
    pub async fn list(&self, path: &str) -> Result<Vec<SftpEntry>> {
        // russh-sftp 3.0.1: `read_dir` resolves to a `ReadDir`, a plain
        // (synchronous) `Iterator` over `DirEntry`s that already skips
        // `.` / `..`.
        let reader = self.sftp.read_dir(path).await.map_err(sftp_err)?;
        let mut entries = Vec::new();
        for entry in reader {
            let name = entry.file_name();
            let full = join_path(path, &name);
            let stat = self.stat(&full).await.ok();
            entries.push(SftpEntry {
                name,
                is_dir: stat.as_ref().and_then(|s| s.is_dir),
                size: stat.as_ref().and_then(|s| s.size),
            });
        }
        Ok(entries)
    }

    /// Stat a remote path.
    pub async fn stat(&self, path: &str) -> Result<SftpStat> {
        // russh-sftp 3.0.1: `metadata` returns `FileAttributes`, whose
        // `size` / `permissions` fields are already `Option`s.
        let attrs = self.sftp.metadata(path).await.map_err(sftp_err)?;
        let size = attrs.size;
        let permissions = attrs.permissions;
        let is_dir = permissions.map(|mode| mode & SftpStat::MODE_MASK == SftpStat::DIRECTORY_MODE);
        Ok(SftpStat { size, is_dir, permissions })
    }

    /// Read a whole remote file into memory.
    #[instrument(skip_all, fields(path = %path))]
    pub async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let mut file = self.sftp.open(path).await.map_err(sftp_err)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await.map_err(sftp_err)?;
        Ok(buffer)
    }

    /// Read a remote file as (lossily decoded) text.
    pub async fn read_to_string(&self, path: &str) -> Result<String> {
        let bytes = self.read(path).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Create/truncate a remote file and write `data` to it.
    #[instrument(skip_all, fields(path = %path, len = data.len()))]
    pub async fn write(&self, path: &str, data: &[u8]) -> Result<()> {
        let mut file = self.sftp.create(path).await.map_err(sftp_err)?;
        file.write_all(data).await.map_err(sftp_err)?;
        file.flush().await.map_err(sftp_err)?;
        Ok(())
    }

    /// Remove a remote file.
    pub async fn remove(&self, path: &str) -> Result<()> {
        // russh-sftp 3.0.1 names the operation `remove_file` (not `remove`).
        self.sftp.remove_file(path).await.map_err(sftp_err)
    }

    /// Create a remote directory.
    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.sftp.create_dir(path).await.map_err(sftp_err)
    }

    /// Remove an empty remote directory.
    pub async fn rmdir(&self, path: &str) -> Result<()> {
        self.sftp.remove_dir(path).await.map_err(sftp_err)
    }

    /// Rename/move a remote path.
    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.sftp.rename(from, to).await.map_err(sftp_err)
    }

    /// Escape hatch for operations outside this small surface.
    pub fn session(&self) -> &SftpSession {
        &self.sftp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_path_handles_slashes_and_empty_roots() {
        assert_eq!(join_path("/home/dev", "notes.txt"), "/home/dev/notes.txt");
        assert_eq!(join_path("/home/dev/", "notes.txt"), "/home/dev/notes.txt");
        assert_eq!(join_path("/", "etc"), "/etc");
        assert_eq!(join_path("", "relative"), "relative");
        assert_eq!(join_path("dir", "sub/leaf"), "dir/sub/leaf");
    }

    #[test]
    fn stat_directory_detection_from_mode() {
        let dir = SftpStat { size: None, is_dir: Some(true), permissions: Some(0o040755) };
        assert!(dir.is_dir());
        assert!(!dir.is_file());

        let file = SftpStat { size: Some(42), is_dir: Some(false), permissions: Some(0o100644) };
        assert!(file.is_file());
        assert!(!file.is_dir());

        let unknown = SftpStat { size: None, is_dir: None, permissions: None };
        assert!(!unknown.is_dir());
        assert!(!unknown.is_file());
    }
}
