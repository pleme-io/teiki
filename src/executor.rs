use crate::config::TaskConfig;
use crate::error::TeikiError;
use crate::outcome::TaskOutcome;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{error, info};

// ── Interface-segregated execution spec ────────────────────────

/// Minimal spec for task execution — only what the runner needs.
/// Separates execution concerns from scheduling/platform/metadata.
#[derive(Debug, Clone)]
pub struct ExecSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub extra_path: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub timeout_secs: u64,
}

impl From<&TaskConfig> for ExecSpec {
    fn from(task: &TaskConfig) -> Self {
        Self {
            command: task.command.clone(),
            args: task.args.clone(),
            env: task.env.clone(),
            extra_path: task.extra_path.clone(),
            working_directory: task.working_directory.clone(),
            timeout_secs: task.timeout_secs,
        }
    }
}

// ── Core trait ─────────────────────────────────────────────────

/// Trait for task execution. Takes only what it needs (ISP).
pub trait TaskRunner: Send + Sync {
    fn run(
        &self,
        name: &str,
        spec: &ExecSpec,
    ) -> impl std::future::Future<Output = Result<TaskOutcome, TeikiError>> + Send;
}

// ── Notification resolution ────────────────────────────────────

/// Resolves a notifier for a given webhook URL. Trait-based for mockability.
pub trait NotifierFactory: Send + Sync {
    fn notify(
        &self,
        url: &str,
        task_name: &str,
        exit_code: i32,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// HTTP webhook notifier factory. Reuses a single `reqwest::Client` (connection pool).
pub struct HttpNotifierFactory {
    client: reqwest::Client,
}

impl HttpNotifierFactory {
    #[must_use]
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }
}

impl Default for HttpNotifierFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl NotifierFactory for HttpNotifierFactory {
    async fn notify(&self, url: &str, task_name: &str, exit_code: i32) {
        let body = serde_json::json!({
            "text": format!("teiki task `{task_name}` failed (exit {exit_code})")
        });
        let _ = self.client.post(url).json(&body).send().await;
    }
}

/// No-op factory for tasks without webhooks or for testing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopNotifierFactory;

impl NotifierFactory for NoopNotifierFactory {
    async fn notify(&self, _url: &str, _task_name: &str, _exit_code: i32) {}
}

// ── Command builder (public for testability) ───────────────────

/// Build a `tokio::process::Command` from an `ExecSpec`.
#[must_use]
pub fn build_command(spec: &ExecSpec) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(&spec.command);
    cmd.args(&spec.args);

    for (k, v) in &spec.env {
        cmd.env(k, v);
    }

    if !spec.extra_path.is_empty() {
        let current = std::env::var("PATH").unwrap_or_default();
        let extended = format!("{}:{current}", spec.extra_path.join(":"));
        cmd.env("PATH", extended);
    }

    if let Some(dir) = &spec.working_directory {
        cmd.current_dir(dir);
    }

    cmd
}

// ── Production runner ──────────────────────────────────────────

/// Subprocess runner with configurable notification factory.
pub struct ProcessRunner<N: NotifierFactory> {
    notifier: N,
}

impl<N: NotifierFactory> ProcessRunner<N> {
    pub fn new(notifier: N) -> Self {
        Self { notifier }
    }

    /// Run a task and optionally notify on failure.
    pub async fn run_with_notify(
        &self,
        name: &str,
        task: &TaskConfig,
    ) -> Result<TaskOutcome, TeikiError> {
        let spec = ExecSpec::from(task);
        let outcome = self.run(name, &spec).await?;
        if !outcome.is_success() {
            if let Some(url) = &task.notify_on_failure {
                self.notifier.notify(url, name, outcome.exit_code).await;
            }
        }
        Ok(outcome)
    }
}

impl<N: NotifierFactory> TaskRunner for ProcessRunner<N> {
    async fn run(&self, name: &str, spec: &ExecSpec) -> Result<TaskOutcome, TeikiError> {
        info!(task = name, command = %spec.command, "starting");
        let start = Instant::now();
        let mut cmd = build_command(spec);

        let status = if spec.timeout_secs > 0 {
            let timeout = std::time::Duration::from_secs(spec.timeout_secs);
            match tokio::time::timeout(timeout, cmd.status()).await {
                Ok(Ok(status)) => status,
                Ok(Err(e)) => return Err(TeikiError::Spawn { name: name.into(), source: e }),
                Err(_) => {
                    let elapsed = start.elapsed();
                    error!(task = name, timeout_secs = spec.timeout_secs, "timed out");
                    return Ok(TaskOutcome::failure(name, -1, elapsed));
                }
            }
        } else {
            cmd.status().await.map_err(|e| TeikiError::Spawn { name: name.into(), source: e })?
        };

        let elapsed = start.elapsed();
        let code = status.code().unwrap_or(-1);

        if status.success() {
            info!(task = name, elapsed_ms = elapsed.as_millis() as u64, "completed");
            Ok(TaskOutcome::success(name, elapsed))
        } else {
            error!(task = name, exit_code = code, elapsed_ms = elapsed.as_millis() as u64, "failed");
            Ok(TaskOutcome::failure(name, code, elapsed))
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::config::tests::sample_task;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Recording notifier factory for test assertions.
    #[derive(Clone, Default)]
    pub struct RecordingNotifierFactory {
        pub calls: Arc<Mutex<Vec<(String, String, i32)>>>,
    }

    impl NotifierFactory for RecordingNotifierFactory {
        async fn notify(&self, url: &str, task_name: &str, exit_code: i32) {
            self.calls.lock().unwrap().push((
                url.to_string(),
                task_name.to_string(),
                exit_code,
            ));
        }
    }

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
        async fn run(&self, name: &str, _spec: &ExecSpec) -> Result<TaskOutcome, TeikiError> {
            self.calls.lock().unwrap().push(name.to_string());
            if self.exit_code == 0 {
                Ok(TaskOutcome::success(name, Duration::from_millis(1)))
            } else {
                Ok(TaskOutcome::failure(name, self.exit_code, Duration::from_millis(1)))
            }
        }
    }

    fn echo_spec() -> ExecSpec {
        ExecSpec::from(&sample_task("echo"))
    }

    // ── ExecSpec tests ──────────────────────────────────────────

    #[test]
    fn exec_spec_from_task_config() {
        let task = sample_task("echo");
        let spec = ExecSpec::from(&task);
        assert_eq!(spec.command, "echo");
        assert_eq!(spec.timeout_secs, 30);
    }

    #[test]
    fn exec_spec_preserves_env() {
        let mut task = sample_task("env");
        task.env.insert("KEY".into(), "VAL".into());
        let spec = ExecSpec::from(&task);
        assert_eq!(spec.env["KEY"], "VAL");
    }

    // ── build_command tests ─────────────────────────────────────

    #[test]
    fn build_command_sets_program() {
        let spec = echo_spec();
        let cmd = build_command(&spec);
        let prog = cmd.as_std().get_program();
        assert_eq!(prog, "echo");
    }

    #[test]
    fn build_command_with_args() {
        let mut spec = echo_spec();
        spec.args = vec!["hello".into(), "world".into()];
        let cmd = build_command(&spec);
        let args: Vec<_> = cmd.as_std().get_args().collect();
        assert_eq!(args, &["hello", "world"]);
    }

    #[test]
    fn build_command_with_working_dir() {
        let mut spec = echo_spec();
        spec.working_directory = Some(PathBuf::from("/tmp"));
        let cmd = build_command(&spec);
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp")));
    }

    // ── ProcessRunner tests ──────────────────────────────────────

    #[tokio::test]
    async fn process_runner_executes_echo() {
        let runner = ProcessRunner::new(NoopNotifierFactory);
        let spec = echo_spec();
        let outcome = runner.run("echo-test", &spec).await.unwrap();
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_captures_failure() {
        let runner = ProcessRunner::new(NoopNotifierFactory);
        let spec = ExecSpec::from(&sample_task("false"));
        let outcome = runner.run("fail-test", &spec).await.unwrap();
        assert!(!outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_timeout() {
        let runner = ProcessRunner::new(NoopNotifierFactory);
        let mut spec = ExecSpec::from(&sample_task("sleep"));
        spec.args = vec!["10".into()];
        spec.timeout_secs = 1;
        let outcome = runner.run("timeout-test", &spec).await.unwrap();
        assert!(!outcome.is_success());
        assert_eq!(outcome.exit_code, -1);
    }

    #[tokio::test]
    async fn process_runner_spawn_error() {
        let runner = ProcessRunner::new(NoopNotifierFactory);
        let spec = ExecSpec::from(&sample_task("/nonexistent/binary/xyz"));
        let result = runner.run("bad-cmd", &spec).await;
        assert!(matches!(result, Err(TeikiError::Spawn { .. })));
    }

    // ── run_with_notify tests ───────────────────────────────────

    #[tokio::test]
    async fn run_with_notify_sends_on_failure() {
        let factory = RecordingNotifierFactory::default();
        let runner = ProcessRunner::new(factory.clone());
        let mut task = sample_task("false");
        task.notify_on_failure = Some("http://example.com/hook".into());
        runner.run_with_notify("notify-test", &task).await.unwrap();
        let calls = factory.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "http://example.com/hook");
        assert_eq!(calls[0].1, "notify-test");
    }

    #[tokio::test]
    async fn run_with_notify_silent_on_success() {
        let factory = RecordingNotifierFactory::default();
        let runner = ProcessRunner::new(factory.clone());
        let mut task = sample_task("true");
        task.notify_on_failure = Some("http://example.com/hook".into());
        runner.run_with_notify("ok-test", &task).await.unwrap();
        let calls = factory.calls.lock().unwrap();
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn run_with_notify_no_url_no_notify() {
        let factory = RecordingNotifierFactory::default();
        let runner = ProcessRunner::new(factory.clone());
        let task = sample_task("false");
        runner.run_with_notify("no-url", &task).await.unwrap();
        let calls = factory.calls.lock().unwrap();
        assert!(calls.is_empty());
    }

    // ── MockRunner tests ────────────────────────────────────────

    #[tokio::test]
    async fn mock_runner_records_calls() {
        let runner = MockRunner::succeeding();
        let spec = echo_spec();
        runner.run("a", &spec).await.unwrap();
        runner.run("b", &spec).await.unwrap();
        let calls = runner.calls.lock().unwrap();
        assert_eq!(*calls, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn mock_runner_failing_returns_code() {
        let runner = MockRunner::failing(42);
        let spec = echo_spec();
        let outcome = runner.run("x", &spec).await.unwrap();
        assert_eq!(outcome.exit_code, 42);
    }
}
