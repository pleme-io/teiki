use crate::config::TaskConfig;
use crate::notifier::Notifier;
use crate::outcome::TaskOutcome;
use std::time::Instant;
use tracing::{error, info};

/// Trait for executing tasks. Abstracted for mockability in tests.
pub trait TaskRunner: Send + Sync {
    fn run(
        &self,
        name: &str,
        task: &TaskConfig,
    ) -> impl std::future::Future<Output = anyhow::Result<TaskOutcome>> + Send;
}

/// Executes tasks as subprocesses via `tokio::process::Command`.
pub struct ProcessRunner<N: Notifier> {
    notifier: N,
}

impl<N: Notifier> ProcessRunner<N> {
    pub fn new(notifier: N) -> Self {
        Self { notifier }
    }

    fn build_command(task: &TaskConfig) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&task.command);
        cmd.args(&task.args);

        for (k, v) in &task.env {
            cmd.env(k, v);
        }

        if !task.extra_path.is_empty() {
            let current = std::env::var("PATH").unwrap_or_default();
            let extended = format!("{}:{current}", task.extra_path.join(":"));
            cmd.env("PATH", extended);
        }

        if let Some(dir) = &task.working_directory {
            cmd.current_dir(dir);
        }

        cmd
    }
}

impl<N: Notifier> TaskRunner for ProcessRunner<N> {
    async fn run(&self, name: &str, task: &TaskConfig) -> anyhow::Result<TaskOutcome> {
        info!(task = name, command = %task.command, "starting");
        let start = Instant::now();
        let mut cmd = Self::build_command(task);

        let status = if task.timeout_secs > 0 {
            let timeout = std::time::Duration::from_secs(task.timeout_secs);
            match tokio::time::timeout(timeout, cmd.status()).await {
                Ok(result) => result?,
                Err(_) => {
                    let elapsed = start.elapsed();
                    error!(task = name, timeout_secs = task.timeout_secs, "timed out");
                    self.notifier.notify(name, -1).await;
                    return Ok(TaskOutcome::failure(name, -1, elapsed));
                }
            }
        } else {
            cmd.status().await?
        };

        let elapsed = start.elapsed();
        let code = status.code().unwrap_or(-1);

        if status.success() {
            info!(task = name, elapsed_ms = elapsed.as_millis() as u64, "completed");
            Ok(TaskOutcome::success(name, elapsed))
        } else {
            error!(task = name, exit_code = code, elapsed_ms = elapsed.as_millis() as u64, "failed");
            if task.notify_on_failure.is_some() {
                self.notifier.notify(name, code).await;
            }
            Ok(TaskOutcome::failure(name, code, elapsed))
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::config::tests::sample_task;
    use crate::notifier::NoopNotifier;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Mock runner that records calls and returns predetermined outcomes.
    pub struct MockRunner {
        pub calls: Arc<Mutex<Vec<String>>>,
        pub exit_code: i32,
    }

    impl MockRunner {
        pub fn succeeding() -> Self {
            Self { calls: Arc::new(Mutex::new(vec![])), exit_code: 0 }
        }

        pub fn failing(code: i32) -> Self {
            Self { calls: Arc::new(Mutex::new(vec![])), exit_code: code }
        }
    }

    impl TaskRunner for MockRunner {
        async fn run(&self, name: &str, _task: &TaskConfig) -> anyhow::Result<TaskOutcome> {
            self.calls.lock().unwrap().push(name.to_string());
            if self.exit_code == 0 {
                Ok(TaskOutcome::success(name, Duration::from_millis(1)))
            } else {
                Ok(TaskOutcome::failure(name, self.exit_code, Duration::from_millis(1)))
            }
        }
    }

    #[tokio::test]
    async fn process_runner_executes_echo() {
        let runner = ProcessRunner::new(NoopNotifier);
        let task = sample_task("echo");
        let outcome = runner.run("echo-test", &task).await.unwrap();
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_captures_failure() {
        let runner = ProcessRunner::new(NoopNotifier);
        let task = sample_task("false");
        let outcome = runner.run("fail-test", &task).await.unwrap();
        assert!(!outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_with_args() {
        let runner = ProcessRunner::new(NoopNotifier);
        let mut task = sample_task("echo");
        task.args = vec!["hello".into(), "world".into()];
        let outcome = runner.run("args-test", &task).await.unwrap();
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_with_env() {
        let runner = ProcessRunner::new(NoopNotifier);
        let mut task = sample_task("env");
        task.env = BTreeMap::from([("TEIKI_TEST_VAR".into(), "1".into())]);
        // env command just prints environment, should succeed
        let outcome = runner.run("env-test", &task).await.unwrap();
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_timeout() {
        let runner = ProcessRunner::new(NoopNotifier);
        let mut task = sample_task("sleep");
        task.args = vec!["10".into()];
        task.timeout_secs = 1;
        let outcome = runner.run("timeout-test", &task).await.unwrap();
        assert!(!outcome.is_success());
        assert_eq!(outcome.exit_code, -1);
    }

    #[tokio::test]
    async fn process_runner_notifies_on_failure() {
        use crate::notifier::tests::RecordingNotifier;
        let notifier = RecordingNotifier::default();
        let runner = ProcessRunner::new(notifier.clone());
        let mut task = sample_task("false");
        task.notify_on_failure = Some("http://example.com/hook".into());
        let outcome = runner.run("notify-test", &task).await.unwrap();
        assert!(!outcome.is_success());
        let calls = notifier.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "notify-test");
    }

    #[tokio::test]
    async fn process_runner_no_notify_without_url() {
        use crate::notifier::tests::RecordingNotifier;
        let notifier = RecordingNotifier::default();
        let runner = ProcessRunner::new(notifier.clone());
        let task = sample_task("false"); // no notify_on_failure
        runner.run("no-notify", &task).await.unwrap();
        let calls = notifier.calls.lock().unwrap();
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn mock_runner_records_calls() {
        let runner = MockRunner::succeeding();
        let task = sample_task("anything");
        runner.run("a", &task).await.unwrap();
        runner.run("b", &task).await.unwrap();
        let calls = runner.calls.lock().unwrap();
        assert_eq!(*calls, vec!["a", "b"]);
    }
}
