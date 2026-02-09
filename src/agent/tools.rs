use std::fs;
use std::io::Read;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::llm::provider::{ContentBlock, ToolDef};
use crate::net::json::{json_obj, json_arr, JsonValue};
use crate::security::audit::{AuditEvent, Auditor};
use crate::security::capability::{CapabilityChecker, CapabilityResult};

// ── Tool executor ───────────────────────────────────────────────────────────

pub struct ToolExecutor<'a> {
    caps: &'a CapabilityChecker,
    command_timeout: Duration,
}

impl<'a> ToolExecutor<'a> {
    pub fn new(caps: &'a CapabilityChecker, command_timeout_secs: u64) -> Self {
        ToolExecutor {
            caps,
            command_timeout: Duration::from_secs(command_timeout_secs),
        }
    }

    pub fn tool_definitions() -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "read_file".into(),
                description: "Read the contents of a file at the given path.".into(),
                input_schema: json_obj()
                    .field_str("type", "object")
                    .field(
                        "properties",
                        json_obj()
                            .field(
                                "path",
                                json_obj()
                                    .field_str("type", "string")
                                    .field_str("description", "Absolute path to the file to read")
                                    .build(),
                            )
                            .build(),
                    )
                    .field("required", json_arr().push_str("path").build())
                    .build(),
            },
            ToolDef {
                name: "write_file".into(),
                description: "Write content to a file at the given path.".into(),
                input_schema: json_obj()
                    .field_str("type", "object")
                    .field(
                        "properties",
                        json_obj()
                            .field(
                                "path",
                                json_obj()
                                    .field_str("type", "string")
                                    .field_str("description", "Absolute path to the file to write")
                                    .build(),
                            )
                            .field(
                                "content",
                                json_obj()
                                    .field_str("type", "string")
                                    .field_str("description", "Content to write to the file")
                                    .build(),
                            )
                            .build(),
                    )
                    .field(
                        "required",
                        json_arr().push_str("path").push_str("content").build(),
                    )
                    .build(),
            },
            ToolDef {
                name: "list_directory".into(),
                description: "List the contents of a directory.".into(),
                input_schema: json_obj()
                    .field_str("type", "object")
                    .field(
                        "properties",
                        json_obj()
                            .field(
                                "path",
                                json_obj()
                                    .field_str("type", "string")
                                    .field_str("description", "Absolute path to the directory")
                                    .build(),
                            )
                            .build(),
                    )
                    .field("required", json_arr().push_str("path").build())
                    .build(),
            },
            ToolDef {
                name: "run_command".into(),
                description: "Run a shell command and return its output.".into(),
                input_schema: json_obj()
                    .field_str("type", "object")
                    .field(
                        "properties",
                        json_obj()
                            .field(
                                "command",
                                json_obj()
                                    .field_str("type", "string")
                                    .field_str("description", "The command to run")
                                    .build(),
                            )
                            .field(
                                "args",
                                json_obj()
                                    .field_str("type", "array")
                                    .field(
                                        "items",
                                        json_obj().field_str("type", "string").build(),
                                    )
                                    .field_str("description", "Arguments to the command")
                                    .build(),
                            )
                            .build(),
                    )
                    .field("required", json_arr().push_str("command").build())
                    .build(),
            },
        ]
    }

    pub fn execute(
        &self,
        tool_use_id: &str,
        name: &str,
        input: &JsonValue,
        auditor: &mut Auditor,
    ) -> ContentBlock {
        let params_str = input.to_json_string();

        let result = match name {
            "read_file" => self.exec_read_file(input, auditor, &params_str),
            "write_file" => self.exec_write_file(input, auditor, &params_str),
            "list_directory" => self.exec_list_directory(input, auditor, &params_str),
            "run_command" => self.exec_run_command(input, auditor, &params_str),
            _ => Err(format!("unknown tool: {}", name)),
        };

        match result {
            Ok(output) => ContentBlock::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: output,
                is_error: false,
            },
            Err(err) => ContentBlock::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: err,
                is_error: true,
            },
        }
    }

    fn exec_read_file(
        &self,
        input: &JsonValue,
        auditor: &mut Auditor,
        params_str: &str,
    ) -> Result<String, String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("missing 'path' parameter")?;

        match self.caps.check_file_read(path) {
            CapabilityResult::Allowed => {
                auditor.log(AuditEvent::ToolCallAllowed {
                    tool: "read_file",
                    params: params_str,
                });
            }
            CapabilityResult::Denied(reason) => {
                auditor.log(AuditEvent::ToolCallDenied {
                    tool: "read_file",
                    params: params_str,
                    reason: &reason,
                });
                return Err(format!("access denied: {}", reason));
            }
        }

        fs::read_to_string(path).map_err(|e| format!("failed to read '{}': {}", path, e))
    }

    fn exec_write_file(
        &self,
        input: &JsonValue,
        auditor: &mut Auditor,
        params_str: &str,
    ) -> Result<String, String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("missing 'path' parameter")?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("missing 'content' parameter")?;

        match self.caps.check_file_write(path) {
            CapabilityResult::Allowed => {
                auditor.log(AuditEvent::ToolCallAllowed {
                    tool: "write_file",
                    params: params_str,
                });
            }
            CapabilityResult::Denied(reason) => {
                auditor.log(AuditEvent::ToolCallDenied {
                    tool: "write_file",
                    params: params_str,
                    reason: &reason,
                });
                return Err(format!("access denied: {}", reason));
            }
        }

        fs::write(path, content)
            .map(|_| format!("wrote {} bytes to '{}'", content.len(), path))
            .map_err(|e| format!("failed to write '{}': {}", path, e))
    }

    fn exec_list_directory(
        &self,
        input: &JsonValue,
        auditor: &mut Auditor,
        params_str: &str,
    ) -> Result<String, String> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("missing 'path' parameter")?;

        match self.caps.check_file_read(path) {
            CapabilityResult::Allowed => {
                auditor.log(AuditEvent::ToolCallAllowed {
                    tool: "list_directory",
                    params: params_str,
                });
            }
            CapabilityResult::Denied(reason) => {
                auditor.log(AuditEvent::ToolCallDenied {
                    tool: "list_directory",
                    params: params_str,
                    reason: &reason,
                });
                return Err(format!("access denied: {}", reason));
            }
        }

        let entries = fs::read_dir(path).map_err(|e| format!("failed to list '{}': {}", path, e))?;

        let mut lines = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("error reading entry: {}", e))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().map_err(|e| format!("error: {}", e))?;
            let suffix = if file_type.is_dir() { "/" } else { "" };
            lines.push(format!("{}{}", name, suffix));
        }

        lines.sort();
        Ok(lines.join("\n"))
    }

    fn exec_run_command(
        &self,
        input: &JsonValue,
        auditor: &mut Auditor,
        params_str: &str,
    ) -> Result<String, String> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("missing 'command' parameter")?;

        match self.caps.check_command(command) {
            CapabilityResult::Allowed => {
                auditor.log(AuditEvent::ToolCallAllowed {
                    tool: "run_command",
                    params: params_str,
                });
            }
            CapabilityResult::Denied(reason) => {
                auditor.log(AuditEvent::ToolCallDenied {
                    tool: "run_command",
                    params: params_str,
                    reason: &reason,
                });
                return Err(format!("access denied: {}", reason));
            }
        }

        let args: Vec<String> = input
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let mut child = Command::new(command)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to run '{}': {}", command, e))?;

        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Process finished — collect output
                    let mut stdout_buf = Vec::new();
                    let mut stderr_buf = Vec::new();
                    if let Some(ref mut out) = child.stdout {
                        let _ = out.read_to_end(&mut stdout_buf);
                    }
                    if let Some(ref mut err) = child.stderr {
                        let _ = err.read_to_end(&mut stderr_buf);
                    }

                    let stdout = String::from_utf8_lossy(&stdout_buf);
                    let stderr = String::from_utf8_lossy(&stderr_buf);

                    let mut result = String::new();
                    if !stdout.is_empty() {
                        result.push_str(&stdout);
                    }
                    if !stderr.is_empty() {
                        if !result.is_empty() {
                            result.push_str("\n--- stderr ---\n");
                        }
                        result.push_str(&stderr);
                    }

                    if status.success() {
                        return Ok(result);
                    } else {
                        return Err(format!(
                            "command exited with status {}\n{}",
                            status.code().unwrap_or(-1),
                            result
                        ));
                    }
                }
                Ok(None) => {
                    // Still running — check timeout
                    if start.elapsed() >= self.command_timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "command '{}' timed out after {}s",
                            command,
                            self.command_timeout.as_secs()
                        ));
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(format!("error waiting for '{}': {}", command, e));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::capability::CapabilityChecker;
    use crate::security::audit::Auditor;

    #[test]
    fn test_tool_definitions_count() {
        let defs = ToolExecutor::tool_definitions();
        assert_eq!(defs.len(), 4);
        assert_eq!(defs[0].name, "read_file");
        assert_eq!(defs[1].name, "write_file");
        assert_eq!(defs[2].name, "list_directory");
        assert_eq!(defs[3].name, "run_command");
    }

    #[test]
    fn test_command_timeout() {
        let checker = CapabilityChecker::new(
            vec![],
            vec![],
            vec!["sleep".into()],
        );
        let executor = ToolExecutor::new(&checker, 1); // 1 second timeout
        let mut auditor = Auditor::new(None);

        let input = json_obj()
            .field_str("command", "sleep")
            .field("args", json_arr().push_str("10").build())
            .build();

        let result = executor.execute("test-id", "run_command", &input, &mut auditor);
        match result {
            ContentBlock::ToolResult { is_error, content, .. } => {
                assert!(is_error, "should be an error");
                assert!(content.contains("timed out"), "should mention timeout: {}", content);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_command_success() {
        let checker = CapabilityChecker::new(
            vec![],
            vec![],
            vec!["echo".into()],
        );
        let executor = ToolExecutor::new(&checker, 5);
        let mut auditor = Auditor::new(None);

        let input = json_obj()
            .field_str("command", "echo")
            .field("args", json_arr().push_str("hello").build())
            .build();

        let result = executor.execute("test-id", "run_command", &input, &mut auditor);
        match result {
            ContentBlock::ToolResult { is_error, content, .. } => {
                assert!(!is_error, "should succeed");
                assert!(content.contains("hello"), "should contain output: {}", content);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_command_denied() {
        let checker = CapabilityChecker::new(
            vec![],
            vec![],
            vec!["echo".into()],
        );
        let executor = ToolExecutor::new(&checker, 5);
        let mut auditor = Auditor::new(None);

        let input = json_obj()
            .field_str("command", "rm")
            .build();

        let result = executor.execute("test-id", "run_command", &input, &mut auditor);
        match result {
            ContentBlock::ToolResult { is_error, content, .. } => {
                assert!(is_error, "should be denied");
                assert!(content.contains("access denied"), "should mention access denied: {}", content);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_unknown_tool() {
        let checker = CapabilityChecker::new(vec![], vec![], vec![]);
        let executor = ToolExecutor::new(&checker, 5);
        let mut auditor = Auditor::new(None);

        let input = JsonValue::Null;
        let result = executor.execute("test-id", "nonexistent_tool", &input, &mut auditor);
        match result {
            ContentBlock::ToolResult { is_error, content, .. } => {
                assert!(is_error);
                assert!(content.contains("unknown tool"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_read_file() {
        // Write a temp file, then read it via the tool
        let path = "/tmp/sentinel_test_read.txt";
        std::fs::write(path, "test content").unwrap();

        let checker = CapabilityChecker::new(
            vec!["/tmp".into()],
            vec![],
            vec![],
        );
        let executor = ToolExecutor::new(&checker, 5);
        let mut auditor = Auditor::new(None);

        let input = json_obj().field_str("path", path).build();
        let result = executor.execute("test-id", "read_file", &input, &mut auditor);
        match result {
            ContentBlock::ToolResult { is_error, content, .. } => {
                assert!(!is_error, "should succeed: {}", content);
                assert_eq!(content, "test content");
            }
            _ => panic!("expected ToolResult"),
        }

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_write_file() {
        let path = "/tmp/sentinel_test_write.txt";

        let checker = CapabilityChecker::new(
            vec![],
            vec!["/tmp".into()],
            vec![],
        );
        let executor = ToolExecutor::new(&checker, 5);
        let mut auditor = Auditor::new(None);

        let input = json_obj()
            .field_str("path", path)
            .field_str("content", "written by test")
            .build();
        let result = executor.execute("test-id", "write_file", &input, &mut auditor);
        match result {
            ContentBlock::ToolResult { is_error, content, .. } => {
                assert!(!is_error, "should succeed: {}", content);
                assert!(content.contains("wrote"));
            }
            _ => panic!("expected ToolResult"),
        }

        let written = std::fs::read_to_string(path).unwrap();
        assert_eq!(written, "written by test");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_list_directory() {
        let checker = CapabilityChecker::new(
            vec!["/tmp".into()],
            vec![],
            vec![],
        );
        let executor = ToolExecutor::new(&checker, 5);
        let mut auditor = Auditor::new(None);

        let input = json_obj().field_str("path", "/tmp").build();
        let result = executor.execute("test-id", "list_directory", &input, &mut auditor);
        match result {
            ContentBlock::ToolResult { is_error, .. } => {
                assert!(!is_error, "should succeed listing /tmp");
            }
            _ => panic!("expected ToolResult"),
        }
    }
}
