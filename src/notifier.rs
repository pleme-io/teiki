/// Trait for task failure notifications. Abstracted for mockability.
pub trait Notifier: Send + Sync {
    /// Notify about a task failure. Implementations should be best-effort.
    fn notify(
        &self,
        task_name: &str,
        exit_code: i32,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Webhook-based notifier (Discord, Slack, etc.).
pub struct WebhookNotifier {
    client: reqwest::Client,
    url: String,
}

impl WebhookNotifier {
    #[must_use]
    pub fn new(url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
        }
    }
}

impl Notifier for WebhookNotifier {
    async fn notify(&self, task_name: &str, exit_code: i32) {
        let body = serde_json::json!({
            "text": format!("teiki task `{task_name}` failed (exit {exit_code})")
        });
        let _ = self.client.post(&self.url).json(&body).send().await;
    }
}

/// No-op notifier for tasks without notification or for testing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    async fn notify(&self, _task_name: &str, _exit_code: i32) {}
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Recording notifier for test assertions.
    #[derive(Clone, Default)]
    pub struct RecordingNotifier {
        pub calls: Arc<Mutex<Vec<(String, i32)>>>,
    }

    impl Notifier for RecordingNotifier {
        async fn notify(&self, task_name: &str, exit_code: i32) {
            self.calls.lock().unwrap().push((task_name.to_string(), exit_code));
        }
    }

    #[tokio::test]
    async fn noop_does_nothing() {
        let n = NoopNotifier;
        n.notify("test", 1).await;
        // No panic = success
    }

    #[tokio::test]
    async fn recording_captures_calls() {
        let n = RecordingNotifier::default();
        n.notify("task-a", 1).await;
        n.notify("task-b", 127).await;
        let calls = n.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ("task-a".to_string(), 1));
        assert_eq!(calls[1], ("task-b".to_string(), 127));
    }
}
