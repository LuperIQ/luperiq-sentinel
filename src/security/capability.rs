use std::path::Path;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CapabilityResult {
    Allowed,
    Denied(String),
}

pub struct CapabilityChecker {
    allowed_read_paths: Vec<String>,
    allowed_write_paths: Vec<String>,
    allowed_commands: Vec<String>,
}

// ── Implementation ──────────────────────────────────────────────────────────

impl CapabilityChecker {
    pub fn new(
        allowed_read_paths: Vec<String>,
        allowed_write_paths: Vec<String>,
        allowed_commands: Vec<String>,
    ) -> Self {
        CapabilityChecker {
            allowed_read_paths,
            allowed_write_paths,
            allowed_commands,
        }
    }

    pub fn check_file_read(&self, path: &str) -> CapabilityResult {
        check_path(path, &self.allowed_read_paths, "read")
    }

    pub fn check_file_write(&self, path: &str) -> CapabilityResult {
        check_path(path, &self.allowed_write_paths, "write")
    }

    pub fn check_command(&self, command: &str) -> CapabilityResult {
        if self.allowed_commands.is_empty() {
            return CapabilityResult::Denied("no commands are allowed".into());
        }

        // Extract base command name (strip path prefix)
        let base = Path::new(command)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(command);

        if self.allowed_commands.iter().any(|c| c == base || c == command) {
            CapabilityResult::Allowed
        } else {
            CapabilityResult::Denied(format!(
                "command '{}' not in allowlist",
                command
            ))
        }
    }
}

fn check_path(path: &str, allowed: &[String], operation: &str) -> CapabilityResult {
    if allowed.is_empty() {
        return CapabilityResult::Denied(format!("no {} paths are allowed", operation));
    }

    // Canonicalize the path to prevent ../traversal
    let canonical = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => {
            // For files that don't exist yet (write), try canonicalizing the parent
            let p = Path::new(path);
            if let Some(parent) = p.parent() {
                match std::fs::canonicalize(parent) {
                    Ok(canon_parent) => {
                        if let Some(filename) = p.file_name() {
                            canon_parent.join(filename)
                        } else {
                            return CapabilityResult::Denied(format!(
                                "cannot resolve path '{}'",
                                path
                            ));
                        }
                    }
                    Err(_) => {
                        return CapabilityResult::Denied(format!(
                            "cannot resolve path '{}'",
                            path
                        ));
                    }
                }
            } else {
                return CapabilityResult::Denied(format!("cannot resolve path '{}'", path));
            }
        }
    };

    let canonical_str = canonical.to_string_lossy();

    for allowed_prefix in allowed {
        // Canonicalize the allowed prefix too
        let canon_prefix = match std::fs::canonicalize(allowed_prefix) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => allowed_prefix.clone(),
        };

        if canonical_str.starts_with(&canon_prefix) {
            return CapabilityResult::Allowed;
        }
    }

    CapabilityResult::Denied(format!(
        "{} access denied for path '{}'",
        operation, path
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_allowlist() {
        let checker = CapabilityChecker::new(vec![], vec![], vec!["ls".into(), "cat".into()]);
        assert!(matches!(checker.check_command("ls"), CapabilityResult::Allowed));
        assert!(matches!(checker.check_command("rm"), CapabilityResult::Denied(_)));
        assert!(matches!(checker.check_command("/bin/ls"), CapabilityResult::Allowed));
    }

    #[test]
    fn test_empty_allowlist_denies() {
        let checker = CapabilityChecker::new(vec![], vec![], vec![]);
        assert!(matches!(checker.check_command("ls"), CapabilityResult::Denied(_)));
        assert!(matches!(checker.check_file_read("/tmp/x"), CapabilityResult::Denied(_)));
    }

    #[test]
    fn test_path_check() {
        let checker = CapabilityChecker::new(vec!["/tmp".into()], vec![], vec![]);
        assert!(matches!(checker.check_file_read("/tmp/test"), CapabilityResult::Allowed));
        assert!(matches!(checker.check_file_read("/etc/passwd"), CapabilityResult::Denied(_)));
    }
}
