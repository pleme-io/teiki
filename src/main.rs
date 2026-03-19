use clap::{Parser, Subcommand};
use std::process::ExitCode;

mod app;
mod config;
mod executor;
mod notifier;
mod outcome;
mod platform;

#[derive(Parser)]
#[command(name = "teiki", version, about = "Cross-platform scheduled task management")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,

    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a specific task by name
    Run { name: String },
    /// Execute all enabled tasks for the current platform
    RunAll,
    /// List configured tasks and their schedules
    List {
        #[arg(long)]
        current_platform: bool,
        #[arg(long)]
        tag: Option<String>,
    },
    /// Validate the configuration file
    Validate,
    /// Print the resolved configuration as YAML
    Show,
    /// Generate a sample configuration file
    Init {
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

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    // Wire production dependencies
    let config_source = match cli.config {
        Some(p) => config::ShikumiSource::with_path(p),
        None => config::ShikumiSource::new(),
    };
    let runner = executor::ProcessRunner::new(notifier::NoopNotifier);
    let platform = platform::NativePlatform;
    let app = app::App::new(config_source, runner, platform);

    match cli.command {
        Command::Run { name } => app.run_task(&name).await,
        Command::RunAll => app.run_all().await,
        Command::List { current_platform, tag } => {
            app.list(current_platform, tag.as_deref())
        }
        Command::Validate => app.validate(),
        Command::Show => app.show(),
        Command::Init { output } => {
            let sample = include_str!("sample.yaml");
            match output {
                Some(path) => {
                    std::fs::write(&path, sample)?;
                    tracing::info!(path = %path.display(), "wrote sample config");
                }
                None => print!("{sample}"),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn init_tracing(json: bool) {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if json { fmt().json().with_env_filter(filter).init(); }
    else { fmt().with_env_filter(filter).init(); }
}
