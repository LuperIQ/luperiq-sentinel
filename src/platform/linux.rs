use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use super::{
    CapType, CommandOutput, DirEntry, Platform, PlatformError, TcpStream as PlatformTcpStream,
};

// ── Linux platform ─────────────────────────────────────────────────────────

pub struct LinuxPlatform {
    allowed_read_paths: Vec<String>,
    allowed_write_paths: Vec<String>,
    allowed_commands: Vec<String>,
    audit_file: std::cell::RefCell<Option<fs::File>>,
}

impl LinuxPlatform {
    pub fn new(
        allowed_read_paths: Vec<String>,
        allowed_write_paths: Vec<String>,
        allowed_commands: Vec<String>,
        audit_log_path: Option<&str>,
    ) -> Self {
        let audit_file = audit_log_path.and_then(|path| {
            match fs::OpenOptions::new().create(true).append(true).open(path) {
                Ok(f) => Some(f),
                Err(e) => {
                    eprintln!(
                        "sentinel: warning: cannot open audit log '{}': {}",
                        path, e
                    );
                    None
                }
            }
        });
        LinuxPlatform {
            allowed_read_paths,
            allowed_write_paths,
            allowed_commands,
            audit_file: std::cell::RefCell::new(audit_file),
        }
    }
}

impl Platform for LinuxPlatform {
    fn read_file(&self, path: &str) -> Result<String, PlatformError> {
        fs::read_to_string(path)
            .map_err(|e| PlatformError::Io(format!("failed to read '{}': {}", path, e)))
    }

    fn write_file(&self, path: &str, content: &str) -> Result<(), PlatformError> {
        fs::write(path, content)
            .map_err(|e| PlatformError::Io(format!("failed to write '{}': {}", path, e)))
    }

    fn list_directory(&self, path: &str) -> Result<Vec<DirEntry>, PlatformError> {
        let entries = fs::read_dir(path)
            .map_err(|e| PlatformError::Io(format!("failed to list '{}': {}", path, e)))?;

        let mut result = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|e| PlatformError::Io(format!("error reading entry: {}", e)))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry
                .file_type()
                .map(|ft| ft.is_dir())
                .unwrap_or(false);
            result.push(DirEntry { name, is_dir });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    fn run_command(
        &self,
        command: &str,
        args: &[String],
        timeout_secs: u64,
    ) -> Result<CommandOutput, PlatformError> {
        let timeout = Duration::from_secs(timeout_secs);
        let mut child = Command::new(command)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| PlatformError::Io(format!("failed to run '{}': {}", command, e)))?;

        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout_buf = Vec::new();
                    let mut stderr_buf = Vec::new();
                    if let Some(ref mut out) = child.stdout {
                        let _ = out.read_to_end(&mut stdout_buf);
                    }
                    if let Some(ref mut err) = child.stderr {
                        let _ = err.read_to_end(&mut stderr_buf);
                    }

                    return Ok(CommandOutput {
                        stdout: String::from_utf8_lossy(&stdout_buf).to_string(),
                        stderr: String::from_utf8_lossy(&stderr_buf).to_string(),
                        exit_code: status.code().unwrap_or(-1),
                    });
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(PlatformError::Timeout(format!(
                            "command '{}' timed out after {}s",
                            command, timeout_secs
                        )));
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(PlatformError::Io(format!(
                        "error waiting for '{}': {}",
                        command, e
                    )));
                }
            }
        }
    }

    fn canonicalize(&self, path: &str) -> Result<String, PlatformError> {
        // Try canonicalizing the path
        match fs::canonicalize(path) {
            Ok(p) => Ok(p.to_string_lossy().to_string()),
            Err(_) => {
                // For files that don't exist yet, try canonicalizing the parent
                let p = Path::new(path);
                if let Some(parent) = p.parent() {
                    match fs::canonicalize(parent) {
                        Ok(canon_parent) => {
                            if let Some(filename) = p.file_name() {
                                Ok(canon_parent
                                    .join(filename)
                                    .to_string_lossy()
                                    .to_string())
                            } else {
                                Err(PlatformError::NotFound(format!(
                                    "cannot resolve path '{}'",
                                    path
                                )))
                            }
                        }
                        Err(_) => Err(PlatformError::NotFound(format!(
                            "cannot resolve path '{}'",
                            path
                        ))),
                    }
                } else {
                    Err(PlatformError::NotFound(format!(
                        "cannot resolve path '{}'",
                        path
                    )))
                }
            }
        }
    }

    fn check_capability(&self, cap_type: CapType, resource: &str) -> Result<bool, PlatformError> {
        match cap_type {
            CapType::FileRead => Ok(check_path_allowed(
                resource,
                &self.allowed_read_paths,
                self,
            )),
            CapType::FileWrite => Ok(check_path_allowed(
                resource,
                &self.allowed_write_paths,
                self,
            )),
            CapType::Command => {
                if self.allowed_commands.is_empty() {
                    return Ok(false);
                }
                let base = Path::new(resource)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(resource);
                Ok(self
                    .allowed_commands
                    .iter()
                    .any(|c| c == base || c == resource))
            }
            CapType::Network => Ok(true), // Linux: network always allowed (TLS handles auth)
        }
    }

    fn request_capability(
        &self,
        _cap_type: CapType,
        _resource: &str,
    ) -> Result<(), PlatformError> {
        // On Linux, capabilities are static (from config). No runtime request mechanism.
        Err(PlatformError::NotSupported(
            "capability requests not supported on Linux".into(),
        ))
    }

    fn audit_event(&self, event_json: &str) -> Result<(), PlatformError> {
        eprintln!("audit: {}", event_json);
        if let Some(ref mut f) = *self.audit_file.borrow_mut() {
            let _ = writeln!(f, "{}", event_json);
        }
        Ok(())
    }

    fn tcp_connect(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn PlatformTcpStream>, PlatformError> {
        let addr = format!("{}:{}", host, port);
        let tcp = std::net::TcpStream::connect(&addr)
            .map_err(|e| PlatformError::Io(format!("connect to {}: {}", addr, e)))?;
        tcp.set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| PlatformError::Io(e.to_string()))?;
        tcp.set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| PlatformError::Io(e.to_string()))?;
        Ok(Box::new(LinuxTcpStream(tcp)))
    }
}

fn check_path_allowed(path: &str, allowed: &[String], platform: &LinuxPlatform) -> bool {
    if allowed.is_empty() {
        return false;
    }
    let canonical = match platform.canonicalize(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    for prefix in allowed {
        let canon_prefix = match fs::canonicalize(prefix) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => prefix.clone(),
        };
        if canonical.starts_with(&canon_prefix) {
            return true;
        }
    }
    false
}

// ── Linux TCP stream wrapper ───────────────────────────────────────────────

struct LinuxTcpStream(std::net::TcpStream);

impl std::io::Read for LinuxTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Write for LinuxTcpStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl PlatformTcpStream for LinuxTcpStream {
    fn set_read_timeout(&self, dur: Duration) -> Result<(), PlatformError> {
        self.0
            .set_read_timeout(Some(dur))
            .map_err(|e| PlatformError::Io(e.to_string()))
    }

    fn set_write_timeout(&self, dur: Duration) -> Result<(), PlatformError> {
        self.0
            .set_write_timeout(Some(dur))
            .map_err(|e| PlatformError::Io(e.to_string()))
    }
}
