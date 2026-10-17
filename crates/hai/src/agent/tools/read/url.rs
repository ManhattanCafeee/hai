use super::{Read, ReadArgs, ReadOutput, paging::paginate};
use crate::{
    agent::web::{Fetched, WebUrl},
    agentcore::tool::ToolError,
    domain::vo::Subject,
};

impl Read {
    pub(super) async fn read_url(
        &self,
        url: &WebUrl,
        args: &ReadArgs,
    ) -> Result<ReadOutput, ToolError> {
        match self.web.fetch(url).await? {
            Fetched::Text { text, engine } => Ok(ReadOutput::Paged(paginate(
                &text,
                args.offset,
                args.limit,
                Some(engine),
            ))),
            fetched => {
                let (payload, parser) = fetched
                    .into_payload()
                    .expect("a non-text fetch carries a payload");
                let content = self
                    .analyzer
                    .analyze(
                        payload,
                        parser,
                        args.parser,
                        Some(Subject::url(url.as_str())),
                        args.focus.as_deref(),
                    )
                    .await?;
                Ok(ReadOutput::Text { content })
            }
        }
    }
}
