use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{self, Read, Seek, SeekFrom, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use mcp_server::McpProfile;
use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
const WORKSPACE_RECORD_NAME: &str = "bsl-analyzer-mcp-http.pid.json";
const REFERENCE_RECORD_NAME: &str = "bsl-analyzer-mcp-http-reference.pid.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ProcessState {
    Starting,
    Running,
    Stopping,
    Stopped,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProcessRecord {
    schema_version: u32,
    instance_id: String,
    pid: u32,
    host: String,
    port: u16,
    profile: String,
    source_dir: Option<PathBuf>,
    executable: PathBuf,
    started_at: String,
    state: ProcessState,
}

#[derive(Debug)]
pub(super) struct ProcessRecordGuard {
    file: File,
    path: PathBuf,
    record: ProcessRecord,
}

impl ProcessRecordGuard {
    pub(super) fn acquire(
        profile: McpProfile,
        source_dir: Option<PathBuf>,
        requested_address: SocketAddr,
    ) -> io::Result<Self> {
        let path = record_path(profile, source_dir.as_deref())?;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "process record has no parent directory")
        })?;
        std::fs::create_dir_all(parent)?;

        let mut file = OpenOptions::new().read(true).write(true).create(true).open(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let existing = read_record(&mut file).ok();
                return Err(already_running_error(&path, existing.as_ref()));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }

        let record = ProcessRecord {
            schema_version: SCHEMA_VERSION,
            instance_id: uuid::Uuid::new_v4().to_string(),
            pid: std::process::id(),
            host: requested_address.ip().to_string(),
            port: requested_address.port(),
            profile: profile.as_str().to_owned(),
            source_dir,
            executable: std::env::current_exe().or_else(|_| {
                std::env::args_os().next().map(PathBuf::from).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "current executable is unknown")
                })
            })?,
            started_at: chrono::Utc::now().to_rfc3339(),
            state: ProcessState::Starting,
        };
        let mut guard = Self { file, path, record };
        if let Err(error) = guard.write_unchecked() {
            let _ = guard.file.unlock();
            return Err(error);
        }
        Ok(guard)
    }

    pub(super) fn mark_running(&mut self, address: SocketAddr) -> io::Result<()> {
        self.write_bound_process(address)?;
        self.record.state = ProcessState::Running;
        self.write_owned()
    }

    pub(super) fn write_bound_process(&mut self, address: SocketAddr) -> io::Result<()> {
        self.record.host = address.ip().to_string();
        self.record.port = address.port();
        self.write_owned()
    }

    pub(super) fn mark_stopping(&mut self) -> io::Result<()> {
        self.record.state = ProcessState::Stopping;
        self.write_owned()
    }

    pub(super) fn mark_stopped(&mut self) -> io::Result<()> {
        self.record.state = ProcessState::Stopped;
        self.write_owned()
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }

    fn write_owned(&mut self) -> io::Result<()> {
        let instance_id = self.record.instance_id.clone();
        let existing = read_record(&mut self.file)?;
        if existing.instance_id != instance_id {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("process record ownership changed: {}", self.path.display()),
            ));
        }
        self.write_unchecked()
    }

    fn write_unchecked(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        serde_json::to_writer_pretty(&mut self.file, &self.record)?;
        self.file.write_all(b"\n")?;
        self.file.flush()
    }
}

impl Drop for ProcessRecordGuard {
    fn drop(&mut self) {
        if self.record.state != ProcessState::Stopped {
            let _ = self.mark_stopped();
        }
        let _ = self.file.unlock();
    }
}

fn record_path(profile: McpProfile, source_dir: Option<&Path>) -> io::Result<PathBuf> {
    match profile {
        McpProfile::Workspace => {
            let source_dir = source_dir.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "workspace profile requires a source directory for its process record",
                )
            })?;
            Ok(source_dir.join(".build").join(WORKSPACE_RECORD_NAME))
        }
        McpProfile::Reference => {
            let base = dirs::state_dir().or_else(dirs::data_local_dir).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "application state directory is unavailable",
                )
            })?;
            Ok(base.join("bsl-analyzer").join(REFERENCE_RECORD_NAME))
        }
    }
}

fn read_record(file: &mut File) -> io::Result<ProcessRecord> {
    file.seek(SeekFrom::Start(0))?;
    let mut json = Vec::new();
    file.read_to_end(&mut json)?;
    serde_json::from_slice(&json).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn already_running_error(path: &Path, existing: Option<&ProcessRecord>) -> io::Error {
    let message = match existing {
        Some(record) => format!(
            "MCP HTTP already serves this profile or project (pid {}, address {}:{}, record {})",
            record.pid,
            record.host,
            record.port,
            path.display()
        ),
        None => {
            format!("MCP HTTP already serves this profile or project (record {})", path.display())
        }
    };
    io::Error::new(io::ErrorKind::AlreadyExists, message)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use mcp_server::McpProfile;

    use super::{ProcessRecord, ProcessRecordGuard, ProcessState};

    fn address(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    #[test]
    fn exclusive_record_lock_rejects_a_second_server_and_is_released_on_drop() {
        let project = tempfile::tempdir().expect("temporary project should be created");
        let source_dir = project.path().canonicalize().expect("project path should canonicalize");
        let first = ProcessRecordGuard::acquire(
            McpProfile::Workspace,
            Some(source_dir.clone()),
            address(8021),
        )
        .expect("first server should acquire the project record");

        let error = ProcessRecordGuard::acquire(
            McpProfile::Workspace,
            Some(source_dir.clone()),
            address(9021),
        )
        .expect_err("second server for the same project must be rejected");
        let message = error.to_string();
        assert!(message.contains(&std::process::id().to_string()));
        assert!(message.contains("8021"));

        drop(first);
        ProcessRecordGuard::acquire(McpProfile::Workspace, Some(source_dir), address(9021))
            .expect("dropping the owner should release the system lock");
    }

    #[test]
    fn process_record_tracks_actual_address_and_stopped_state() {
        let project = tempfile::tempdir().expect("temporary project should be created");
        let source_dir = project.path().canonicalize().expect("project path should canonicalize");
        let mut guard = ProcessRecordGuard::acquire(
            McpProfile::Workspace,
            Some(source_dir.clone()),
            address(8021),
        )
        .expect("record should be acquired");

        guard.mark_running(address(32123)).expect("running record should be written");
        let path = guard.path().to_owned();
        let running: ProcessRecord = serde_json::from_slice(
            &std::fs::read(&path).expect("running record should be readable"),
        )
        .expect("running record should be valid JSON");
        assert_eq!(running.pid, std::process::id());
        assert_eq!(running.host, "127.0.0.1");
        assert_eq!(running.port, 32123);
        assert_eq!(running.profile, "workspace");
        assert_eq!(running.source_dir.as_deref(), Some(source_dir.as_path()));
        assert_eq!(running.state, ProcessState::Running);

        guard.mark_stopping().expect("stopping state should be written");
        drop(guard);

        let stopped: ProcessRecord = serde_json::from_slice(
            &std::fs::read(path).expect("stopped record should remain on disk"),
        )
        .expect("stopped record should be valid JSON");
        assert_eq!(stopped.instance_id, running.instance_id);
        assert_eq!(stopped.state, ProcessState::Stopped);
    }
}
