use crate::config::parse_toml;

// ── Skill manifest types ─────────────────────────────────────────────────────

pub struct SkillManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub binary: String,
    // Capabilities
    pub cap_network: bool,
    pub cap_file_read: Vec<String>,
    pub cap_file_write: Vec<String>,
    pub cap_commands: Vec<String>,
    // Tool definition
    pub tool_name: String,
    pub tool_description: String,
    pub parameters: Vec<SkillParam>,
}

pub struct SkillParam {
    pub name: String,
    pub param_type: String,
    pub description: String,
    pub required: bool,
}

// ── Parser ──────────────────────────────────────────────────────────────────

pub fn parse_manifest(content: &str) -> Result<SkillManifest, String> {
    let doc = parse_toml(content)?;

    // [skill] section
    let name = doc
        .get_str("skill", "name")
        .ok_or("skill.name is required")?;
    let version = doc.get_str("skill", "version").unwrap_or_else(|| "0.1.0".into());
    let description = doc
        .get_str("skill", "description")
        .unwrap_or_else(|| name.clone());
    let binary = doc
        .get_str("skill", "binary")
        .ok_or("skill.binary is required")?;

    // [capabilities] section
    let cap_network = doc
        .get_str("capabilities", "network")
        .map(|v| v == "true")
        .unwrap_or(false);
    let cap_file_read = doc
        .get_str_list("capabilities", "file_read")
        .unwrap_or_default();
    let cap_file_write = doc
        .get_str_list("capabilities", "file_write")
        .unwrap_or_default();
    let cap_commands = doc
        .get_str_list("capabilities", "commands")
        .unwrap_or_default();

    // [tool] section
    let tool_name = doc
        .get_str("tool", "name")
        .ok_or("tool.name is required")?;
    let tool_description = doc
        .get_str("tool", "description")
        .unwrap_or_else(|| description.clone());

    // Parameters from parallel arrays
    let param_names = doc
        .get_str_list("tool", "param_names")
        .unwrap_or_default();
    let param_types = doc
        .get_str_list("tool", "param_types")
        .unwrap_or_default();
    let param_descriptions = doc
        .get_str_list("tool", "param_descriptions")
        .unwrap_or_default();
    let param_required = doc
        .get_str_list("tool", "param_required")
        .unwrap_or_default();

    let mut parameters = Vec::new();
    for (i, name) in param_names.iter().enumerate() {
        parameters.push(SkillParam {
            name: name.clone(),
            param_type: param_types.get(i).cloned().unwrap_or_else(|| "string".into()),
            description: param_descriptions
                .get(i)
                .cloned()
                .unwrap_or_else(|| name.clone()),
            required: param_required.contains(name),
        });
    }

    // Validate tool_name is a valid identifier
    if !tool_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!(
            "tool.name '{}' must be alphanumeric with underscores",
            tool_name
        ));
    }

    Ok(SkillManifest {
        name,
        version,
        description,
        binary,
        cap_network,
        cap_file_read,
        cap_file_write,
        cap_commands,
        tool_name,
        tool_description,
        parameters,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest_basic() {
        let content = r#"
[skill]
name = "web-search"
version = "0.1.0"
description = "Search the web"
binary = "web-search"

[capabilities]
network = true
file_read = ["/tmp"]
file_write = []
commands = []

[tool]
name = "web_search"
description = "Search the web for information"
param_names = ["query"]
param_types = ["string"]
param_descriptions = ["The search query"]
param_required = ["query"]
"#;
        let manifest = parse_manifest(content).unwrap();
        assert_eq!(manifest.name, "web-search");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.binary, "web-search");
        assert!(manifest.cap_network);
        assert_eq!(manifest.cap_file_read, vec!["/tmp"]);
        assert_eq!(manifest.tool_name, "web_search");
        assert_eq!(manifest.parameters.len(), 1);
        assert_eq!(manifest.parameters[0].name, "query");
        assert!(manifest.parameters[0].required);
    }

    #[test]
    fn test_parse_manifest_minimal() {
        let content = r#"
[skill]
name = "hello"
binary = "hello-skill"

[tool]
name = "hello"
"#;
        let manifest = parse_manifest(content).unwrap();
        assert_eq!(manifest.name, "hello");
        assert_eq!(manifest.binary, "hello-skill");
        assert!(!manifest.cap_network);
        assert!(manifest.parameters.is_empty());
    }

    #[test]
    fn test_parse_manifest_missing_name() {
        let content = r#"
[skill]
binary = "test"

[tool]
name = "test"
"#;
        assert!(parse_manifest(content).is_err());
    }

    #[test]
    fn test_parse_manifest_invalid_tool_name() {
        let content = r#"
[skill]
name = "test"
binary = "test"

[tool]
name = "invalid-name"
"#;
        assert!(parse_manifest(content).is_err());
    }

    #[test]
    fn test_parse_manifest_multiple_params() {
        let content = r#"
[skill]
name = "calculator"
binary = "calc"

[capabilities]
network = false

[tool]
name = "calculate"
description = "Perform a calculation"
param_names = ["expression", "precision"]
param_types = ["string", "number"]
param_descriptions = ["Math expression", "Decimal places"]
param_required = ["expression"]
"#;
        let manifest = parse_manifest(content).unwrap();
        assert_eq!(manifest.parameters.len(), 2);
        assert!(manifest.parameters[0].required);
        assert!(!manifest.parameters[1].required);
        assert_eq!(manifest.parameters[1].param_type, "number");
    }
}
