use super::ReadOutput;
use crate::agentcore::{
    skills::{SkillManager, SkillName},
    tool::{ToolError, tool_err},
};

pub(super) fn skill_body(skills: &SkillManager, name: &SkillName) -> Result<ReadOutput, ToolError> {
    let skill = skills
        .find(name)
        .ok_or_else(|| tool_err(format!("Skill '{}' not found", name.as_str())))?;
    Ok(ReadOutput::Skill {
        name: skill.name.clone(),
        instructions: skill.body(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[tokio::test]
    async fn skill_body_substitutes_base_dir() {
        let root = TempDir::create("hai-skill-test");
        let dir = root.path().join("demo");
        std::fs::create_dir_all(&dir).expect("skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo\ndescription: demo skill\n---\n\nInstructions live in {baseDir}\n",
        )
        .expect("skill file");

        let skills = SkillManager::load(&[root.path().to_path_buf()], &[])
            .await
            .expect("load");

        match skill_body(&skills, &SkillName::new("demo").expect("name")).expect("found") {
            ReadOutput::Skill { name, instructions } => {
                assert_eq!(name, "demo");
                assert!(!instructions.contains("{baseDir}"));
                assert!(instructions.contains(&dir.display().to_string()));
            }
            _ => panic!("expected a skill output"),
        }

        match skill_body(&skills, &SkillName::new("nope").expect("name")) {
            Err(err) => assert_eq!(err.to_string(), "Skill 'nope' not found"),
            Ok(_) => panic!("unknown skill must fail"),
        }
    }
}
