//! Explicitly configured translation service boundary.

use std::{env, error::Error as _, io, time::Duration};

const ENDPOINT_ENV: &str = "FLASH_SHOT_TRANSLATION_ENDPOINT";
const TOKEN_ENV: &str = "FLASH_SHOT_TRANSLATION_TOKEN";
const TARGET_LANGUAGE_ENV: &str = "FLASH_SHOT_TRANSLATION_TARGET";
const TRANSLATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationConfig {
    endpoint: String,
    token: Option<String>,
    target_language: String,
}

impl TranslationConfig {
    pub fn from_environment() -> io::Result<Option<Self>> {
        let Some(endpoint) = env::var(ENDPOINT_ENV)
            .ok()
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        validate_endpoint(&endpoint)?;
        Ok(Some(Self {
            endpoint,
            token: env::var(TOKEN_ENV).ok().filter(|value| !value.is_empty()),
            target_language: env::var(TARGET_LANGUAGE_ENV)
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "en".to_owned()),
        }))
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn target_language(&self) -> &str {
        &self.target_language
    }
}

/// Translates user-selected text only after an HTTPS endpoint is explicitly configured.
pub fn translate(config: &TranslationConfig, text: &str) -> io::Result<String> {
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    // Bound both connection setup and the complete request so an unavailable service
    // cannot leave the background recognition task blocked indefinitely.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(TRANSLATION_REQUEST_TIMEOUT)
        .timeout(TRANSLATION_REQUEST_TIMEOUT)
        .redirects(0)
        .build();
    let request = agent
        .post(config.endpoint())
        .set("content-type", "application/json")
        .set("accept", "application/json");
    let request = match config.token() {
        Some(token) => request.set("authorization", &format!("Bearer {token}")),
        None => request,
    };
    let response = request
        .send_json(serde_json::json!({
            "text": text,
            "target_language": config.target_language(),
        }))
        .map_err(translation_error)?;
    translation_from_response(response.into_json().map_err(translation_body_error)?)
}

fn validate_endpoint(endpoint: &str) -> io::Result<()> {
    if !endpoint.starts_with("https://") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "translation endpoint must use HTTPS",
        ));
    }
    Ok(())
}

fn translation_from_response(value: serde_json::Value) -> io::Result<String> {
    value
        .get("translation")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .filter(|translation| !translation.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "translation response does not contain a non-empty translation",
            )
        })
}

fn translation_error(error: ureq::Error) -> io::Error {
    let kind = match &error {
        ureq::Error::Transport(transport) => transport
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(|source| source.kind())
            .unwrap_or(io::ErrorKind::Other),
        ureq::Error::Status(_, _) => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("translation service request failed: {error}"))
}

fn translation_body_error(error: io::Error) -> io::Error {
    let kind = error.kind();
    io::Error::new(
        kind,
        format!("translation response could not be decoded: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{TRANSLATION_REQUEST_TIMEOUT, translation_from_response, validate_endpoint};

    #[test]
    fn translation_requests_have_a_bounded_timeout() {
        assert_eq!(
            TRANSLATION_REQUEST_TIMEOUT,
            std::time::Duration::from_secs(15)
        );
    }

    #[test]
    fn configuration_requires_an_https_endpoint() {
        assert!(validate_endpoint("https://translate.example/v1").is_ok());
        assert!(validate_endpoint("http://localhost:8080/translate").is_err());
    }

    #[test]
    fn translation_response_requires_a_non_empty_translation_field() {
        assert_eq!(
            translation_from_response(serde_json::json!({ "translation": "Hello" })).unwrap(),
            "Hello"
        );
        assert!(translation_from_response(serde_json::json!({})).is_err());
        assert!(translation_from_response(serde_json::json!({ "translation": "" })).is_err());
    }
}
