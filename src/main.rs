use clap::{Parser, Subcommand};
use std::process::ExitCode;
use teiki::{App, ShikumiSource, ProcessRunner, NoopNotifierFactory, NativePlatform};

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
    let config_source = match cli.config {
        Some(p) => ShikumiSource::with_path(p),
        None => ShikumiSource::new(),
    };
    let app = App::new(config_source, ProcessRunner, NoopNotifierFactory, NativePlatform);

    match cli.command {
        Command::Run { name } => app.run_task_exit(&name).await,
        Command::RunAll => app.run_all_exit().await,
        Command::List { current_platform, tag } => app.list_exit(current_platform, tag.as_deref()),
        Command::Validate => app.validate_exit(),
        Command::Show => app.show_exit(),
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
