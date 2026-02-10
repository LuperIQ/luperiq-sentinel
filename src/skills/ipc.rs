use std::io::{BufRead, BufReader, Write};
use std::thread;
use std::time::{Duration, Instant};

use crate::net::json::{self, json_obj, JsonValue};

use super::sandbox::SandboxedProcess;

// ── Skill IPC protocol ──────────────────────────────────────────────────────
//
// Request (written to skill's stdin):
//   {"params":{"key":"value"}}\n
//
// Response (read from skill's stdout):
//   {"result":"output text"}\n
//   or
//   {"error":"error message"}\n

/// Invoke a skill binary with the given parameters and return the result.
/// The skill process is spawned, given the request, and expected to respond
/// with a single JSON line. Killed after timeout_secs if no response.
pub fn invoke_skill(
    process: &mut SandboxedProcess,
    params: &JsonValue,
    timeout_secs: u64,
) -> Result<String, String> {
    // Build request JSON
    let request = json_obj().field("params", params.clone()).build();
    let request_str = format!("{}\n", request.to_json_string());

    // Write request to stdin then close it
    {
        let stdin = process
            .stdin()
            .ok_or("failed to get skill stdin")?;
        stdin
            .write_all(request_str.as_bytes())
            .map_err(|e| format!("failed to write to skill stdin: {}", e))?;
        stdin
            .flush()
            .map_err(|e| format!("failed to flush skill stdin: {}", e))?;
    }
    // Drop stdin to signal EOF to the child
    // (take it from the child so it gets closed)
    drop(process.stdin().take());

    // Read response with timeout
    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();

    // Wait for the child to exit (with timeout)
    loop {
        match process.try_wait() {
            Ok(Some(_exit_code)) => {
                // Child exited — read stdout
                break;
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    process.kill();
                    return Err(format!("skill timed out after {}s", timeout_secs));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                process.kill();
                return Err(format!("error waiting for skill: {}", e));
            }
        }
    }

    // Read all of stdout
    let stdout = process
        .stdout()
        .ok_or("failed to get skill stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .map_err(|e| format!("failed to read skill response: {}", e))?;

    let response_line = response_line.trim();
    if response_line.is_empty() {
        return Err("skill produced no output".into());
    }

    // Parse response JSON
    let json_val = json::parse(response_line)
        .map_err(|e| format!("skill response is not valid JSON: {}", e))?;

    // Check for error
    if let Some(err) = json_val.get("error") {
        if let Some(err_str) = err.as_str() {
            return Err(format!("skill error: {}", err_str));
        }
    }

    // Get result
    if let Some(result) = json_val.get("result") {
        if let Some(s) = result.as_str() {
            return Ok(s.to_string());
        }
        // If result is not a string, serialize it
        return Ok(result.to_json_string());
    }

    // No result or error field — return the whole response
    Ok(response_line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_invoke_skill_echo() {
        // Create a simple skill script that echoes back the input
        let script_dir = "/tmp/sentinel_test_ipc";
        let script_path = format!("{}/echo.sh", script_dir);
        let _ = fs::create_dir_all(script_dir);
        fs::write(
            &script_path,
            "#!/bin/sh\nread line\necho '{\"result\":\"got it\"}'\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut process = SandboxedProcess::spawn(&script_path, script_dir).unwrap();
        let params = json_obj().field_str("text", "hello").build();
        let result = invoke_skill(&mut process, &params, 5);
        assert!(result.is_ok(), "should succeed: {:?}", result);
        assert_eq!(result.unwrap(), "got it");

        let _ = fs::remove_dir_all(script_dir);
    }

    #[test]
    fn test_invoke_skill_error() {
        let script_dir = "/tmp/sentinel_test_ipc_err";
        let script_path = format!("{}/err.sh", script_dir);
        let _ = fs::create_dir_all(script_dir);
        fs::write(
            &script_path,
            "#!/bin/sh\nread line\necho '{\"error\":\"something failed\"}'\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut process = SandboxedProcess::spawn(&script_path, script_dir).unwrap();
        let params = json_obj().build();
        let result = invoke_skill(&mut process, &params, 5);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("something failed"));

        let _ = fs::remove_dir_all(script_dir);
    }

    #[test]
    fn test_invoke_skill_timeout() {
        let script_dir = "/tmp/sentinel_test_ipc_timeout";
        let script_path = format!("{}/slow.sh", script_dir);
        let _ = fs::create_dir_all(script_dir);
        fs::write(
            &script_path,
            "#!/bin/sh\nsleep 30\necho '{\"result\":\"too late\"}'\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut process = SandboxedProcess::spawn(&script_path, script_dir).unwrap();
        let params = json_obj().build();
        let result = invoke_skill(&mut process, &params, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));

        let _ = fs::remove_dir_all(script_dir);
    }
}
