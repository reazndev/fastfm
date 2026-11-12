//! `example.echo`: returns its `text` parameter. The template for real methods.

use async_trait::async_trait;
use lfm_core::{ApiError, Node, PageSpec, Paging, Response};

use crate::method::{Context, Method};

const PAGING: PageSpec = PageSpec::new(50, 1000);

/// Echoes `text` back with paging attributes, as
/// `<echo page="1" perPage="50"><text>…</text></echo>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Echo;

#[async_trait]
impl Method for Echo {
    fn name(&self) -> &'static str {
        "example.echo"
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let text = ctx.params.required("text")?;
        let paging = Paging::parse(ctx.params, PAGING);
        let root = Node::new("echo")
            .attr("page", paging.page)
            .attr("perPage", paging.limit)
            .child(Node::leaf("text", text.to_owned()));
        Ok(Response::new(root))
    }
}
