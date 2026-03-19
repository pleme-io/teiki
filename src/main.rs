use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::process::ExitCode;

mod config;
mod executor;

#[derive(Parser)]
#[command(
    name = "teiki",
    version,
    about = "Cross-platform scheduled task management — declarative, configurable, observable"
)]
struct Cli {
    /// Enable JSON log output (for systemd journal / structured logging)
    #[arg(long, global = true)]
    json: bool,

    /// Path to config file (default: shikumi discovery ~/.config/teiki/teiki.yaml)
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a specific task by name
    Run {
        /// Task name from config
        name: String,
    },

    /// Execute all enabled tasks for the current platform
    RunAll,

    /// List configured tasks and their schedules
    List {
        /// Show only tasks for the current platform
        #[arg(long)]
        current_platform: bool,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },

    /// Validate the configuration file
    Validate,

    /// Print the resolved configuration as YAML
    Show,

    /// Generate a sample configuration file
    Init {
        /// Output path (default: stdout)
        #[arg(long)]
        output: Option<std::path::PathBuf>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.json);

    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            tracing::error!(error = %e, "fatal");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Run { name } => {
            let cfg = load_config(&cli.config)?;
            let tasks = cfg.tasks_for_platform();
            let task = tasks
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("task '{name}' not found or not enabled for this platform"))?;
            executor::run_task(&name, task).await
        }

        Command::RunAll => {
            let cfg = load_config(&cli.config)?;
            let tasks = cfg.tasks_for_platform();
            if tasks.is_empty() {
                tracing::info!("no tasks enabled for this platform");
                return Ok(ExitCode::SUCCESS);
            }
            let mut any_failed = false;
            for (name, task) in &tasks {
                let result = executor::run_task(name, task).await?;
                if result != ExitCode::SUCCESS {
                    any_failed = true;
                }
            }
            Ok(if any_failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }

        Command::List {
            current_platform,
            tag,
        } => {
            let cfg = load_config(&cli.config)?;
            let tasks = if current_platform {
                cfg.tasks_for_platform()
            } else {
                cfg.tasks
                    .iter()
                    .map(|(k, v)| (k.clone(), v))
                    .collect()
            };

            for (name, task) in &tasks {
                if let Some(ref t) = tag {
                    if !task.tags.contains(t) {
                        continue;
                    }
                }
                let status = if task.enabled { "enabled" } else { "disabled" };
                let schedule = task.schedule.describe();
                let platforms: Vec<&str> = task
                    .platforms
                    .iter()
                    .map(|p| match p {
                        config::Platform::Darwin => "darwin",
                        config::Platform::Linux => "linux",
                    })
                    .collect();
                let tags = if task.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", task.tags.join(", "))
                };
                println!(
                    "{name:24} {status:8} {schedule:30} ({}){}",
                    platforms.join(", "),
                    tags
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Validate => {
            let cfg = load_config(&cli.config)?;
            let task_count = cfg.tasks.len();
            let enabled = cfg.tasks.values().filter(|t| t.enabled).count();
            let platform_tasks = cfg.tasks_for_platform();
            tracing::info!(
                tasks = task_count,
                enabled,
                current_platform = platform_tasks.len(),
                "configuration valid"
            );

            // Check for common issues
            for (name, task) in &cfg.tasks {
                if task.command.is_empty() {
                    bail!("task '{name}' has empty command");
                }
                if task.enabled && task.platforms.is_empty() {
                    tracing::warn!(task = name, "enabled but no platforms specified");
                }
            }

            Ok(ExitCode::SUCCESS)
        }

        Command::Show => {
            let cfg = load_config(&cli.config)?;
            let yaml = serde_yaml::to_string(&cfg)?;
            print!("{yaml}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Init { output } => {
            let sample = sample_config();
            match output {
                Some(path) => {
                    std::fs::write(&path, &sample)?;
                    tracing::info!(path = %path.display(), "wrote sample config");
                }
                None => print!("{sample}"),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn load_config(path: &Option<std::path::PathBuf>) -> Result<config::Config> {
    match path {
        Some(p) => config::Config::load_from(p),
        None => config::Config::load(),
    }
}

fn init_tracing(json: bool) {
    use tracing_subscriber::{fmt, EnvFilter};

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if json {
        fmt().json().with_env_filter(filter).init();
    } else {
        fmt().with_env_filter(filter).init();
    }
}

fn sample_config() -> String {
    r#"# teiki — scheduled task configuration
# Config discovery: ~/.config/teiki/teiki.yaml
# Override with TEIKI_ env prefix or --config flag

defaults:
  low_priority: true
  timeout_secs: 3600
  platforms: [darwin, linux]

tasks:
  rust-cleanup:
    description: "Clean Rust target/ directories to reclaim disk space"
    command: seibi
    args: ["rust-cleanup", "--paths", "~/code"]
    schedule:
      type: calendar
      hour: 3
      minute: 0
    platforms: [darwin]
    tags: [cleanup, disk]

  docker-cleanup:
    description: "Prune unused Docker images, containers, and volumes"
    command: docker
    args: ["system", "prune", "-af", "--volumes"]
    schedule:
      type: calendar
      weekday: 7
      hour: 4
      minute: 0
    platforms: [darwin]
    tags: [cleanup, docker]

  attic-push:
    description: "Push Nix store to Attic binary cache"
    command: seibi
    args: ["attic-push", "--json"]
    schedule:
      type: interval
      seconds: 3600
    tags: [cache, nix]

  dns-refresh:
    description: "Flush macOS DNS cache to pick up topology changes"
    command: /usr/bin/dscacheutil
    args: ["-flushcache"]
    schedule:
      type: interval
      seconds: 3600
    platforms: [darwin]
    tags: [dns]

  ddns-update:
    description: "Update Cloudflare DNS with current public IP"
    command: seibi
    args: ["ddns", "--json"]
    schedule:
      type: interval
      seconds: 300
    platforms: [linux]
    tags: [dns, network]

  nix-gc:
    description: "Garbage-collect old Nix store paths"
    command: nix-collect-garbage
    args: ["--delete-older-than", "3d"]
    schedule:
      type: calendar
      weekday: 0
      hour: 2
      minute: 0
    tags: [cleanup, nix]
"#
    .to_string()
}
