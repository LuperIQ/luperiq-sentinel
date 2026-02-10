pub mod linux;
#[cfg(target_os = "none")]
pub mod luperiq;

// ── Platform trait ─────────────────────────────────────────────────────────

/// Abstraction over OS-specific operations.
///
/// On Linux: uses std::fs, std::process, std::net + rustls.
/// On LuperIQ OS: uses kernel syscalls via luperiq-rt.
pub trait Platform {
    // ── File operations ────────────────────────────────────────────────

    fn read_file(&self, path: &str) -> Result<String, PlatformError>;
    fn write_file(&self, path: &str, content: &str) -> Result<(), PlatformError>;
    fn list_directory(&self, path: &str) -> Result<Vec<DirEntry>, PlatformError>;

    // ── Process operations ─────────────────────────────────────────────

    fn run_command(
        &self,
        command: &str,
        args: &[String],
        timeout_secs: u64,
    ) -> Result<CommandOutput, PlatformError>;

    // ── Path operations ────────────────────────────────────────────────

    fn canonicalize(&self, path: &str) -> Result<String, PlatformError>;

    // ── Capability operations ──────────────────────────────────────────

    fn check_capability(&self, cap_type: CapType, resource: &str) -> Result<bool, PlatformError>;
    fn request_capability(
        &self,
        cap_type: CapType,
        resource: &str,
    ) -> Result<(), PlatformError>;

    // ── Audit operations ───────────────────────────────────────────────

    fn audit_event(&self, event_json: &str) -> Result<(), PlatformError>;

    // ── Network operations ─────────────────────────────────────────────

    fn tcp_connect(&self, host: &str, port: u16) -> Result<Box<dyn TcpStream>, PlatformError>;
}

// ── Supporting types ───────────────────────────────────────────────────────

pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Copy)]
pub enum CapType {
    FileRead,
    FileWrite,
    Command,
    Network,
}

#[derive(Debug)]
pub enum PlatformError {
    Io(String),
    PermissionDenied(String),
    NotFound(String),
    Timeout(String),
    NotSupported(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::Io(s) => write!(f, "I/O error: {}", s),
            PlatformError::PermissionDenied(s) => write!(f, "permission denied: {}", s),
            PlatformError::NotFound(s) => write!(f, "not found: {}", s),
            PlatformError::Timeout(s) => write!(f, "timeout: {}", s),
            PlatformError::NotSupported(s) => write!(f, "not supported: {}", s),
        }
    }
}

/// Trait for platform-specific TCP streams (with TLS where applicable).
pub trait TcpStream: std::io::Read + std::io::Write {
    fn set_read_timeout(&self, dur: std::time::Duration) -> Result<(), PlatformError>;
    fn set_write_timeout(&self, dur: std::time::Duration) -> Result<(), PlatformError>;
}
