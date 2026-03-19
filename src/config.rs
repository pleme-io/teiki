use crate::platform::Platform;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::TeikiError;

/// Trait for loading configuration. Abstracted for testing.
pub trait ConfigSource: Send + Sync {
    /// Load the full configuration.
    ///
    /// # Errors
    ///
    /// Returns `TeikiError::ConfigNotFound` or `TeikiError::ConfigParse`.
    fn load(&self) -> Result<Config, TeikiError>;
}

/// Loads config via shikumi discovery (production default).
pub struct ShikumiSource {
    path_override: Option<PathBuf>,
}

impl ShikumiSource {
    #[must_use]
    pub fn new() -> Self {
        Self { path_override: None }
    }

    #[must_use]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path_override: Some(path) }
    }
}

impl Default for ShikumiSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigSource for ShikumiSource {
    fn load(&self) -> Result<Config, TeikiError> {
        let path = match &self.path_override {
            Some(p) => p.clone(),
            None => shikumi::ConfigDiscovery::new("teiki")
                .env_override("TEIKI_CONFIG")
                .discover()
                .map_err(|e| TeikiError::ConfigNotFound(e.to_string()))?,
        };
        let store = shikumi::ConfigStore::<Config>::load(&path, "TEIKI_")
            .map_err(|e| TeikiError::ConfigParse(format!("{}: {e}", path.display())))?;
        Ok((*store.get().as_ref()).clone())
    }
}

// ── Config types ───────────────────────────────────────────────

/// Root configuration loaded from `~/.config/teiki/teiki.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub tasks: BTreeMap<String, TaskConfig>,

    #[serde(default)]
    pub defaults: TaskDefaults,
}

impl Config {
    /// Filter tasks enabled for the given platform.
    #[must_use]
    pub fn tasks_for(&self, platform: Platform) -> BTreeMap<&str, &TaskConfig> {
        self.tasks
            .iter()
            .filter(|(_, t)| t.enabled && t.platforms.contains(&platform))
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }

    /// Validate all task definitions. Returns a list of issues.
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for (name, task) in &self.tasks {
            if task.command.is_empty() {
                issues.push(format!("task '{name}': empty command"));
            }
            if task.enabled && task.platforms.is_empty() {
                issues.push(format!("task '{name}': enabled but no platforms"));
            }
            if task.timeout_secs == 0 && task.enabled {
                // Not an error, but noteworthy
            }
        }
        issues
    }
}

/// Per-task configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    pub description: String,

    #[serde(default = "default_true")]
    pub enabled: bool,

    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub env: BTreeMap<String, String>,

    #[serde(default)]
    pub extra_path: Vec<String>,

    pub schedule: Schedule,

    #[serde(default = "default_platforms")]
    pub platforms: Vec<Platform>,

    #[serde(default = "default_true")]
    pub low_priority: bool,

    #[serde(default)]
    pub working_directory: Option<PathBuf>,

    #[serde(default)]
    pub timeout_secs: u64,

    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub notify_on_failure: Option<String>,
}

/// Schedule specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    Interval { seconds: u64 },
    Calendar {
        #[serde(default)]
        month: Option<u32>,
        #[serde(default)]
        day: Option<u32>,
        #[serde(default)]
        weekday: Option<u32>,
        #[serde(default)]
        hour: Option<u32>,
        #[serde(default)]
        minute: Option<u32>,
    },
    Cron { expression: String },
}

impl std::fmt::Display for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Interval { seconds } => {
                if *seconds >= 3600 {
                    write!(f, "every {}h", seconds / 3600)
                } else if *seconds >= 60 {
                    write!(f, "every {}m", seconds / 60)
                } else {
                    write!(f, "every {seconds}s")
                }
            }
            Self::Calendar { weekday, hour, minute, month, day } => {
                static DAYS: [&str; 8] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                let mut parts = Vec::new();
                if let Some(w) = weekday {
                    parts.push(DAYS[(*w as usize) % 8].to_string());
                }
                if let Some(m) = month { parts.push(format!("month {m}")); }
                if let Some(d) = day { parts.push(format!("day {d}")); }
                let h = hour.unwrap_or(0);
                let m = minute.unwrap_or(0);
                parts.push(format!("{h:02}:{m:02}"));
                f.write_str(&parts.join(" "))
            }
            Self::Cron { expression } => f.write_str(expression),
        }
    }
}

/// Global defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDefaults {
    #[serde(default = "default_true")]
    pub low_priority: bool,

    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    #[serde(default = "default_platforms")]
    pub platforms: Vec<Platform>,

    #[serde(default)]
    pub notify_on_failure: Option<String>,
}

impl Default for TaskDefaults {
    fn default() -> Self {
        Self {
            low_priority: true,
            timeout_secs: 3600,
            platforms: default_platforms(),
            notify_on_failure: None,
        }
    }
}

fn default_true() -> bool { true }
fn default_timeout() -> u64 { 3600 }
fn default_platforms() -> Vec<Platform> { vec![Platform::Darwin, Platform::Linux] }

// ── Tests ──────────────────────────────────────────────────────

// ── Test/mock types (public for downstream crate tests) ────────

pub mod tests {
    use super::*;

    /// In-memory config source — inject any config without filesystem.
    pub struct StaticSource(pub Config);

    impl ConfigSource for StaticSource {
        fn load(&self) -> Result<Config, crate::error::TeikiError> {
            Ok(self.0.clone())
        }
    }

    /// Always-failing config source for error-path testing.
    pub struct FailingSource;

    impl ConfigSource for FailingSource {
        fn load(&self) -> Result<Config, crate::error::TeikiError> {
            Err(crate::error::TeikiError::ConfigNotFound("test failure".into()))
        }
    }

    /// Build a `TaskConfig` with sensible defaults for testing.
    #[must_use]
    pub fn sample_task(command: &str) -> TaskConfig {
        TaskConfig {
            description: "test task".into(),
            enabled: true,
            command: command.into(),
            args: vec![],
            env: BTreeMap::new(),
            extra_path: vec![],
            schedule: Schedule::Interval { seconds: 60 },
            platforms: vec![Platform::Darwin, Platform::Linux],
            low_priority: true,
            working_directory: None,
            timeout_secs: 30,
            tags: vec![],
            notify_on_failure: None,
        }
    }

    #[cfg(test)]
    fn sample_config() -> Config {
        let mut tasks = BTreeMap::new();
        tasks.insert("echo-test".into(), sample_task("echo"));
        tasks.insert("darwin-only".into(), TaskConfig {
            platforms: vec![Platform::Darwin],
            ..sample_task("true")
        });
        tasks.insert("disabled".into(), TaskConfig {
            enabled: false,
            ..sample_task("false")
        });
        Config { tasks, defaults: TaskDefaults::default() }
    }

    #[test]
    fn tasks_for_darwin_filters_correctly() {
        let cfg = sample_config();
        let tasks = cfg.tasks_for(Platform::Darwin);
        assert!(tasks.contains_key("echo-test"));
        assert!(tasks.contains_key("darwin-only"));
        assert!(!tasks.contains_key("disabled"));
    }

    #[test]
    fn tasks_for_linux_excludes_darwin_only() {
        let cfg = sample_config();
        let tasks = cfg.tasks_for(Platform::Linux);
        assert!(tasks.contains_key("echo-test"));
        assert!(!tasks.contains_key("darwin-only"));
    }

    #[test]
    fn validate_catches_empty_command() {
        let mut cfg = sample_config();
        cfg.tasks.insert("bad".into(), sample_task(""));
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("empty command")));
    }

    #[test]
    fn validate_catches_no_platforms() {
        let mut cfg = sample_config();
        cfg.tasks.insert("bad".into(), TaskConfig {
            platforms: vec![],
            ..sample_task("echo")
        });
        let issues = cfg.validate();
        assert!(issues.iter().any(|i| i.contains("no platforms")));
    }

    #[test]
    fn validate_clean_config_has_no_issues() {
        let cfg = sample_config();
        assert!(cfg.validate().is_empty());
    }

    #[test]
    fn schedule_display_interval_hours() {
        let s = Schedule::Interval { seconds: 7200 };
        assert_eq!(s.to_string(), "every 2h");
    }

    #[test]
    fn schedule_display_interval_minutes() {
        let s = Schedule::Interval { seconds: 300 };
        assert_eq!(s.to_string(), "every 5m");
    }

    #[test]
    fn schedule_display_interval_seconds() {
        let s = Schedule::Interval { seconds: 45 };
        assert_eq!(s.to_string(), "every 45s");
    }

    #[test]
    fn schedule_display_calendar_daily() {
        let s = Schedule::Calendar {
            month: None, day: None, weekday: None,
            hour: Some(3), minute: Some(0),
        };
        assert_eq!(s.to_string(), "03:00");
    }

    #[test]
    fn schedule_display_calendar_weekly() {
        let s = Schedule::Calendar {
            month: None, day: None, weekday: Some(7),
            hour: Some(4), minute: Some(30),
        };
        assert_eq!(s.to_string(), "Sun 04:30");
    }

    #[test]
    fn schedule_display_cron() {
        let s = Schedule::Cron { expression: "*-*-* 03:00:00".into() };
        assert_eq!(s.to_string(), "*-*-* 03:00:00");
    }

    #[test]
    fn yaml_roundtrip() {
        let cfg = sample_config();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.tasks.len(), cfg.tasks.len());
    }

    #[test]
    fn defaults_are_sensible() {
        let d = TaskDefaults::default();
        assert!(d.low_priority);
        assert_eq!(d.timeout_secs, 3600);
        assert_eq!(d.platforms.len(), 2);
    }

    #[test]
    fn static_source_returns_config() {
        let cfg = sample_config();
        let src = StaticSource(cfg.clone());
        let loaded = src.load().unwrap();
        assert_eq!(loaded.tasks.len(), cfg.tasks.len());
    }

    #[test]
    fn yaml_parse_minimal() {
        let yaml = r#"
tasks:
  hello:
    description: "say hello"
    command: echo
    args: ["hello"]
    schedule:
      type: interval
      seconds: 60
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.tasks.len(), 1);
        assert!(cfg.tasks.contains_key("hello"));
    }

    #[test]
    fn yaml_parse_calendar_schedule() {
        let yaml = r#"
tasks:
  cleanup:
    description: "clean up"
    command: rm
    schedule:
      type: calendar
      hour: 3
      minute: 0
      weekday: 7
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let task = &cfg.tasks["cleanup"];
        match &task.schedule {
            Schedule::Calendar { hour, minute, weekday, .. } => {
                assert_eq!(*hour, Some(3));
                assert_eq!(*minute, Some(0));
                assert_eq!(*weekday, Some(7));
            }
            other => panic!("expected Calendar, got {other:?}"),
        }
    }
}
