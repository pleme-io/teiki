use crate::config::TaskConfig;
use anyhow::{Context, Result};
use std::process::ExitCode;
use std::time::Instant;
use tracing::{error, info};

/// Execute a single task by name
pub async fn run_task(name: &str, task: &TaskConfig) -> Result<ExitCode> {
    info!(task = name, command = %task.command, "starting task");
    let start = Instant::now();

    let mut cmd = tokio::process::Command::new(&task.command);
    cmd.args(&task.args);

    // Environment
    for (k, v) in &task.env {
        cmd.env(k, v);
    }

    // Extend PATH
    if !task.extra_path.is_empty() {
        let current_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{current_path}", task.extra_path.join(":"));
        cmd.env("PATH", new_path);
    }

    // Working directory
    if let Some(dir) = &task.working_directory {
        cmd.current_dir(dir);
    }

    // Execute with optional timeout
    let result = if task.timeout_secs > 0 {
        let timeout = std::time::Duration::from_secs(task.timeout_secs);
        match tokio::time::timeout(timeout, cmd.status()).await {
            Ok(status) => status.context("spawning task"),
            Err(_) => {
                error!(
                    task = name,
                    timeout_secs = task.timeout_secs,
                    "task timed out"
                );
                return Ok(ExitCode::FAILURE);
            }
        }
    } else {
        cmd.status().await.context("spawning task")
    };

    let elapsed = start.elapsed();

    match result {
        Ok(status) if status.success() => {
            info!(
                task = name,
                elapsed_ms = elapsed.as_millis() as u64,
                "task completed successfully"
            );
            Ok(ExitCode::SUCCESS)
        }
        Ok(status) => {
            let code = status.code().unwrap_or(-1);
            error!(
                task = name,
                exit_code = code,
                elapsed_ms = elapsed.as_millis() as u64,
                "task failed"
            );
            if let Some(url) = &task.notify_on_failure {
                notify_failure(name, code, url).await;
            }
            Ok(ExitCode::FAILURE)
        }
        Err(e) => {
            error!(task = name, error = %e, "failed to spawn task");
            if let Some(url) = &task.notify_on_failure {
                notify_failure(name, -1, url).await;
            }
            Err(e)
        }
    }
}

/// Send failure notification via webhook (best-effort)
async fn notify_failure(task_name: &str, exit_code: i32, webhook_url: &str) {
    let body = serde_json::json!({
        "text": format!("teiki task `{task_name}` failed (exit {exit_code})")
    });
    let client = reqwest::Client::new();
    let _ = client.post(webhook_url).json(&body).send().await;
}
