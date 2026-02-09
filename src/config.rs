use std::collections::HashMap;
use std::env;
use std::fs;

// ── Config struct ───────────────────────────────────────────────────────────

pub struct Config {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub openai_base_url: String,
    pub system_prompt: Option<String>,
    pub telegram_token: String,
    pub telegram_allowed_users: Vec<i64>,
    pub allowed_read_paths: Vec<String>,
    pub allowed_write_paths: Vec<String>,
    pub allowed_commands: Vec<String>,
    pub command_timeout: u64,
    pub audit_log_path: Option<String>,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config error: {}", self.0)
    }
}

// ── Loading ─────────────────────────────────────────────────────────────────

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        // Try loading TOML file
        let toml = try_load_toml();

        let get_str = |section: &str, key: &str, env_key: &str| -> Option<String> {
            // Check env var first
            if let Ok(val) = env::var(env_key) {
                return Some(val);
            }
            // Then TOML
            if let Some(ref t) = toml {
                if let Some(val) = t.get_str(section, key) {
                    return Some(val);
                }
            }
            None
        };

        let get_str_list = |section: &str, key: &str, env_key: &str| -> Vec<String> {
            // Env var: comma-separated
            if let Ok(val) = env::var(env_key) {
                return val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
            if let Some(ref t) = toml {
                if let Some(vals) = t.get_str_list(section, key) {
                    return vals;
                }
            }
            Vec::new()
        };

        let get_i64_list = |section: &str, key: &str, env_key: &str| -> Vec<i64> {
            if let Ok(val) = env::var(env_key) {
                return val
                    .split(',')
                    .filter_map(|s| s.trim().parse::<i64>().ok())
                    .collect();
            }
            if let Some(ref t) = toml {
                if let Some(vals) = t.get_i64_list(section, key) {
                    return vals;
                }
            }
            Vec::new()
        };

        // Provider selection: "anthropic" (default) or "openai"
        let provider = get_str("agent", "provider", "SENTINEL_PROVIDER")
            .unwrap_or_else(|| "anthropic".to_string());

        // API key: try provider-specific env first, then fall back
        let api_key = if provider == "openai" {
            resolve_secret(&toml, "openai", "api_key_env", "OPENAI_API_KEY")
                .or_else(|| resolve_secret(&toml, "anthropic", "api_key_env", "ANTHROPIC_API_KEY"))
                .ok_or_else(|| ConfigError("OPENAI_API_KEY not set".into()))?
        } else {
            resolve_secret(&toml, "anthropic", "api_key_env", "ANTHROPIC_API_KEY")
                .ok_or_else(|| ConfigError("ANTHROPIC_API_KEY not set".into()))?
        };

        let telegram_token = resolve_secret(&toml, "telegram", "token_env", "TELEGRAM_BOT_TOKEN")
            .ok_or_else(|| ConfigError("TELEGRAM_BOT_TOKEN not set".into()))?;

        let default_model = if provider == "openai" {
            "gpt-4o".to_string()
        } else {
            "claude-sonnet-4-5-20250929".to_string()
        };
        let model = get_str("anthropic", "model", "SENTINEL_MODEL")
            .or_else(|| get_str("openai", "model", "SENTINEL_MODEL"))
            .unwrap_or(default_model);

        let max_tokens = get_str("anthropic", "max_tokens", "SENTINEL_MAX_TOKENS")
            .or_else(|| get_str("openai", "max_tokens", "SENTINEL_MAX_TOKENS"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(4096);

        let openai_base_url = get_str("openai", "base_url", "OPENAI_BASE_URL")
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

        let system_prompt = get_str("agent", "system_prompt", "SENTINEL_SYSTEM_PROMPT");

        let telegram_allowed_users =
            get_i64_list("telegram", "allowed_users", "SENTINEL_ALLOWED_USERS");

        let allowed_read_paths =
            get_str_list("security", "allowed_read_paths", "SENTINEL_READ_PATHS");
        let allowed_write_paths =
            get_str_list("security", "allowed_write_paths", "SENTINEL_WRITE_PATHS");
        let allowed_commands =
            get_str_list("security", "allowed_commands", "SENTINEL_COMMANDS");

        let command_timeout = get_str("security", "command_timeout", "SENTINEL_COMMAND_TIMEOUT")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);

        let audit_log_path = get_str("security", "audit_log_path", "SENTINEL_AUDIT_LOG");

        Ok(Config {
            provider,
            api_key,
            model,
            max_tokens,
            openai_base_url,
            system_prompt,
            telegram_token,
            telegram_allowed_users,
            allowed_read_paths,
            allowed_write_paths,
            allowed_commands,
            command_timeout,
            audit_log_path,
        })
    }
}

fn resolve_secret(toml: &Option<TomlDoc>, section: &str, env_key_field: &str, fallback_env: &str) -> Option<String> {
    // Check if TOML specifies an env var name to read from
    if let Some(t) = toml {
        if let Some(env_name) = t.get_str(section, env_key_field) {
            if let Ok(val) = env::var(&env_name) {
                return Some(val);
            }
        }
    }
    // Fallback to direct env var
    env::var(fallback_env).ok()
}

// ── Minimal TOML parser ─────────────────────────────────────────────────────

struct TomlDoc {
    sections: HashMap<String, HashMap<String, TomlValue>>,
}

enum TomlValue {
    Str(String),
    Int(i64),
    StrList(Vec<String>),
    IntList(Vec<i64>),
}

impl TomlDoc {
    fn get_str(&self, section: &str, key: &str) -> Option<String> {
        match self.sections.get(section)?.get(key)? {
            TomlValue::Str(s) => Some(s.clone()),
            TomlValue::Int(n) => Some(n.to_string()),
            _ => None,
        }
    }

    fn get_str_list(&self, section: &str, key: &str) -> Option<Vec<String>> {
        match self.sections.get(section)?.get(key)? {
            TomlValue::StrList(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn get_i64_list(&self, section: &str, key: &str) -> Option<Vec<i64>> {
        match self.sections.get(section)?.get(key)? {
            TomlValue::IntList(v) => Some(v.clone()),
            _ => None,
        }
    }
}

fn try_load_toml() -> Option<TomlDoc> {
    let paths = ["sentinel.toml", "/etc/sentinel/sentinel.toml"];
    for path in &paths {
        if let Ok(content) = fs::read_to_string(path) {
            match parse_toml(&content) {
                Ok(doc) => return Some(doc),
                Err(e) => {
                    eprintln!("sentinel: warning: failed to parse {}: {}", path, e);
                }
            }
        }
    }
    None
}

fn parse_toml(input: &str) -> Result<TomlDoc, String> {
    let mut sections: HashMap<String, HashMap<String, TomlValue>> = HashMap::new();
    let mut current_section = String::new();

    for (line_num, raw_line) in input.lines().enumerate() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        // Section header
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_string();
            sections.entry(current_section.clone()).or_default();
            continue;
        }

        // Key = value
        let eq_pos = line.find('=').ok_or_else(|| {
            format!("line {}: expected '='", line_num + 1)
        })?;

        let key = line[..eq_pos].trim().to_string();
        let val_str = line[eq_pos + 1..].trim();

        let value = parse_toml_value(val_str).map_err(|e| {
            format!("line {}: {}", line_num + 1, e)
        })?;

        sections
            .entry(current_section.clone())
            .or_default()
            .insert(key, value);
    }

    Ok(TomlDoc { sections })
}

fn parse_toml_value(s: &str) -> Result<TomlValue, String> {
    let s = s.trim();

    // String
    if s.starts_with('"') {
        let end = s[1..]
            .find('"')
            .ok_or("unterminated string")?;
        return Ok(TomlValue::Str(s[1..end + 1].to_string()));
    }

    // Array
    if s.starts_with('[') {
        let inner = s
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or("unterminated array")?
            .trim();

        if inner.is_empty() {
            return Ok(TomlValue::StrList(Vec::new()));
        }

        // Determine type from first element
        let first = inner.split(',').next().unwrap_or("").trim();
        if first.starts_with('"') {
            let items: Vec<String> = inner
                .split(',')
                .filter_map(|item| {
                    let item = item.trim();
                    if item.starts_with('"') && item.ends_with('"') && item.len() >= 2 {
                        Some(item[1..item.len() - 1].to_string())
                    } else {
                        None
                    }
                })
                .collect();
            Ok(TomlValue::StrList(items))
        } else {
            let items: Vec<i64> = inner
                .split(',')
                .filter_map(|item| item.trim().parse::<i64>().ok())
                .collect();
            Ok(TomlValue::IntList(items))
        }
    } else if let Ok(n) = s.parse::<i64>() {
        Ok(TomlValue::Int(n))
    } else if s == "true" || s == "false" {
        Ok(TomlValue::Str(s.to_string()))
    } else {
        Err(format!("cannot parse value: {}", s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_toml() {
        let input = r#"
# Top-level config
[anthropic]
model = "claude-sonnet-4-5-20250929"
max_tokens = 4096
api_key_env = "MY_API_KEY"

[telegram]
allowed_users = [123, 456]

[security]
allowed_read_paths = ["/tmp", "/home/user"]
allowed_commands = ["ls", "cat"]
"#;

        let doc = parse_toml(input).unwrap();
        assert_eq!(
            doc.get_str("anthropic", "model").unwrap(),
            "claude-sonnet-4-5-20250929"
        );
        assert_eq!(
            doc.get_str("anthropic", "max_tokens").unwrap(),
            "4096"
        );
        assert_eq!(
            doc.get_i64_list("telegram", "allowed_users").unwrap(),
            vec![123, 456]
        );
        assert_eq!(
            doc.get_str_list("security", "allowed_read_paths").unwrap(),
            vec!["/tmp", "/home/user"]
        );
    }

    #[test]
    fn test_parse_toml_empty_arrays() {
        let input = r#"
[security]
allowed_commands = []
"#;
        let doc = parse_toml(input).unwrap();
        assert_eq!(
            doc.get_str_list("security", "allowed_commands").unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_parse_toml_boolean() {
        let input = r#"
[agent]
verbose = true
"#;
        let doc = parse_toml(input).unwrap();
        assert_eq!(doc.get_str("agent", "verbose").unwrap(), "true");
    }

    #[test]
    fn test_parse_toml_comments_stripped() {
        let input = r#"
[section]
key = "value" # this is a comment
"#;
        let doc = parse_toml(input).unwrap();
        assert_eq!(doc.get_str("section", "key").unwrap(), "value");
    }

    #[test]
    fn test_parse_toml_missing_key() {
        let input = r#"
[section]
key = "value"
"#;
        let doc = parse_toml(input).unwrap();
        assert!(doc.get_str("section", "nonexistent").is_none());
        assert!(doc.get_str("nonexistent", "key").is_none());
    }

    #[test]
    fn test_parse_toml_value_types() {
        assert!(matches!(parse_toml_value("42").unwrap(), TomlValue::Int(42)));
        assert!(matches!(parse_toml_value("-5").unwrap(), TomlValue::Int(-5)));
        assert!(matches!(parse_toml_value("\"hello\"").unwrap(), TomlValue::Str(s) if s == "hello"));
    }

    #[test]
    fn test_parse_toml_invalid_line() {
        let input = "no_section_key";
        assert!(parse_toml(input).is_err());
    }

    #[test]
    fn test_parse_toml_command_timeout() {
        let input = r#"
[security]
command_timeout = 60
"#;
        let doc = parse_toml(input).unwrap();
        assert_eq!(doc.get_str("security", "command_timeout").unwrap(), "60");
    }
}
