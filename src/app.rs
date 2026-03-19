use crate::config::{Config, ConfigSource, TaskConfig};
use crate::error::TeikiError;
use crate::executor::{ExecSpec, NotifierFactory, TaskRunner};
use crate::outcome::TaskOutcome;
use crate::platform::PlatformDetector;
use std::process::ExitCode;
use tracing::{error, info};

/// Core application logic, parameterized over all dependencies.
///
/// Generic over `ConfigSource`, `TaskRunner`, `NotifierFactory`, and
/// `PlatformDetector` for full testability — no filesystem, process,
/// network, or platform coupling in tests.
pub struct App<C, R, N, P> {
    config_source: C,
    runner: R,
    notifier: N,
    platform: P,
}

impl<C: ConfigSource, R: TaskRunner, N: NotifierFactory, P: PlatformDetector> App<C, R, N, P> {
    pub fn new(config_source: C, runner: R, notifier: N, platform: P) -> Self {
        Self { config_source, runner, notifier, platform }
    }

    fn load_and_resolve(&self) -> Result<(Config, crate::platform::Platform), TeikiError> {
        let cfg = self.config_source.load()?;
        let platform = self.platform.current();
        Ok((cfg, platform))
    }

    pub async fn run_task(&self, name: &str) -> Result<TaskOutcome, TeikiError> {
        let (cfg, platform) = self.load_and_resolve()?;
        let task = cfg.tasks.get(name)
            .filter(|t| t.enabled && t.platforms.contains(&platform))
            .ok_or_else(|| TeikiError::TaskNotFound {
                name: name.into(),
                platform: platform.to_string(),
            })?;
        self.execute_with_notify(name, task).await
    }

    pub async fn run_all(&self) -> Result<Vec<TaskOutcome>, TeikiError> {
        let (cfg, platform) = self.load_and_resolve()?;
        let enabled: Vec<_> = cfg.tasks.iter()
            .filter(|(_, t)| t.enabled && t.platforms.contains(&platform))
            .collect();

        if enabled.is_empty() {
            info!("no tasks enabled for {platform}");
            return Ok(vec![]);
        }

        let mut outcomes = Vec::with_capacity(enabled.len());
        for (name, task) in &enabled {
            let outcome = self.execute_with_notify(name, task).await?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    pub fn list<'a>(
        &self,
        filter_platform: bool,
        tag: Option<&str>,
    ) -> Result<Vec<TaskListEntry>, TeikiError> {
        let (cfg, platform) = self.load_and_resolve()?;
        let mut entries = Vec::new();

        for (name, task) in &cfg.tasks {
            if filter_platform && !(task.enabled && task.platforms.contains(&platform)) {
                continue;
            }
            if let Some(t) = tag {
                if !task.tags.iter().any(|tag| tag == t) {
                    continue;
                }
            }
            entries.push(TaskListEntry {
                name: name.clone(),
                description: task.description.clone(),
                enabled: task.enabled,
                schedule: task.schedule.to_string(),
                platforms: task.platforms.iter().map(ToString::to_string).collect(),
                tags: task.tags.clone(),
            });
        }
        Ok(entries)
    }

    pub fn validate(&self) -> Result<ValidationResult, TeikiError> {
        let (cfg, platform) = self.load_and_resolve()?;
        let issues = cfg.validate();
        let enabled = cfg.tasks.values().filter(|t| t.enabled).count();
        let platform_count = cfg.tasks.values()
            .filter(|t| t.enabled && t.platforms.contains(&platform))
            .count();
        Ok(ValidationResult {
            total: cfg.tasks.len(),
            enabled,
            current_platform: platform_count,
            issues,
        })
    }

    pub fn show(&self) -> Result<String, TeikiError> {
        let (cfg, _) = self.load_and_resolve()?;
        serde_yaml::to_string(&cfg).map_err(|e| TeikiError::ConfigParse(e.to_string()))
    }

    async fn execute_with_notify(
        &self,
        name: &str,
        task: &TaskConfig,
    ) -> Result<TaskOutcome, TeikiError> {
        let spec = ExecSpec::from(task);
        let outcome = self.runner.run(name, &spec).await?;
        if !outcome.is_success() {
            if let Some(url) = &task.notify_on_failure {
                self.notifier.notify(url, name, outcome.exit_code).await;
            }
        }
        Ok(outcome)
    }
}

// ── Return types ───────────────────────────────────────────────

/// Entry in task list output — no references, owned for easy serialization.
#[derive(Debug, Clone)]
pub struct TaskListEntry {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub schedule: String,
    pub platforms: Vec<String>,
    pub tags: Vec<String>,
}

impl std::fmt::Display for TaskListEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = if self.enabled { "enabled" } else { "disabled" };
        let platforms = self.platforms.join(", ");
        let tags = if self.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", self.tags.join(", "))
        };
        write!(f, "{:24} {:8} {:30} ({platforms}){tags}", self.name, status, self.schedule)
    }
}

/// Result of config validation.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub total: usize,
    pub enabled: usize,
    pub current_platform: usize,
    pub issues: Vec<String>,
}

impl ValidationResult {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }
}

// ── Convenience: App → ExitCode adapter for main.rs ────────────

impl<C: ConfigSource, R: TaskRunner, N: NotifierFactory, P: PlatformDetector> App<C, R, N, P> {
    pub async fn run_task_exit(&self, name: &str) -> anyhow::Result<ExitCode> {
        let outcome = self.run_task(name).await?;
        Ok(if outcome.is_success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
    }

    pub async fn run_all_exit(&self) -> anyhow::Result<ExitCode> {
        let outcomes = self.run_all().await?;
        let any_failed = outcomes.iter().any(|o| !o.is_success());
        Ok(if any_failed { ExitCode::FAILURE } else { ExitCode::SUCCESS })
    }

    pub fn list_exit(&self, filter: bool, tag: Option<&str>) -> anyhow::Result<ExitCode> {
        let entries = self.list(filter, tag)?;
        for entry in &entries {
            println!("{entry}");
        }
        Ok(ExitCode::SUCCESS)
    }

    pub fn validate_exit(&self) -> anyhow::Result<ExitCode> {
        let result = self.validate()?;
        if result.is_valid() {
            info!(
                total = result.total,
                enabled = result.enabled,
                current_platform = result.current_platform,
                "configuration valid"
            );
            Ok(ExitCode::SUCCESS)
        } else {
            for issue in &result.issues {
                error!("{issue}");
            }
            Ok(ExitCode::FAILURE)
        }
    }

    pub fn show_exit(&self) -> anyhow::Result<ExitCode> {
        let yaml = self.show()?;
        print!("{yaml}");
        Ok(ExitCode::SUCCESS)
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, tests::{StaticSource, sample_task}};
    use crate::executor::{NoopNotifierFactory, tests::MockRunner};
    use crate::platform::{Platform, tests::MockPlatform};
    use std::collections::BTreeMap;

    fn test_config() -> Config {
        let mut tasks = BTreeMap::new();
        tasks.insert("task-a".into(), sample_task("echo"));
        tasks.insert("task-b".into(), crate::config::TaskConfig {
            platforms: vec![Platform::Darwin],
            tags: vec!["cleanup".into()],
            ..sample_task("true")
        });
        tasks.insert("task-c".into(), crate::config::TaskConfig {
            enabled: false,
            ..sample_task("false")
        });
        Config { tasks, defaults: Default::default() }
    }

    fn test_app(platform: Platform) -> App<StaticSource, MockRunner, NoopNotifierFactory, MockPlatform> {
        App::new(
            StaticSource(test_config()),
            MockRunner::succeeding(),
            NoopNotifierFactory,
            MockPlatform(platform),
        )
    }

    // ── run_task ────────────────────────────────────────────────

    #[tokio::test]
    async fn run_task_success() {
        let app = test_app(Platform::Darwin);
        let outcome = app.run_task("task-a").await.unwrap();
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn run_task_not_found() {
        let app = test_app(Platform::Darwin);
        let result = app.run_task("nonexistent").await;
        assert!(matches!(result, Err(TeikiError::TaskNotFound { .. })));
    }

    #[tokio::test]
    async fn run_task_wrong_platform() {
        let app = test_app(Platform::Linux);
        let result = app.run_task("task-b").await;
        assert!(matches!(result, Err(TeikiError::TaskNotFound { .. })));
    }

    #[tokio::test]
    async fn run_task_disabled() {
        let app = test_app(Platform::Darwin);
        let result = app.run_task("task-c").await;
        assert!(matches!(result, Err(TeikiError::TaskNotFound { .. })));
    }

    // ── run_all ─────────────────────────────────────────────────

    #[tokio::test]
    async fn run_all_executes_enabled() {
        let app = test_app(Platform::Darwin);
        let outcomes = app.run_all().await.unwrap();
        assert_eq!(outcomes.len(), 2);
    }

    #[tokio::test]
    async fn run_all_linux_excludes_darwin() {
        let app = test_app(Platform::Linux);
        let outcomes = app.run_all().await.unwrap();
        assert_eq!(outcomes.len(), 1);
    }

    #[tokio::test]
    async fn run_all_empty_config() {
        let cfg = Config { tasks: BTreeMap::new(), defaults: Default::default() };
        let app = App::new(
            StaticSource(cfg),
            MockRunner::succeeding(),
            NoopNotifierFactory,
            MockPlatform(Platform::Darwin),
        );
        let outcomes = app.run_all().await.unwrap();
        assert!(outcomes.is_empty());
    }

    // ── list ────────────────────────────────────────────────────

    #[test]
    fn list_all() {
        let app = test_app(Platform::Darwin);
        let entries = app.list(false, None).unwrap();
        assert_eq!(entries.len(), 3); // includes disabled
    }

    #[test]
    fn list_platform_filtered() {
        let app = test_app(Platform::Linux);
        let entries = app.list(true, None).unwrap();
        assert_eq!(entries.len(), 1); // only task-a
    }

    #[test]
    fn list_tag_filtered() {
        let app = test_app(Platform::Darwin);
        let entries = app.list(false, Some("cleanup")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "task-b");
    }

    #[test]
    fn list_tag_no_match() {
        let app = test_app(Platform::Darwin);
        let entries = app.list(false, Some("nonexistent")).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn list_entry_display() {
        let entry = TaskListEntry {
            name: "test".into(),
            description: "desc".into(),
            enabled: true,
            schedule: "every 1h".into(),
            platforms: vec!["darwin".into()],
            tags: vec!["cleanup".into()],
        };
        let s = entry.to_string();
        assert!(s.contains("test"));
        assert!(s.contains("enabled"));
        assert!(s.contains("[cleanup]"));
    }

    // ── validate ────────────────────────────────────────────────

    #[test]
    fn validate_clean() {
        let app = test_app(Platform::Darwin);
        let result = app.validate().unwrap();
        assert!(result.is_valid());
        assert_eq!(result.total, 3);
        assert_eq!(result.enabled, 2);
    }

    #[test]
    fn validate_bad() {
        let mut cfg = test_config();
        cfg.tasks.insert("empty".into(), sample_task(""));
        let app = App::new(
            StaticSource(cfg),
            MockRunner::succeeding(),
            NoopNotifierFactory,
            MockPlatform(Platform::Darwin),
        );
        let result = app.validate().unwrap();
        assert!(!result.is_valid());
        assert!(result.issues.iter().any(|i| i.contains("empty command")));
    }

    // ── show ────────────────────────────────────────────────────

    #[test]
    fn show_returns_yaml() {
        let app = test_app(Platform::Darwin);
        let yaml = app.show().unwrap();
        assert!(yaml.contains("task-a"));
    }

    // ── exit code adapters ──────────────────────────────────────

    #[tokio::test]
    async fn run_all_exit_success() {
        let app = test_app(Platform::Darwin);
        let exit = app.run_all_exit().await.unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[tokio::test]
    async fn run_all_exit_failure() {
        let app = App::new(
            StaticSource(test_config()),
            MockRunner::failing(1),
            NoopNotifierFactory,
            MockPlatform(Platform::Darwin),
        );
        let exit = app.run_all_exit().await.unwrap();
        assert_eq!(exit, ExitCode::FAILURE);
    }

    // ── error paths ─────────────────────────────────────────────

    #[tokio::test]
    async fn run_task_config_source_failure() {
        let app = App::new(
            crate::config::tests::FailingSource,
            MockRunner::succeeding(),
            NoopNotifierFactory,
            MockPlatform(Platform::Darwin),
        );
        let result = app.run_task("any").await;
        assert!(matches!(result, Err(TeikiError::ConfigNotFound(_))));
    }

    #[tokio::test]
    async fn run_all_config_source_failure() {
        let app = App::new(
            crate::config::tests::FailingSource,
            MockRunner::succeeding(),
            NoopNotifierFactory,
            MockPlatform(Platform::Darwin),
        );
        let result = app.run_all().await;
        assert!(matches!(result, Err(TeikiError::ConfigNotFound(_))));
    }

    #[test]
    fn validate_config_source_failure() {
        let app = App::new(
            crate::config::tests::FailingSource,
            MockRunner::succeeding(),
            NoopNotifierFactory,
            MockPlatform(Platform::Darwin),
        );
        let result = app.validate();
        assert!(matches!(result, Err(TeikiError::ConfigNotFound(_))));
    }

    // ── notification routing ────────────────────────────────────

    #[tokio::test]
    async fn run_task_notifies_on_failure_with_correct_url() {
        use crate::executor::tests::RecordingNotifierFactory;
        let factory = RecordingNotifierFactory::default();
        let mut cfg = test_config();
        cfg.tasks.get_mut("task-a").unwrap().notify_on_failure =
            Some("http://hooks.example.com/fail".into());
        let app = App::new(
            StaticSource(cfg),
            MockRunner::failing(42),
            factory.clone(),
            MockPlatform(Platform::Darwin),
        );
        app.run_task("task-a").await.unwrap();
        let calls = factory.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "http://hooks.example.com/fail");
        assert_eq!(calls[0].1, "task-a");
        assert_eq!(calls[0].2, 42);
    }

    #[tokio::test]
    async fn run_all_notifies_per_task_url() {
        use crate::executor::tests::RecordingNotifierFactory;
        let factory = RecordingNotifierFactory::default();
        let mut cfg = test_config();
        cfg.tasks.get_mut("task-a").unwrap().notify_on_failure =
            Some("http://url-a".into());
        cfg.tasks.get_mut("task-b").unwrap().notify_on_failure =
            Some("http://url-b".into());
        let app = App::new(
            StaticSource(cfg),
            MockRunner::failing(1),
            factory.clone(),
            MockPlatform(Platform::Darwin),
        );
        app.run_all().await.unwrap();
        let calls = factory.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let urls: Vec<_> = calls.iter().map(|c| c.0.as_str()).collect();
        assert!(urls.contains(&"http://url-a"));
        assert!(urls.contains(&"http://url-b"));
    }
}
