use super::{Read, ReadOutput};
use crate::{
    agentcore::tool::ToolError,
    domain::vo::{AttachmentId, AttachmentParser},
};

impl Read {
    pub(super) async fn read_attachment(
        &self,
        id: AttachmentId,
        parser: Option<AttachmentParser>,
        focus: Option<&str>,
    ) -> Result<ReadOutput, ToolError> {
        let content = self.analyzer.analyze_attachment(id, parser, focus).await?;
        Ok(ReadOutput::Text { content })
    }
}
