use std::fs;
use std::process::Command;

use crate::llm::anthropic::{ContentBlock, ToolDef};
use crate::net::json::{json_obj, json_arr, JsonValue};
use crate::security::audit::{AuditEvent, Auditor};
use crate::security::capability::{CapabilityChecker, CapabilityResult};

// ── Tool executor ───────────────────────────────────────────────────────────

pub struct ToolExecutor<'a> {
    caps: &'a CapabilityChecker,
}

impl<'a> ToolExecutor<'a> {
    pub fn new(caps: &'a CapabilityChecker) -> Self {
        ToolExecutor { caps }
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

        let output = Command::new(command)
            .args(&args)
            .output()
            .map_err(|e| format!("failed to run '{}': {}", command, e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

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

        if output.status.success() {
            Ok(result)
        } else {
            Err(format!(
                "command exited with status {}\n{}",
                output.status.code().unwrap_or(-1),
                result
            ))
        }
    }
}
