//! The one MCP search engine: it drives whichever tool the provider table declares, so a new MCP
//! provider is a table row and not an adapter.

use std::sync::Arc;

use async_trait::async_trait;

use super::{SearchEngine, SearchRequest, SearchResponse, limit_of};
use crate::{
    agent::web::provider::{McpClient, Search},
    config::schema::WebProvider,
    error::Result,
};

pub(super) struct McpSearch {
    client: Arc<McpClient>,
    capability: &'static Search,
}

impl McpSearch {
    pub(super) fn new(client: Arc<McpClient>, capability: &'static Search) -> Self {
        Self { client, capability }
    }
}

#[async_trait]
impl SearchEngine for McpSearch {
    fn kind(&self) -> WebProvider {
        self.client.spec().id
    }

    async fn search(&self, req: &SearchRequest) -> Result<SearchResponse> {
        let limit = limit_of(req, self.client.spec().max_results);
        let arguments = (self.capability.arguments)(req, limit);
        let text = self.client.call(self.capability.tool, arguments).await?;
        (self.capability.parse)(&text, limit)
    }
}
