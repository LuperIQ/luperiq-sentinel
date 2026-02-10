// LuperIQ OS platform implementation
//
// This module is only compiled when targeting LuperIQ OS (target_os = "none").
// It uses kernel syscalls via luperiq-rt instead of std.

// NOTE: This file will only compile in no_std context with luperiq-rt.
// On Linux builds (the default), it is excluded via #[cfg(target_os = "none")].

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use luperiq_rt::syscall;

use super::{
    CapType, CommandOutput, DirEntry, Platform, PlatformError, TcpStream as PlatformTcpStream,
};

// ── Syscall numbers ────────────────────────────────────────────────────────

const SYS_OPEN: u64 = 62;
const SYS_READ: u64 = 60;
const SYS_WRITE: u64 = 61;
const SYS_CLOSE: u64 = 63;
const SYS_STAT: u64 = 64;
const SYS_SPAWN: u64 = 2;
const SYS_WAIT: u64 = 3;
const SYS_SOCKET_CREATE: u64 = 120;
const SYS_SOCKET_CONNECT: u64 = 124;
const SYS_SOCKET_SEND: u64 = 125;
const SYS_SOCKET_RECV: u64 = 126;
const SYS_CAP_REQUEST: u64 = 130;
const SYS_AUDIT_WRITE: u64 = 134;

const O_READ: u32 = 1;
const O_WRITE: u32 = 2;
const O_CREATE: u32 = 4;
const O_TRUNCATE: u32 = 8;

const SOCKET_TCP: u8 = 1;

// ── LuperIQ platform ──────────────────────────────────────────────────────

pub struct LuperiqPlatform;

impl LuperiqPlatform {
    pub fn new() -> Self {
        LuperiqPlatform
    }
}

impl Platform for LuperiqPlatform {
    fn read_file(&self, path: &str) -> Result<String, PlatformError> {
        let fd = syscall::open(path, O_READ)
            .map_err(|e| PlatformError::Io(format!("open '{}': error {}", path, e)))?;

        let mut contents = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = syscall::read(fd, &mut buf)
                .map_err(|e| PlatformError::Io(format!("read '{}': error {}", path, e)))?;
            if n == 0 {
                break;
            }
            contents.extend_from_slice(&buf[..n]);
        }
        let _ = syscall::close(fd);

        String::from_utf8(contents)
            .map_err(|_| PlatformError::Io(format!("'{}' is not valid UTF-8", path)))
    }

    fn write_file(&self, path: &str, content: &str) -> Result<(), PlatformError> {
        let fd = syscall::open(path, O_WRITE | O_CREATE | O_TRUNCATE)
            .map_err(|e| PlatformError::Io(format!("open '{}': error {}", path, e)))?;

        let data = content.as_bytes();
        let mut offset = 0;
        while offset < data.len() {
            let n = syscall::write(fd, &data[offset..])
                .map_err(|e| PlatformError::Io(format!("write '{}': error {}", path, e)))?;
            offset += n;
        }
        let _ = syscall::close(fd);
        Ok(())
    }

    fn list_directory(&self, path: &str) -> Result<Vec<DirEntry>, PlatformError> {
        // The kernel VFS supports reading directory entries via stat + open+read on dir
        // For now, use a simple approach: open the directory and read entries
        // The kernel's ramfs returns entries as newline-separated "name\ttype\n" when
        // reading a directory fd.
        let fd = syscall::open(path, O_READ)
            .map_err(|e| PlatformError::Io(format!("open dir '{}': error {}", path, e)))?;

        let mut buf = [0u8; 8192];
        let n = syscall::read(fd, &mut buf)
            .map_err(|e| PlatformError::Io(format!("read dir '{}': error {}", path, e)))?;
        let _ = syscall::close(fd);

        let text = core::str::from_utf8(&buf[..n])
            .map_err(|_| PlatformError::Io("directory listing not UTF-8".into()))?;

        let mut entries = Vec::new();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            // Format: "name\ttype" where type is "dir" or "file"
            let (name, is_dir) = if let Some(tab) = line.find('\t') {
                let name = &line[..tab];
                let kind = &line[tab + 1..];
                (name.into(), kind == "dir")
            } else {
                (line.into(), false)
            };
            entries.push(DirEntry { name, is_dir });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn run_command(
        &self,
        command: &str,
        _args: &[String],
        _timeout_secs: u64,
    ) -> Result<CommandOutput, PlatformError> {
        // On LuperIQ OS, spawn a child process from the binary path
        let handle = syscall::spawn(command)
            .map_err(|e| PlatformError::Io(format!("spawn '{}': error {}", command, e)))?;

        let exit_code = syscall::waitpid(handle)
            .map_err(|e| PlatformError::Io(format!("wait '{}': error {}", command, e)))?;

        // No stdout/stderr capture yet — child writes to its own fd 2/3
        Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: exit_code as i32,
        })
    }

    fn canonicalize(&self, path: &str) -> Result<String, PlatformError> {
        // No symlinks on LuperIQ OS. Just resolve . and ..
        if !path.starts_with('/') {
            return Err(PlatformError::NotFound(format!(
                "path must be absolute: '{}'",
                path
            )));
        }
        let mut parts: Vec<&str> = Vec::new();
        for component in path.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(PlatformError::NotFound(
                            "path traverses above root".into(),
                        ));
                    }
                }
                name => parts.push(name),
            }
        }
        if parts.is_empty() {
            Ok("/".into())
        } else {
            let mut result = String::new();
            for part in &parts {
                result.push('/');
                result.push_str(part);
            }
            Ok(result)
        }
    }

    fn check_capability(&self, cap_type: CapType, resource: &str) -> Result<bool, PlatformError> {
        // On LuperIQ OS, the kernel enforces capabilities.
        // The kernel will deny the actual operation (open, connect, spawn) if not allowed.
        // We return true here and let the kernel enforce.
        let _ = (cap_type, resource);
        Ok(true)
    }

    fn request_capability(
        &self,
        cap_type: CapType,
        resource: &str,
    ) -> Result<(), PlatformError> {
        let spec = match cap_type {
            CapType::FileRead => format!("FILE_READ:{}", resource),
            CapType::FileWrite => format!("FILE_WRITE:{}", resource),
            CapType::Command => format!("SPAWN:{}", resource),
            CapType::Network => format!("NETWORK:{}", resource),
        };

        // Syscall 130: CapabilityRequest(spec_ptr, spec_len)
        let ret = syscall::syscall2(
            SYS_CAP_REQUEST,
            spec.as_ptr() as u64,
            spec.len() as u64,
        );
        if ret < 0 {
            Err(PlatformError::PermissionDenied(format!(
                "capability request denied: error {}",
                ret
            )))
        } else {
            Ok(())
        }
    }

    fn audit_event(&self, event_json: &str) -> Result<(), PlatformError> {
        // Syscall 134: AuditWrite(msg_ptr, msg_len)
        let ret = syscall::syscall2(
            SYS_AUDIT_WRITE,
            event_json.as_ptr() as u64,
            event_json.len() as u64,
        );
        if ret < 0 {
            Err(PlatformError::Io(format!("audit_write: error {}", ret)))
        } else {
            Ok(())
        }
    }

    fn tcp_connect(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn PlatformTcpStream>, PlatformError> {
        // Create TCP socket via kernel
        let handle = {
            let ret = syscall::syscall1(SYS_SOCKET_CREATE, SOCKET_TCP as u64);
            if ret < 0 {
                return Err(PlatformError::Io(format!(
                    "socket_create: error {}",
                    ret
                )));
            }
            ret as u32
        };

        // Resolve host to IP — for now, use kernel DNS or assume IP literal
        // Pack IP as u64: (a << 24) | (b << 16) | (c << 8) | d
        let ip_packed = parse_ip_or_resolve(host)?;

        // Connect
        let ret = syscall::syscall3(
            SYS_SOCKET_CONNECT,
            handle as u64,
            ip_packed,
            port as u64,
        );
        if ret < 0 {
            return Err(PlatformError::Io(format!(
                "socket_connect to {}:{}: error {}",
                host, port, ret
            )));
        }

        Ok(Box::new(LuperiqTcpStream { handle }))
    }
}

fn parse_ip_or_resolve(host: &str) -> Result<u64, PlatformError> {
    // Try parsing as dotted-quad IP
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4 {
        if let (Ok(a), Ok(b), Ok(c), Ok(d)) = (
            parts[0].parse::<u8>(),
            parts[1].parse::<u8>(),
            parts[2].parse::<u8>(),
            parts[3].parse::<u8>(),
        ) {
            return Ok(((a as u64) << 24) | ((b as u64) << 16) | ((c as u64) << 8) | (d as u64));
        }
    }

    // TODO: kernel DNS resolution syscall
    Err(PlatformError::NotSupported(format!(
        "DNS resolution not yet implemented for '{}'",
        host
    )))
}

// ── LuperIQ TCP stream ────────────────────────────────────────────────────

struct LuperiqTcpStream {
    handle: u32,
}

impl core::io::Read for LuperiqTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> core::io::Result<usize> {
        let ret = syscall::syscall3(
            SYS_SOCKET_RECV,
            self.handle as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        );
        if ret < 0 {
            Err(core::io::Error::new(
                core::io::ErrorKind::Other,
                "socket recv error",
            ))
        } else {
            Ok(ret as usize)
        }
    }
}

impl core::io::Write for LuperiqTcpStream {
    fn write(&mut self, buf: &[u8]) -> core::io::Result<usize> {
        let ret = syscall::syscall3(
            SYS_SOCKET_SEND,
            self.handle as u64,
            buf.as_ptr() as u64,
            buf.len() as u64,
        );
        if ret < 0 {
            Err(core::io::Error::new(
                core::io::ErrorKind::Other,
                "socket send error",
            ))
        } else {
            Ok(ret as usize)
        }
    }

    fn flush(&mut self) -> core::io::Result<()> {
        Ok(())
    }
}

impl Drop for LuperiqTcpStream {
    fn drop(&mut self) {
        let _ = syscall::close(self.handle);
    }
}

impl PlatformTcpStream for LuperiqTcpStream {
    fn set_read_timeout(&self, _dur: core::time::Duration) -> Result<(), PlatformError> {
        // Kernel sockets don't have configurable timeouts yet
        Ok(())
    }

    fn set_write_timeout(&self, _dur: core::time::Duration) -> Result<(), PlatformError> {
        Ok(())
    }
}
