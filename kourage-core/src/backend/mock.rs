//! A scripted backend for tests.

use std::sync::{Mutex, PoisonError};

use super::{Backend, BackendError, GenerateRequest};

/// Replies from a script instead of a model, and records every request it receives.
#[derive(Debug)]
pub struct MockBackend {
    outcome: Outcome,
    models: Models,
    requests: Mutex<Vec<GenerateRequest>>,
}

#[derive(Debug)]
enum Outcome {
    Replies(Vec<String>),
    Error(BackendError),
}

/// What `installed_models` answers.
#[derive(Debug)]
enum Models {
    Unsupported,
    Listed(Vec<String>),
    Unreachable,
}

impl MockBackend {
    /// Answer with `replies` in order, starting over after the last one.
    pub fn replying(replies: &[&str]) -> Self {
        Self::with(Outcome::Replies(replies.iter().map(|r| r.to_string()).collect()))
    }

    /// Fail every request with `error`.
    pub fn failing(error: BackendError) -> Self {
        Self::with(Outcome::Error(error))
    }

    /// Report `models` as installed (and reply with nothing to `generate`).
    pub fn with_models(models: &[&str]) -> Self {
        let mut mock = Self::replying(&[]);
        mock.models = Models::Listed(models.iter().map(|m| m.to_string()).collect());
        mock
    }

    /// Fail `installed_models` as if the server could not be reached.
    pub fn unreachable() -> Self {
        let mut mock = Self::replying(&[]);
        mock.models = Models::Unreachable;
        mock
    }

    fn with(outcome: Outcome) -> Self {
        Self { outcome, models: Models::Unsupported, requests: Mutex::new(Vec::new()) }
    }

    /// Every request received so far, oldest first.
    pub fn requests(&self) -> Vec<GenerateRequest> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Backend for MockBackend {
    fn generate(&self, request: &GenerateRequest) -> Result<String, BackendError> {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        requests.push(request.clone());
        match &self.outcome {
            Outcome::Replies(replies) if replies.is_empty() => Ok(String::new()),
            Outcome::Replies(replies) => Ok(replies[(requests.len() - 1) % replies.len()].clone()),
            Outcome::Error(error) => Err(error.clone()),
        }
    }

    fn installed_models(&self) -> Result<Option<Vec<String>>, BackendError> {
        match &self.models {
            Models::Unsupported => Ok(None),
            Models::Listed(models) => Ok(Some(models.clone())),
            Models::Unreachable => Err(BackendError::Connect {
                url: "http://localhost:11434/api/generate".into(),
                model: self.model().into(),
                detail: "connection refused".into(),
            }),
        }
    }

    fn model(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::GenerateOptions;

    fn request(prompt: &str) -> GenerateRequest {
        GenerateRequest { prompt: prompt.into(), options: GenerateOptions::for_command(0.0) }
    }

    #[test]
    fn replying_returns_replies_in_order_then_cycles() {
        let mock = MockBackend::replying(&["ls", "pwd"]);
        let replies: Vec<_> = (0..3).map(|_| mock.generate(&request("p")).unwrap()).collect();
        assert_eq!(replies, ["ls", "pwd", "ls"]);
    }

    #[test]
    fn replying_with_nothing_scripted_returns_an_empty_reply() {
        assert_eq!(MockBackend::replying(&[]).generate(&request("p")).unwrap(), "");
    }

    #[test]
    fn failing_returns_the_error_every_time() {
        let error = BackendError::Timeout { url: "http://x".into(), timeout_secs: 3 };
        let mock = MockBackend::failing(error.clone());
        assert_eq!(mock.generate(&request("p")), Err(error.clone()));
        assert_eq!(mock.generate(&request("p")), Err(error));
    }

    #[test]
    fn requests_records_what_was_sent() {
        let mock = MockBackend::failing(BackendError::Malformed {
            url: "http://x".into(),
            detail: "d".into(),
        });
        let _ = mock.generate(&request("first"));
        let _ = mock.generate(&request("second"));
        assert_eq!(mock.requests(), vec![request("first"), request("second")]);
    }

    #[test]
    fn warm_up_is_a_no_op() {
        let mock = MockBackend::replying(&["ls"]);
        assert_eq!(mock.warm_up("prefix"), Ok(()));
        assert!(mock.requests().is_empty());
        assert_eq!(mock.model(), "mock");
    }
}
