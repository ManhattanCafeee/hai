//! What a `source` resolves to: the bytes of a local address, or the URL the platform fetches itself.

use std::path::Path;

use super::SendMessage;
use crate::{
    agent::{
        channel::{OutboundPayload, OutboundSource},
        tools::addressing::Address,
    },
    agentcore::tool::{ToolError, tool_err},
    domain::vo::AttachmentParser,
};

impl SendMessage {
    pub(super) async fn media_payload(
        &self,
        source: Address,
        content: Option<String>,
    ) -> Result<OutboundPayload, ToolError> {
        let (media, filename) = self.media_source(source).await?;
        let sniffed = match &media {
            OutboundSource::Bytes(bytes) => AttachmentParser::from_bytes(bytes),
            OutboundSource::Url(_) => None,
        };
        let is_image = sniffed.or_else(|| AttachmentParser::from_filename(&filename))
            == Some(AttachmentParser::Image);
        if is_image {
            Ok(OutboundPayload::Photo {
                source: media,
                content,
            })
        } else {
            Ok(OutboundPayload::Document {
                source: media,
                filename,
                content,
            })
        }
    }

    /// The bytes of a local address, or the URL the platform fetches itself; either way with the name
    /// to fall back on.
    async fn media_source(&self, source: Address) -> Result<(OutboundSource, String), ToolError> {
        match &source {
            Address::Url(url) => Ok((OutboundSource::Url(url.clone()), file_name_of(url.path()))),
            Address::Local(_) | Address::Workspace(_) => {
                let path = source
                    .local_path(&self.sandbox)
                    .expect("the arm matched an address that names a local path");
                self.local_media(&path).await
            }
            _ => Err(tool_err(
                "only workspace://, file:// and http(s):// addresses can be sent",
            )),
        }
    }

    async fn local_media(&self, path: &Path) -> Result<(OutboundSource, String), ToolError> {
        self.sandbox.ensure_readable(path)?;
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| tool_err(format!("{}: {e}", path.display())))?;
        let limit = self.capabilities.max_media_bytes;
        if metadata.len() > limit {
            return Err(tool_err(format!(
                "{} is larger than the {} MB this platform takes",
                path.display(),
                limit / (1024 * 1024)
            )));
        }
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| tool_err(format!("{}: {e}", path.display())))?;
        Ok((
            OutboundSource::Bytes(bytes),
            file_name_of(&path.to_string_lossy()),
        ))
    }
}

/// The last path segment, or `file` when there is none.
fn file_name_of(path: &str) -> String {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("file")
        .to_owned()
}
