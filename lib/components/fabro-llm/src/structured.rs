//! One-shot structured output.

use lithos_llm::client::Client;
use lithos_llm::middleware::CallContext;
use lithos_llm::types::{Error, ErrorKind, Request, Response, ResponseFormat};

/// A completion whose text parsed as the requested JSON object.
#[derive(Debug, Clone)]
pub struct StructuredCompletion {
    pub response: Response,
    pub object:   serde_json::Value,
}

/// Completes `request` under a JSON schema and parses the reply.
///
/// The schema is attached as the request's response format, so providers
/// with native structured output enforce it. The reply text must still parse
/// as JSON; a reply that does not is a `ResponseDecode` error.
pub async fn complete_object(
    client: &Client,
    request: Request,
    schema_name: &str,
    schema: serde_json::Value,
) -> Result<StructuredCompletion, Error> {
    complete_object_with_context(client, request, schema_name, schema, CallContext::new()).await
}

pub async fn complete_object_with_context(
    client: &Client,
    request: Request,
    schema_name: &str,
    schema: serde_json::Value,
    context: CallContext,
) -> Result<StructuredCompletion, Error> {
    let request = request
        .into_builder()
        .response_format(ResponseFormat::JsonSchema {
            name: schema_name.to_string(),
            schema,
        })
        .build()
        .map_err(|source| {
            Error::new(
                ErrorKind::InvalidRequest,
                "structured output request is invalid",
            )
            .with_source(source)
        })?;
    let response = client.complete_with_context(request, context).await?;
    let object = parse_object(&response)?;
    Ok(StructuredCompletion { response, object })
}

/// Parses a response's JSON output: a `Json` part when the provider returned
/// one, else the concatenated text.
pub fn parse_object(response: &Response) -> Result<serde_json::Value, Error> {
    if let Some(value) = response.content.iter().find_map(|part| match part {
        fabro_types::ContentPart::Json { value } => Some(value.clone()),
        _ => None,
    }) {
        return Ok(value);
    }
    let text = response.text();
    serde_json::from_str(text.trim()).map_err(|source| {
        Error::new(
            ErrorKind::ResponseDecode,
            format!("the model did not return a JSON object: {source}"),
        )
        .with_provider(response.model.provider().clone())
        .with_source(source)
    })
}

#[cfg(test)]
mod tests {
    use fabro_types::{ContentPart, ModelId, ProviderId};
    use serde_json::json;

    use super::*;

    fn response(parts: Vec<ContentPart>) -> Response {
        Response::new(ProviderId::new("openai"), ModelId::new("gpt-5.4"), parts)
    }

    #[test]
    fn parses_text_or_json_parts() {
        let text = response(vec![ContentPart::Text {
            text: " {\"title\": \"x\"} ".to_string(),
        }]);
        assert_eq!(parse_object(&text).unwrap(), json!({"title": "x"}));
        let json = response(vec![ContentPart::Json {
            value: json!({"a": 1}),
        }]);
        assert_eq!(parse_object(&json).unwrap(), json!({"a": 1}));
        let prose = response(vec![ContentPart::Text {
            text: "sorry".to_string(),
        }]);
        assert_eq!(
            parse_object(&prose).unwrap_err().kind(),
            ErrorKind::ResponseDecode
        );
    }
}
