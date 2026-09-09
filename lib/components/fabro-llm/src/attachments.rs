//! Inlines local file attachments before a request reaches a codec.
//!
//! lithos accepts media as a URL or as base64. Fabro lets a caller point an
//! image, document, or audio part at a local path; this middleware reads the
//! file and rewrites the part to inline base64 with an inferred media type.
//! A part whose file cannot be read is dropped, so the model sees the rest of
//! the message rather than a request that fails outright.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use fabro_static::EnvVars;
use lithos_llm::middleware::{Call, Middleware, Next, Output};
use lithos_llm::types::{
    AudioContent, ContentPart, DocumentContent, Error, ImageContent, MediaSource, Message, Request,
    ToolResult,
};
use tokio::fs;

/// Resolves an environment variable name to its value.
type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Middleware that inlines local-path media parts.
#[derive(Clone, Default)]
pub struct InlineLocalAttachments {
    env_lookup: Option<EnvLookup>,
}

impl std::fmt::Debug for InlineLocalAttachments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineLocalAttachments")
            .finish_non_exhaustive()
    }
}

impl InlineLocalAttachments {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves `~/` against this lookup instead of the process environment.
    #[must_use]
    pub fn with_env_lookup(env_lookup: EnvLookup) -> Self {
        Self {
            env_lookup: Some(env_lookup),
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "Attachment path expansion supports the conventional HOME env var."
    )]
    fn home(&self) -> Option<String> {
        match &self.env_lookup {
            Some(lookup) => lookup(EnvVars::HOME),
            None => std::env::var(EnvVars::HOME).ok(),
        }
    }

    fn expand(&self, path: &str) -> String {
        path.strip_prefix("~/").map_or_else(
            || path.to_string(),
            |rest| format!("{}/{rest}", self.home().unwrap_or_else(|| "/".to_string())),
        )
    }

    async fn load(&self, path: &str) -> Option<MediaSource> {
        let expanded = self.expand(path);
        match fs::read(&expanded).await {
            Ok(bytes) => Some(MediaSource::base64(
                BASE64_STANDARD.encode(bytes),
                media_type_for_path(&expanded),
            )),
            Err(err) => {
                tracing::warn!(path = %expanded, error = %err, "dropping unreadable attachment");
                None
            }
        }
    }

    async fn inline_part(&self, part: ContentPart) -> Option<ContentPart> {
        match part {
            ContentPart::Image(ImageContent { source, detail }) if is_local_file(&source) => {
                let source = self.load(url_of(&source)).await?;
                Some(ContentPart::Image(ImageContent { source, detail }))
            }
            ContentPart::Document(DocumentContent { source, name }) if is_local_file(&source) => {
                let source = self.load(url_of(&source)).await?;
                Some(ContentPart::Document(DocumentContent { source, name }))
            }
            ContentPart::Audio(AudioContent { source }) if is_local_file(&source) => {
                let source = self.load(url_of(&source)).await?;
                Some(ContentPart::Audio(AudioContent { source }))
            }
            ContentPart::ToolResult(result) if result.content.iter().any(part_is_local_file) => {
                let mut content = Vec::with_capacity(result.content.len());
                for part in result.content {
                    if let Some(part) = Box::pin(self.inline_part(part)).await {
                        content.push(part);
                    }
                }
                Some(ContentPart::ToolResult(ToolResult { content, ..result }))
            }
            other => Some(other),
        }
    }

    async fn inline_request(&self, request: Request) -> Request {
        let mut messages = Vec::with_capacity(request.messages().len());
        for message in request.messages() {
            let mut content = Vec::with_capacity(message.content().len());
            for part in message.content() {
                if let Some(part) = self.inline_part(part.clone()).await {
                    content.push(part);
                }
            }
            let mut rebuilt = Message::new(message.role(), content);
            if let Some(name) = message.name() {
                rebuilt = rebuilt.with_name(name);
            }
            if let Some(id) = message.tool_call_id() {
                rebuilt = rebuilt.with_tool_call_id(id);
            }
            messages.push(rebuilt);
        }
        replace_messages(&request, messages).unwrap_or(request)
    }
}

/// Rebuilds `request` with `messages` in place of its own.
///
/// The request builder appends messages and has no way to clear them, so the
/// swap goes through the request's serde form.
fn replace_messages(request: &Request, messages: Vec<Message>) -> Option<Request> {
    let mut value = serde_json::to_value(request).ok()?;
    value["messages"] = serde_json::to_value(messages).ok()?;
    serde_json::from_value(value).ok()
}

fn part_is_local_file(part: &ContentPart) -> bool {
    match part {
        ContentPart::Image(ImageContent { source, .. })
        | ContentPart::Document(DocumentContent { source, .. })
        | ContentPart::Audio(AudioContent { source }) => is_local_file(source),
        _ => false,
    }
}

fn url_of(source: &MediaSource) -> &str {
    match source {
        MediaSource::Url { url, .. } => url,
        _ => "",
    }
}

fn is_local_file(source: &MediaSource) -> bool {
    matches!(
        source,
        MediaSource::Url { url, .. }
            if url.starts_with('/') || url.starts_with("./") || url.starts_with("~/")
    )
}

fn needs_inlining(request: &Request) -> bool {
    request.messages().iter().any(|message| {
        message.content().iter().any(|part| match part {
            ContentPart::ToolResult(result) => result.content.iter().any(part_is_local_file),
            part => part_is_local_file(part),
        })
    })
}

/// Media type for a local path, from its extension.
#[must_use]
pub fn media_type_for_path(path: &str) -> String {
    mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string()
}

#[async_trait]
impl Middleware for InlineLocalAttachments {
    async fn handle(&self, call: Call, next: Next) -> Result<Output, Error> {
        if !needs_inlining(call.request()) {
            return next.run(call).await;
        }
        let inlined = self.inline_request(call.request().clone()).await;
        let call = call.map_request(|_| Ok(inlined))?;
        next.run(call).await
    }
}

#[cfg(test)]
mod tests {
    use lithos_llm::types::Role;

    use super::*;

    fn request_with(part: ContentPart) -> Request {
        Request::builder()
            .model("openai/gpt-5.4")
            .message(Message::new(Role::User, [
                ContentPart::Text {
                    text: "look".to_string(),
                },
                part,
            ]))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn inlines_local_images_and_drops_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pixel.png");
        fs::write(&path, b"\x89PNG").await.unwrap();
        let middleware = InlineLocalAttachments::new();

        let request = request_with(ContentPart::Image(ImageContent::new(MediaSource::url(
            path.to_string_lossy().to_string(),
        ))));
        let inlined = middleware.inline_request(request).await;
        match &inlined.messages()[0].content()[1] {
            ContentPart::Image(image) => {
                assert_eq!(image.source.media_type(), Some("image/png"));
                assert_eq!(
                    image.source.base64_data(),
                    Some(BASE64_STANDARD.encode(b"\x89PNG").as_str())
                );
            }
            other => panic!("expected inlined image, got {other:?}"),
        }

        let missing = request_with(ContentPart::Document(DocumentContent::new(
            MediaSource::url("/definitely/missing.pdf"),
        )));
        let inlined = middleware.inline_request(missing).await;
        assert_eq!(inlined.messages()[0].content().len(), 1);
    }

    #[test]
    fn remote_urls_and_inline_data_pass_through() {
        let request = request_with(ContentPart::Image(ImageContent::new(MediaSource::url(
            "https://example.com/a.png",
        ))));
        assert!(!needs_inlining(&request));
        let request = request_with(ContentPart::Image(ImageContent::new(MediaSource::base64(
            "AAAA",
            "image/png",
        ))));
        assert!(!needs_inlining(&request));
        let request = request_with(ContentPart::Image(ImageContent::new(MediaSource::url(
            "~/shot.png",
        ))));
        assert!(needs_inlining(&request));
    }

    #[test]
    fn media_types_follow_extensions() {
        assert_eq!(media_type_for_path("a.jpg"), "image/jpeg");
        assert_eq!(media_type_for_path("a.pdf"), "application/pdf");
        assert_eq!(media_type_for_path("a.bin"), "application/octet-stream");
    }
}
