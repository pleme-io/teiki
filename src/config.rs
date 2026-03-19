use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Root configuration — loaded via shikumi from ~/.config/teiki/teiki.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Task definitions keyed by name
    #[serde(default)]
    pub tasks: BTreeMap<String, TaskConfig>,

    /// Global defaults applied to all tasks (overridden per-task)
    #[serde(default)]
    pub defaults: TaskDefaults,
}

/// Per-task configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Human-readable description
    pub description: String,

    /// Whether this task is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Command to execute (binary name or absolute path)
    pub command: String,

    /// Arguments passed to the command
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables for the task
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Extra directories to add to PATH
    #[serde(default)]
    pub extra_path: Vec<String>,

    /// Scheduling configuration
    pub schedule: Schedule,

    /// Platforms this task runs on
    #[serde(default = "default_platforms")]
    pub platforms: Vec<Platform>,

    /// Run as low-priority background task
    #[serde(default = "default_true")]
    pub low_priority: bool,

    /// Working directory for the command
    #[serde(default)]
    pub working_directory: Option<PathBuf>,

    /// Timeout in seconds (0 = no timeout)
    #[serde(default)]
    pub timeout_secs: u64,

    /// Tags for filtering and grouping
    #[serde(default)]
    pub tags: Vec<String>,

    /// Notification on failure (webhook URL or empty)
    #[serde(default)]
    pub notify_on_failure: Option<String>,

    /// Maximum log file size in bytes before rotation
    #[serde(default)]
    pub max_log_bytes: Option<u64>,
}

/// Schedule specification — interval or calendar based
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    /// Run every N seconds
    Interval {
        seconds: u64,
    },
    /// Run at specific calendar times
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
    /// Cron-style expression (systemd OnCalendar format)
    Cron {
        expression: String,
    },
}

/// Supported platforms
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Darwin,
    Linux,
}

/// Global defaults applied when per-task values are not set
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDefaults {
    /// Default low_priority setting
    #[serde(default = "default_true")]
    pub low_priority: bool,

    /// Default timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Default platforms
    #[serde(default = "default_platforms")]
    pub platforms: Vec<Platform>,

    /// Default notification webhook
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

impl Config {
    /// Load config via shikumi discovery (~/.config/teiki/teiki.yaml)
    pub fn load() -> anyhow::Result<Self> {
        let path = shikumi::ConfigDiscovery::new("teiki")
            .env_override("TEIKI_CONFIG")
            .discover()
            .map_err(|e| anyhow::anyhow!("config not found: {e}"))?;
        let store = shikumi::ConfigStore::<Self>::load(&path, "TEIKI_")
            .map_err(|e| anyhow::anyhow!("failed to load config: {e}"))?;
        Ok((*store.get().as_ref()).clone())
    }

    /// Load from a specific path
    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let store = shikumi::ConfigStore::<Self>::load(path, "TEIKI_")
            .map_err(|e| anyhow::anyhow!("failed to load config: {e}"))?;
        Ok((*store.get().as_ref()).clone())
    }

    /// Get enabled tasks for the current platform
    pub fn tasks_for_platform(&self) -> BTreeMap<String, &TaskConfig> {
        let current = current_platform();
        self.tasks
            .iter()
            .filter(|(_, t)| t.enabled && t.platforms.contains(&current))
            .map(|(k, v)| (k.clone(), v))
            .collect()
    }
}

/// Detect current platform
pub fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Darwin
    } else {
        Platform::Linux
    }
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> u64 {
    3600
}

fn default_platforms() -> Vec<Platform> {
    vec![Platform::Darwin, Platform::Linux]
}

impl Schedule {
    /// Human-readable schedule description
    pub fn describe(&self) -> String {
        match self {
            Self::Interval { seconds } => {
                if *seconds >= 3600 {
                    format!("every {} hour(s)", seconds / 3600)
                } else if *seconds >= 60 {
                    format!("every {} minute(s)", seconds / 60)
                } else {
                    format!("every {seconds} second(s)")
                }
            }
            Self::Calendar {
                month,
                day,
                weekday,
                hour,
                minute,
            } => {
                let weekdays = [
                    "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun",
                ];
                let mut parts = Vec::new();
                if let Some(w) = weekday {
                    parts.push(weekdays[*w as usize % 8].to_string());
                }
                if let Some(m) = month {
                    parts.push(format!("month {m}"));
                }
                if let Some(d) = day {
                    parts.push(format!("day {d}"));
                }
                let h = hour.unwrap_or(0);
                let m = minute.unwrap_or(0);
                parts.push(format!("{h:02}:{m:02}"));
                parts.join(" ")
            }
            Self::Cron { expression } => expression.clone(),
        }
    }
}
