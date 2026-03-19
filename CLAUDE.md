# Teiki (定期) — Cross-Platform Scheduled Task Management

## Build & Test

```bash
cargo build
cargo test --lib
```

## Architecture

Two-layer system: Rust binary + Nix home-manager module.

### Rust Binary

| Command | Purpose |
|---------|---------|
| `teiki run <name>` | Execute a specific task from config |
| `teiki run-all` | Execute all enabled tasks for current platform |
| `teiki list` | Show configured tasks, schedules, and platforms |
| `teiki validate` | Validate the configuration file |
| `teiki show` | Print resolved configuration as YAML |
| `teiki init` | Generate a sample configuration file |

### Nix Module (home-manager)

`blackmatter.components.scheduledTasks` — declarative task definitions that generate:
- `~/.config/teiki/teiki.yaml` (config file for the binary)
- Per-task launchd agents (darwin) or systemd user timers (linux)
- Each service calls `teiki run <task-name> --json`

### Config (shikumi)

Config discovery: `~/.config/teiki/teiki.yaml`
Env override: `TEIKI_CONFIG=/path/to/config.yaml`
Env prefix: `TEIKI_` (e.g. `TEIKI_DEFAULTS__TIMEOUT_SECS=7200`)

```yaml
defaults:
  low_priority: true
  timeout_secs: 3600
  platforms: [darwin, linux]

tasks:
  rust-cleanup:
    description: "Clean Rust target/ directories"
    command: seibi
    args: ["rust-cleanup", "--paths", "~/code"]
    schedule:
      type: calendar
      hour: 3
      minute: 0
    platforms: [darwin]
    tags: [cleanup, disk]
```

### Schedule Types

| Type | YAML | launchd | systemd |
|------|------|---------|---------|
| Interval | `type: interval, seconds: 3600` | `StartInterval` | `OnUnitActiveSec` |
| Calendar | `type: calendar, hour: 3` | `StartCalendarInterval` | `OnCalendar` |
| Cron | `type: cron, expression: "..."` | N/A | `OnCalendar` |

### Platform Services Generated

**Darwin:** `launchd.agents.teiki-<name>` with `io.pleme.teiki.<name>` label
**Linux:** `systemd.user.services.teiki-<name>` + `systemd.user.timers.teiki-<name>`

## Design Decisions

- **Edition 2024**, rust-version 1.89.0
- **shikumi** for config discovery + env overrides
- **OS handles scheduling** — teiki is an executor, not a scheduler daemon
- **seibi** as the default task runtime (wraps system commands with logging)
- **Tags** for filtering (`teiki list --tag cleanup`)
- **Timeouts** with configurable per-task limits
- **Failure notifications** via webhook (Discord, Slack, etc.)
