use std::process::{Child, Command, Stdio};

// ── Sandboxed process ───────────────────────────────────────────────────────

pub struct SandboxedProcess {
    child: Child,
}

impl SandboxedProcess {
    /// Spawn a skill binary in a sandboxed subprocess.
    ///
    /// The child process:
    /// - Inherits the parent's seccomp + landlock filters (automatically)
    /// - Has stdin/stdout piped for IPC
    /// - Has stderr inherited for logging
    /// - Runs in the skill's directory
    /// - Has a minimal environment
    pub fn spawn(binary_path: &str, working_dir: &str) -> Result<Self, String> {
        let child = Command::new(binary_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .current_dir(working_dir)
            .env_clear()
            .env("PATH", "/usr/bin:/usr/local/bin:/bin")
            .env("HOME", working_dir)
            .env("LANG", "C.UTF-8")
            .spawn()
            .map_err(|e| format!("failed to spawn skill '{}': {}", binary_path, e))?;

        Ok(SandboxedProcess { child })
    }

    /// Get a mutable reference to the child's stdin.
    pub fn stdin(&mut self) -> Option<&mut std::process::ChildStdin> {
        self.child.stdin.as_mut()
    }

    /// Get a mutable reference to the child's stdout.
    pub fn stdout(&mut self) -> Option<&mut std::process::ChildStdout> {
        self.child.stdout.as_mut()
    }

    /// Try to wait for the child without blocking.
    pub fn try_wait(&mut self) -> Result<Option<i32>, String> {
        self.child
            .try_wait()
            .map(|status| status.map(|s| s.code().unwrap_or(-1)))
            .map_err(|e| format!("wait error: {}", e))
    }

    /// Kill the child process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for SandboxedProcess {
    fn drop(&mut self) {
        // Ensure child is cleaned up
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
