//! Fork-owned structured-output completion (fabro-4c11).
//!
//! lithos's [`Client::complete_object`] attaches the schema as the request's
//! response format — so providers with native structured output enforce it —
//! but reads the reply with [`Response::json_object`], which parses only a
//! bare JSON document. A reply that wraps the document in prose or a Markdown
//! fence (the observed zai/glm-4.7 behavior) fails at column 1 and callers
//! fall back to deterministic skeletons. This module keeps the same request
//! shape and adds a prose/fence-tolerant decode layer on top of the reply.

use lithos_llm::client::Client;
use lithos_llm::types::{Error, ErrorKind, Request, Response, ResponseFormat};
use tracing::warn;

/// A structured reply: the completion plus the decoded JSON document, like
/// lithos's own `StructuredCompletion` but built by the tolerant decode.
#[derive(Clone, Debug)]
pub struct TolerantCompletion {
    pub response: Response,
    pub object:   serde_json::Value,
}

/// [`Client::complete_object`] with a prose/fence-tolerant reply decode.
///
/// The schema is attached exactly as lithos's own method does, so a provider
/// with native structured output still enforces it and the selected model
/// must still declare the capability. Only the reply reading differs: a
/// document embedded in prose or a code fence is extracted instead of
/// failing the call.
///
/// # Errors
///
/// Every error [`Client::complete`] returns, plus the decode error
/// [`Response::json_object`] produces when the reply hides no JSON document.
pub async fn complete_object_tolerant(
    client: &Client,
    request: Request,
    schema_name: &str,
    schema: serde_json::Value,
) -> Result<TolerantCompletion, Error> {
    let request = request
        .into_builder()
        .response_format(ResponseFormat::JsonSchema {
            name: schema_name.to_owned(),
            schema,
        })
        .build()
        .map_err(|source| {
            Error::new(
                ErrorKind::InvalidRequest,
                "the structured output request is invalid",
            )
            .with_source(source)
        })?;
    let response = client.complete(request).await?;
    let object = decode_json_object(&response)?;
    Ok(TolerantCompletion { response, object })
}

/// [`Response::json_object`] with a prose/fence-tolerant fallback.
///
/// A native JSON part or a bare text document is taken as is (the lithos
/// fast path). Otherwise the first fenced or balanced JSON document in the
/// reply text is extracted, and a warn event names the recovery so a
/// misbehaving provider stays observable.
///
/// # Errors
///
/// The error [`Response::json_object`] returns when the reply carries no
/// JSON document at all.
pub fn decode_json_object(response: &Response) -> Result<serde_json::Value, Error> {
    let direct = response.json_object();
    if let Ok(object) = &direct {
        return Ok(object.clone());
    }
    let text = response.text();
    if let Some(object) = extract_json_document(&text) {
        warn!(
            provider = %response.model.provider(),
            model = %response.model.model(),
            "extracted a JSON document from a prose reply",
        );
        return Ok(object);
    }
    direct
}

/// The first JSON document in `text`: the whole trimmed text if it parses,
/// then fenced code blocks in order, then balanced top-level documents.
pub fn extract_json_document(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    for candidate in fenced_blocks(trimmed)
        .into_iter()
        .chain(balanced_documents(trimmed))
    {
        if let Ok(value) = serde_json::from_str(candidate) {
            return Some(value);
        }
    }
    None
}

/// The body of every ```-fenced block in order, trimmed. The fence tag line
/// (`json`, and so on) is skipped.
fn fenced_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let Some(newline) = after.find('\n') else {
            break;
        };
        let body = &after[newline + 1..];
        let Some(end) = body.find("```") else {
            break;
        };
        blocks.push(body[..end].trim());
        rest = &body[end + 3..];
    }
    blocks
}

/// Every top-level balanced `{...}` / `[...]` span in order, ignoring braces
/// inside JSON strings. Nested documents surface only through their outer
/// span, which is the document the model meant.
fn balanced_documents(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut depth = 0usize;
    let mut open = 0usize;
    let mut open_is_object = false;
    let mut in_string = false;
    let mut escaped = false;
    for (index, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                if depth == 0 {
                    open = index;
                    open_is_object = byte == b'{';
                }
                depth += 1;
            }
            b'}' | b']' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 && (byte == b'}') == open_is_object {
                        spans.push(&text[open..=index]);
                    }
                }
            }
            _ => {}
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::extract_json_document;

    #[test]
    fn a_bare_document_parses_directly() {
        assert_eq!(
            extract_json_document(r#"  {"title": "t", "body": "b"}  "#),
            Some(json!({"title": "t", "body": "b"}))
        );
    }

    #[test]
    fn a_fenced_document_is_extracted() {
        let text =
            "Sure! Here is the PR content:\n```json\n{\"title\": \"t\"}\n```\nHope that helps.";
        assert_eq!(extract_json_document(text), Some(json!({"title": "t"})));
    }

    #[test]
    fn a_prose_wrapped_document_is_extracted() {
        let text =
            r#"The title is "Add x". The document: {"body": "has } inside a string? no"} Done."#;
        assert_eq!(
            extract_json_document(text),
            Some(json!({"body": "has } inside a string? no"}))
        );
    }

    #[test]
    fn braces_inside_strings_do_not_close_a_document() {
        let text = r#"prose {"a": "}", "b": ["{"] } trailing"#;
        assert_eq!(
            extract_json_document(text),
            Some(json!({"a": "}", "b": ["{"]}))
        );
    }

    #[test]
    fn prose_without_a_document_yields_none() {
        assert_eq!(
            extract_json_document("Sure! I would title this PR: Add the feature. Prose only."),
            None
        );
    }

    #[test]
    fn an_unterminated_fence_falls_through_to_balanced_spans() {
        let text = "```json\n{\"title\": \"t\"}\n(no closing fence)";
        assert_eq!(extract_json_document(text), Some(json!({"title": "t"})));
    }
}
