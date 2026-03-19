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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Resolves notifications for a given webhook URL. Trait-based for mockability.
pub trait NotifierFactory: Send + Sync {
    fn notify(
        &self,
        url: &str,
        task_name: &str,
        exit_code: i32,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// HTTP webhook notifier factory. Reuses a single `reqwest::Client` (connection pool).
#[cfg(feature = "webhooks")]
pub struct HttpNotifierFactory {
    client: reqwest::Client,
}

#[cfg(feature = "webhooks")]
impl HttpNotifierFactory {
    #[must_use]
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }
}

#[cfg(feature = "webhooks")]
impl Default for HttpNotifierFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "webhooks")]
impl NotifierFactory for HttpNotifierFactory {
    async fn notify(&self, url: &str, task_name: &str, exit_code: i32) {
        let body = serde_json::json!({
            "text": format!("teiki task `{task_name}` failed (exit {exit_code})")
        });
        let _ = self.client.post(url).json(&body).send().await;
    }
}

/// No-op factory — no notifications. Default for most usage.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopNotifierFactory;

impl NotifierFactory for NoopNotifierFactory {
    async fn notify(&self, _url: &str, _task_name: &str, _exit_code: i32) {}
}

// ── Command builder (pure function, no side effects) ───────────

/// Build a `tokio::process::Command` from an `ExecSpec`.
///
/// `current_path`: the current PATH value. Pass `None` to read from
/// the process environment (production), or `Some("...")` for testing.
#[must_use]
pub fn build_command(spec: &ExecSpec, current_path: Option<&str>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(&spec.command);
    cmd.args(&spec.args);

    for (k, v) in &spec.env {
        cmd.env(k, v);
    }

    if !spec.extra_path.is_empty() {
        let base = current_path
            .map(String::from)
            .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
        let extended = format!("{}:{base}", spec.extra_path.join(":"));
        cmd.env("PATH", extended);
    }

    if let Some(dir) = &spec.working_directory {
        cmd.current_dir(dir);
    }

    cmd
}

// ── Production runner (no generics — pure TaskRunner) ──────────

/// Subprocess runner. Implements `TaskRunner` only — notification
/// is handled by `App`, not the runner (separation of concerns).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessRunner;

impl TaskRunner for ProcessRunner {
    async fn run(&self, name: &str, spec: &ExecSpec) -> Result<TaskOutcome, TeikiError> {
        info!(task = name, command = %spec.command, "starting");
        let start = Instant::now();
        let mut cmd = build_command(spec, None);

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

// ── Mock types (public — usable by library consumers) ──────────

/// Recording notifier factory for test assertions.
#[derive(Clone, Default)]
pub struct RecordingNotifierFactory {
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<(String, String, i32)>>>,
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
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    pub exit_code: i32,
}

impl MockRunner {
    #[must_use]
    pub fn succeeding() -> Self {
        Self { calls: std::sync::Arc::new(std::sync::Mutex::new(vec![])), exit_code: 0 }
    }

    #[must_use]
    pub fn failing(code: i32) -> Self {
        Self { calls: std::sync::Arc::new(std::sync::Mutex::new(vec![])), exit_code: code }
    }
}

impl TaskRunner for MockRunner {
    async fn run(&self, name: &str, _spec: &ExecSpec) -> Result<TaskOutcome, TeikiError> {
        self.calls.lock().unwrap().push(name.to_string());
        if self.exit_code == 0 {
            Ok(TaskOutcome::success(name, std::time::Duration::from_millis(1)))
        } else {
            Ok(TaskOutcome::failure(name, self.exit_code, std::time::Duration::from_millis(1)))
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::sample_task;

    fn echo_spec() -> ExecSpec {
        ExecSpec::from(&sample_task("echo"))
    }

    // ── ExecSpec ────────────────────────────────────────────────

    #[test]
    fn exec_spec_from_task_config() {
        let task = sample_task("echo");
        let spec = ExecSpec::from(&task);
        assert_eq!(spec.command, "echo");
        assert_eq!(spec.timeout_secs, 30);
    }

    #[test]
    fn exec_spec_preserves_all_fields() {
        let mut task = sample_task("env");
        task.env.insert("KEY".into(), "VAL".into());
        task.extra_path = vec!["/opt/bin".into()];
        task.working_directory = Some(PathBuf::from("/tmp"));
        let spec = ExecSpec::from(&task);
        assert_eq!(spec.env["KEY"], "VAL");
        assert_eq!(spec.extra_path, vec!["/opt/bin"]);
        assert_eq!(spec.working_directory, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn exec_spec_eq() {
        let a = echo_spec();
        let b = echo_spec();
        assert_eq!(a, b);
    }

    #[test]
    fn exec_spec_ne_on_command() {
        let a = ExecSpec::from(&sample_task("echo"));
        let b = ExecSpec::from(&sample_task("cat"));
        assert_ne!(a, b);
    }

    // ── build_command (pure function) ───────────────────────────

    #[test]
    fn build_command_sets_program() {
        let cmd = build_command(&echo_spec(), Some(""));
        assert_eq!(cmd.as_std().get_program(), "echo");
    }

    #[test]
    fn build_command_with_args() {
        let mut spec = echo_spec();
        spec.args = vec!["hello".into(), "world".into()];
        let cmd = build_command(&spec, Some(""));
        let args: Vec<_> = cmd.as_std().get_args().collect();
        assert_eq!(args, &["hello", "world"]);
    }

    #[test]
    fn build_command_with_working_dir() {
        let mut spec = echo_spec();
        spec.working_directory = Some(PathBuf::from("/tmp"));
        let cmd = build_command(&spec, Some(""));
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp")));
    }

    #[test]
    fn build_command_prepends_extra_path() {
        let mut spec = echo_spec();
        spec.extra_path = vec!["/opt/a".into(), "/opt/b".into()];
        let cmd = build_command(&spec, Some("/usr/bin"));
        let envs: BTreeMap<_, _> = cmd.as_std().get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
            .collect();
        assert_eq!(envs["PATH"], "/opt/a:/opt/b:/usr/bin");
    }

    #[test]
    fn build_command_sets_env_vars() {
        let mut spec = echo_spec();
        spec.env.insert("FOO".into(), "bar".into());
        let cmd = build_command(&spec, Some(""));
        let envs: BTreeMap<_, _> = cmd.as_std().get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
            .collect();
        assert_eq!(envs["FOO"], "bar");
    }

    #[test]
    fn build_command_no_extra_path_no_path_env() {
        let spec = echo_spec(); // no extra_path
        let cmd = build_command(&spec, Some("/usr/bin"));
        let has_path = cmd.as_std().get_envs()
            .any(|(k, _)| k == "PATH");
        assert!(!has_path, "should not set PATH when extra_path is empty");
    }

    // ── ProcessRunner ──────────────────────────────────────────

    #[tokio::test]
    async fn process_runner_echo() {
        let outcome = ProcessRunner.run("t", &echo_spec()).await.unwrap();
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_failure() {
        let spec = ExecSpec::from(&sample_task("false"));
        let outcome = ProcessRunner.run("t", &spec).await.unwrap();
        assert!(!outcome.is_success());
    }

    #[tokio::test]
    async fn process_runner_timeout() {
        let mut spec = ExecSpec::from(&sample_task("sleep"));
        spec.args = vec!["10".into()];
        spec.timeout_secs = 1;
        let outcome = ProcessRunner.run("t", &spec).await.unwrap();
        assert_eq!(outcome.exit_code, -1);
    }

    #[tokio::test]
    async fn process_runner_spawn_error() {
        let spec = ExecSpec::from(&sample_task("/nonexistent/binary/xyz"));
        let result = ProcessRunner.run("t", &spec).await;
        assert!(matches!(result, Err(TeikiError::Spawn { .. })));
    }

    // ── MockRunner ─────────────────────────────────────────────

    #[tokio::test]
    async fn mock_runner_records() {
        let r = MockRunner::succeeding();
        r.run("a", &echo_spec()).await.unwrap();
        r.run("b", &echo_spec()).await.unwrap();
        assert_eq!(*r.calls.lock().unwrap(), vec!["a", "b"]);
    }

    #[tokio::test]
    async fn mock_runner_failing() {
        let r = MockRunner::failing(42);
        let o = r.run("x", &echo_spec()).await.unwrap();
        assert_eq!(o.exit_code, 42);
    }

    // ── RecordingNotifierFactory ────────────────────────────────

    #[tokio::test]
    async fn recording_notifier_captures() {
        let n = RecordingNotifierFactory::default();
        n.notify("http://a", "task-1", 1).await;
        n.notify("http://b", "task-2", 2).await;
        let calls = n.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "http://a");
        assert_eq!(calls[1].2, 2);
    }

    #[tokio::test]
    async fn noop_notifier() {
        NoopNotifierFactory.notify("http://x", "t", 1).await;
    }
}
