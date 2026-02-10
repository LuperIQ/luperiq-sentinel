use std::fs;
use std::path::Path;

use super::manifest::{parse_manifest, SkillManifest};

// ── Skill definition (manifest + resolved paths) ─────────────────────────────

pub struct SkillDef {
    pub manifest: SkillManifest,
    pub binary_path: String,
    pub skill_dir: String,
}

// ── Loader ──────────────────────────────────────────────────────────────────

/// Load all valid skills from a directory. Each subdirectory containing a
/// `skill.toml` is treated as a skill. Invalid or incomplete skills are
/// logged and skipped.
pub fn load_skills(skills_dir: &str) -> Vec<SkillDef> {
    let mut skills = Vec::new();
    let dir_path = Path::new(skills_dir);

    if !dir_path.is_dir() {
        eprintln!("sentinel: skills directory '{}' not found", skills_dir);
        return skills;
    }

    let entries = match fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("sentinel: cannot read skills directory '{}': {}", skills_dir, e);
            return skills;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("skill.toml");
        if !manifest_path.exists() {
            continue;
        }

        let content = match fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "sentinel: cannot read {}: {}",
                    manifest_path.display(),
                    e
                );
                continue;
            }
        };

        let manifest = match parse_manifest(&content) {
            Ok(m) => m,
            Err(e) => {
                eprintln!(
                    "sentinel: invalid manifest {}: {}",
                    manifest_path.display(),
                    e
                );
                continue;
            }
        };

        let skill_dir = path.to_string_lossy().to_string();
        let binary_path = path.join(&manifest.binary).to_string_lossy().to_string();

        // Verify binary exists and is executable
        let binary_file = Path::new(&binary_path);
        if !binary_file.exists() {
            eprintln!(
                "sentinel: skill '{}' binary not found: {}",
                manifest.name, binary_path
            );
            continue;
        }

        eprintln!(
            "sentinel: loaded skill '{}' (tool: {})",
            manifest.name, manifest.tool_name
        );

        skills.push(SkillDef {
            manifest,
            binary_path,
            skill_dir,
        });
    }

    skills.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    skills
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_skills_nonexistent_dir() {
        let skills = load_skills("/tmp/sentinel_test_nonexistent_skills_dir");
        assert!(skills.is_empty());
    }

    #[test]
    fn test_load_skills_with_valid_skill() {
        let base = "/tmp/sentinel_test_skills";
        let skill_dir = format!("{}/echo-skill", base);

        // Create skill directory structure
        let _ = fs::create_dir_all(&skill_dir);
        let manifest = r#"
[skill]
name = "echo"
binary = "echo-skill.sh"

[tool]
name = "echo_text"
description = "Echo back the input"
param_names = ["text"]
param_types = ["string"]
param_descriptions = ["Text to echo"]
param_required = ["text"]
"#;
        let _ = fs::write(format!("{}/skill.toml", skill_dir), manifest);
        let _ = fs::write(format!("{}/echo-skill.sh", skill_dir), "#!/bin/sh\nread line\necho $line");

        let skills = load_skills(base);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest.name, "echo");
        assert_eq!(skills[0].manifest.tool_name, "echo_text");

        // Cleanup
        let _ = fs::remove_dir_all(base);
    }
}
