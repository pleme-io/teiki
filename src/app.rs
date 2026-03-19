use crate::config::ConfigSource;
use crate::executor::TaskRunner;
use crate::outcome::TaskOutcome;
use crate::platform::PlatformDetector;
use std::process::ExitCode;
use tracing::{error, info};

/// Core application logic, parameterized over its dependencies.
///
/// Generic over `ConfigSource`, `TaskRunner`, and `PlatformDetector` for
/// full testability with mocks — no filesystem, process, or platform coupling.
pub struct App<C, R, P> {
    pub config_source: C,
    pub runner: R,
    pub platform: P,
}

impl<C: ConfigSource, R: TaskRunner, P: PlatformDetector> App<C, R, P> {
    pub fn new(config_source: C, runner: R, platform: P) -> Self {
        Self { config_source, runner, platform }
    }

    pub async fn run_task(&self, name: &str) -> anyhow::Result<ExitCode> {
        let cfg = self.config_source.load()?;
        let platform = self.platform.current();
        let tasks = cfg.tasks_for(platform);
        let task = tasks
            .get(name)
            .ok_or_else(|| anyhow::anyhow!(
                "task '{name}' not found or not enabled for {platform}"
            ))?;
        let outcome = self.runner.run(name, task).await?;
        Ok(outcome_to_exit(&outcome))
    }

    pub async fn run_all(&self) -> anyhow::Result<ExitCode> {
        let cfg = self.config_source.load()?;
        let platform = self.platform.current();
        let tasks = cfg.tasks_for(platform);
        if tasks.is_empty() {
            info!("no tasks enabled for {platform}");
            return Ok(ExitCode::SUCCESS);
        }
        let mut any_failed = false;
        for (name, task) in &tasks {
            let outcome = self.runner.run(name, task).await?;
            if !outcome.is_success() {
                any_failed = true;
            }
        }
        Ok(if any_failed { ExitCode::FAILURE } else { ExitCode::SUCCESS })
    }

    pub fn list(&self, filter_platform: bool, tag: Option<&str>) -> anyhow::Result<ExitCode> {
        let cfg = self.config_source.load()?;
        let platform = self.platform.current();
        let tasks: Vec<_> = if filter_platform {
            cfg.tasks_for(platform).into_iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
        } else {
            cfg.tasks.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };

        for (name, task) in &tasks {
            if let Some(t) = tag {
                if !task.tags.contains(&t.to_string()) {
                    continue;
                }
            }
            let status = if task.enabled { "enabled" } else { "disabled" };
            let schedule = task.schedule.to_string();
            let platforms: String = task.platforms.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let tags = if task.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", task.tags.join(", "))
            };
            println!("{name:24} {status:8} {schedule:30} ({platforms}){tags}");
        }
        Ok(ExitCode::SUCCESS)
    }

    pub fn validate(&self) -> anyhow::Result<ExitCode> {
        let cfg = self.config_source.load()?;
        let issues = cfg.validate();
        if issues.is_empty() {
            let platform = self.platform.current();
            info!(
                tasks = cfg.tasks.len(),
                enabled = cfg.tasks.values().filter(|t| t.enabled).count(),
                current_platform = cfg.tasks_for(platform).len(),
                "configuration valid"
            );
            Ok(ExitCode::SUCCESS)
        } else {
            for issue in &issues {
                error!("{issue}");
            }
            Ok(ExitCode::FAILURE)
        }
    }

    pub fn show(&self) -> anyhow::Result<ExitCode> {
        let cfg = self.config_source.load()?;
        let yaml = serde_yaml::to_string(&cfg)?;
        print!("{yaml}");
        Ok(ExitCode::SUCCESS)
    }
}

fn outcome_to_exit(outcome: &TaskOutcome) -> ExitCode {
    if outcome.is_success() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, tests::{StaticSource, sample_task}};
    use crate::executor::tests::MockRunner;
    use crate::platform::{Platform, tests::MockPlatform};
    use std::collections::BTreeMap;

    fn test_config() -> Config {
        let mut tasks = BTreeMap::new();
        tasks.insert("task-a".into(), sample_task("echo"));
        tasks.insert("task-b".into(), crate::config::TaskConfig {
            platforms: vec![Platform::Darwin],
            ..sample_task("true")
        });
        tasks.insert("task-c".into(), crate::config::TaskConfig {
            enabled: false,
            ..sample_task("false")
        });
        Config { tasks, defaults: Default::default() }
    }

    fn test_app(platform: Platform) -> App<StaticSource, MockRunner, MockPlatform> {
        App::new(
            StaticSource(test_config()),
            MockRunner::succeeding(),
            MockPlatform(platform),
        )
    }

    #[tokio::test]
    async fn run_task_success() {
        let app = test_app(Platform::Darwin);
        let exit = app.run_task("task-a").await.unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[tokio::test]
    async fn run_task_not_found() {
        let app = test_app(Platform::Darwin);
        let result = app.run_task("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_task_wrong_platform() {
        let app = test_app(Platform::Linux);
        // task-b is darwin-only
        let result = app.run_task("task-b").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_task_disabled() {
        let app = test_app(Platform::Darwin);
        let result = app.run_task("task-c").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_all_executes_enabled_only() {
        let app = test_app(Platform::Darwin);
        let exit = app.run_all().await.unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let calls = app.runner.calls.lock().unwrap();
        // task-a (both platforms) + task-b (darwin) = 2 tasks
        assert_eq!(calls.len(), 2);
        assert!(calls.contains(&"task-a".to_string()));
        assert!(calls.contains(&"task-b".to_string()));
    }

    #[tokio::test]
    async fn run_all_linux_excludes_darwin_only() {
        let app = test_app(Platform::Linux);
        app.run_all().await.unwrap();
        let calls = app.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], "task-a");
    }

    #[tokio::test]
    async fn run_all_reports_failure() {
        let app = App::new(
            StaticSource(test_config()),
            MockRunner::failing(1),
            MockPlatform(Platform::Darwin),
        );
        let exit = app.run_all().await.unwrap();
        assert_eq!(exit, ExitCode::FAILURE);
    }

    #[test]
    fn validate_clean_config() {
        let app = test_app(Platform::Darwin);
        let exit = app.validate().unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn validate_bad_config() {
        let mut cfg = test_config();
        cfg.tasks.insert("empty".into(), sample_task(""));
        let app = App::new(
            StaticSource(cfg),
            MockRunner::succeeding(),
            MockPlatform(Platform::Darwin),
        );
        let exit = app.validate().unwrap();
        assert_eq!(exit, ExitCode::FAILURE);
    }

    #[test]
    fn list_does_not_panic() {
        let app = test_app(Platform::Darwin);
        let exit = app.list(true, None).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn show_produces_yaml() {
        let app = test_app(Platform::Darwin);
        let exit = app.show().unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }
}
