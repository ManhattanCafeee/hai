//! The image producer: making a picture is not sending one, so this tool only writes it down.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    agent::{multimodal::MultimodalService, runtime::sandbox::Sandbox},
    agentcore::{
        api::ImageUrl,
        tool::{AgentTool, ToolError, schema_of, tool_data, tool_err},
    },
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateImageArgs {
    /// Image generation prompt; describe the scene, style and composition you want in detail
    pub prompt: String,
    /// Reference images that guide the generation
    #[schemars(with = "Option<Vec<String>>")]
    pub reference_images: Option<Vec<ImageUrl>>,
}

pub struct GenerateImage {
    pub(super) sandbox: Arc<Sandbox>,
    pub(super) multimodal: MultimodalService,
}

#[async_trait]
impl AgentTool for GenerateImage {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn description(&self) -> &str {
        "Generate one image into the workspace and return its path. Nothing is sent: pass that path to \
         send_message with kind \"file\" if the image should reach the chat."
    }

    fn schema(&self) -> Option<Value> {
        Some(schema_of::<GenerateImageArgs>())
    }

    fn produces(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value, ToolError> {
        let args: GenerateImageArgs = serde_json::from_value(args)?;
        let references = args.reference_images.unwrap_or_default();
        let bytes = self
            .multimodal
            .generate_image(&args.prompt, &references)
            .await?;
        let path = write_image(&self.sandbox, &bytes, &args.prompt).await?;

        tool_data(json!({ "path": path }))
    }
}

async fn write_image(sandbox: &Sandbox, bytes: &[u8], prompt: &str) -> Result<String, ToolError> {
    let dir = sandbox.host_workspace().join("generated");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| tool_err(format!("{}: {e}", dir.display())))?;

    let stem = format!(
        "{}-{}",
        jiff::Timestamp::now().as_millisecond(),
        slug(prompt)
    );
    let mut name = format!("{stem}.png");
    let mut attempt = 1;
    while dir.join(&name).exists() {
        name = format!("{stem}-{attempt}.png");
        attempt += 1;
    }
    let path = dir.join(&name);
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| tool_err(format!("{}: {e}", path.display())))?;

    Ok(format!("workspace://generated/{name}"))
}

fn slug(prompt: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in prompt.chars() {
        if slug.len() >= 24 {
            break;
        }
        if character.is_ascii_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.push(character.to_ascii_lowercase());
        } else if character == '_' {
            separator = false;
            slug.push('_');
        } else {
            separator = true;
        }
    }
    if slug.is_empty() {
        "image".to_owned()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::{Sandbox, write_image};

    #[test]
    fn a_prompt_becomes_a_file_name_front() {
        assert_eq!(super::slug("A Cat, sitting!"), "a-cat-sitting");
        assert_eq!(super::slug("!!!"), "image");
        assert_eq!(super::slug(""), "image");
        assert!(super::slug(&"x".repeat(50)).len() <= 24);
    }

    #[tokio::test]
    async fn a_generated_image_lands_in_the_workspace() {
        let (workspace, _dir) = crate::agent::test_support::created_workspace().await;
        let sandbox = Sandbox::host(workspace, 30);

        let first = write_image(&sandbox, b"png", "A Cat, sitting!")
            .await
            .expect("write");
        assert!(
            first.starts_with("workspace://generated/") && first.ends_with("-a-cat-sitting.png"),
            "{first}"
        );
        let name = first.trim_start_matches("workspace://");
        assert!(
            tokio::fs::metadata(sandbox.host_workspace().join(name))
                .await
                .is_ok(),
            "the file is on disk at {first}"
        );

        let second = write_image(&sandbox, b"png", "A Cat, sitting!")
            .await
            .expect("write");
        assert_ne!(first, second, "a second image does not overwrite the first");
    }
}
