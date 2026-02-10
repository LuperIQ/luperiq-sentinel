pub mod ipc;
pub mod loader;
pub mod manifest;
pub mod sandbox;

use crate::llm::provider::ToolDef;
use crate::net::json::{json_arr, json_obj, JsonValue};
use crate::security::audit::{AuditEvent, Auditor};

use loader::SkillDef;
use sandbox::SandboxedProcess;

// ── Skill runner ─────────────────────────────────────────────────────────────

pub struct SkillRunner {
    skills: Vec<SkillDef>,
    skill_timeout: u64,
}

impl SkillRunner {
    /// Load skills from a directory and create a runner.
    pub fn load(skills_dir: &str, skill_timeout: u64) -> Self {
        let skills = loader::load_skills(skills_dir);
        eprintln!("sentinel: loaded {} skill(s)", skills.len());
        SkillRunner {
            skills,
            skill_timeout,
        }
    }

    /// Returns true if any skills were loaded.
    pub fn has_skills(&self) -> bool {
        !self.skills.is_empty()
    }

    /// Generate ToolDef instances for each loaded skill.
    pub fn tool_definitions(&self) -> Vec<ToolDef> {
        self.skills
            .iter()
            .map(|skill| {
                let m = &skill.manifest;

                // Build properties object from parameters
                let mut props = json_obj();
                let mut required = json_arr();

                for param in &m.parameters {
                    let prop = json_obj()
                        .field_str("type", &param.param_type)
                        .field_str("description", &param.description)
                        .build();
                    props = props.field(&param.name, prop);

                    if param.required {
                        required = required.push_str(&param.name);
                    }
                }

                ToolDef {
                    name: m.tool_name.clone(),
                    description: m.tool_description.clone(),
                    input_schema: json_obj()
                        .field_str("type", "object")
                        .field("properties", props.build())
                        .field("required", required.build())
                        .build(),
                }
            })
            .collect()
    }

    /// Check if this runner handles a given tool name.
    pub fn handles(&self, tool_name: &str) -> bool {
        self.skills
            .iter()
            .any(|s| s.manifest.tool_name == tool_name)
    }

    /// Execute a skill tool invocation.
    pub fn execute(
        &self,
        tool_name: &str,
        input: &JsonValue,
        auditor: &mut Auditor,
    ) -> Result<String, String> {
        let skill = self
            .skills
            .iter()
            .find(|s| s.manifest.tool_name == tool_name)
            .ok_or_else(|| format!("unknown skill tool: {}", tool_name))?;

        let params_str = input.to_json_string();
        auditor.log(AuditEvent::ToolCallAllowed {
            tool: &format!("skill:{}", tool_name),
            params: &params_str,
        });

        eprintln!(
            "sentinel: invoking skill '{}' ({})",
            skill.manifest.name, skill.binary_path
        );

        // Spawn sandboxed process
        let mut process = SandboxedProcess::spawn(&skill.binary_path, &skill.skill_dir)?;

        // Invoke via IPC
        let result = ipc::invoke_skill(&mut process, input, self.skill_timeout);

        match &result {
            Ok(output) => {
                eprintln!(
                    "sentinel: skill '{}' completed ({} bytes output)",
                    skill.manifest.name,
                    output.len()
                );
            }
            Err(e) => {
                eprintln!("sentinel: skill '{}' failed: {}", skill.manifest.name, e);
            }
        }

        result
    }
}
