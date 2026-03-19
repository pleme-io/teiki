# teiki — cross-platform scheduled task management
#
# Two-layer architecture:
#   1. Nix module (this file) — declarative task definitions, generates YAML + services
#   2. Rust binary (teiki) — executes tasks with logging, timeouts, notifications
#
# The module generates:
#   - ~/.config/teiki/teiki.yaml (task config consumed by the teiki binary)
#   - Per-task launchd agents (darwin) or systemd user timers (linux)
#   - Each service calls `teiki run <task-name>` which reads the YAML and executes
#
# Usage:
#   blackmatter.components.scheduledTasks = {
#     enable = true;
#     package = inputs.teiki.packages.${system}.default;
#     tasks.rust-cleanup = {
#       description = "Clean Rust target/ directories";
#       command = "seibi";
#       args = [ "rust-cleanup" "--paths" "~/code" ];
#       schedule.calendar = { Hour = 3; Minute = 0; };
#     };
#   };
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.blackmatter.components.scheduledTasks;
  isDarwin = pkgs.stdenv.isDarwin;
  homeDir = config.home.homeDirectory;

  # Filter tasks for current platform
  platformKey = if isDarwin then "darwin" else "linux";
  enabledTasks = filterAttrs (_: t:
    t.enable && builtins.elem platformKey t.platforms
  ) cfg.tasks;

  # Convert Nix task options to the YAML config structure teiki expects
  taskToYaml = name: task: {
    description = task.description;
    enabled = task.enable;
    command = task.command;
    args = task.args;
    env = task.env;
    extra_path = task.extraPath;
    schedule =
      if task.schedule.interval != null then {
        type = "interval";
        seconds = task.schedule.interval;
      }
      else if task.schedule.calendar != null then {
        type = "calendar";
      } // (optionalAttrs (task.schedule.calendar ? Month) { month = task.schedule.calendar.Month; })
        // (optionalAttrs (task.schedule.calendar ? Day) { day = task.schedule.calendar.Day; })
        // (optionalAttrs (task.schedule.calendar ? Weekday) { weekday = task.schedule.calendar.Weekday; })
        // (optionalAttrs (task.schedule.calendar ? Hour) { hour = task.schedule.calendar.Hour; })
        // (optionalAttrs (task.schedule.calendar ? Minute) { minute = task.schedule.calendar.Minute; })
      else if task.schedule.cron != null then {
        type = "cron";
        expression = task.schedule.cron;
      }
      else {};
    platforms = task.platforms;
    low_priority = task.lowPriority;
    timeout_secs = task.timeoutSecs;
    tags = task.tags;
  } // optionalAttrs (task.workingDirectory != null) {
    working_directory = task.workingDirectory;
  } // optionalAttrs (task.notifyOnFailure != null) {
    notify_on_failure = task.notifyOnFailure;
  };

  # Full YAML config
  yamlConfig = {
    defaults = {
      low_priority = cfg.defaults.lowPriority;
      timeout_secs = cfg.defaults.timeoutSecs;
      platforms = cfg.defaults.platforms;
    } // optionalAttrs (cfg.defaults.notifyOnFailure != null) {
      notify_on_failure = cfg.defaults.notifyOnFailure;
    };
    tasks = mapAttrs taskToYaml cfg.tasks;
  };

  yamlContent = builtins.toJSON yamlConfig;

  # Convert launchd calendar to systemd OnCalendar
  calendarToSystemd = cal: let
    weekdays = {
      "0" = "Sun"; "1" = "Mon"; "2" = "Tue"; "3" = "Wed";
      "4" = "Thu"; "5" = "Fri"; "6" = "Sat"; "7" = "Sun";
    };
    pad = n: if n < 10 then "0${toString n}" else toString n;
    dow = if cal ? Weekday then "${weekdays.${toString cal.Weekday}} " else "";
    month = if cal ? Month then pad cal.Month else "*";
    day = if cal ? Day then pad cal.Day else "*";
    hour = if cal ? Hour then pad cal.Hour else "*";
    minute = if cal ? Minute then pad cal.Minute
             else if cal ? Hour then "00"
             else "*";
  in "${dow}*-${month}-${day} ${hour}:${minute}:00";

  # ── Service generators ────────────────────────────────────────
  mkDarwinAgent = name: task: let
    scheduleAttrs =
      if task.schedule.interval != null then { StartInterval = task.schedule.interval; }
      else if task.schedule.calendar != null then { StartCalendarInterval = task.schedule.calendar; }
      else {};
  in {
    enable = true;
    config = {
      Label = "io.pleme.teiki.${name}";
      ProgramArguments = [
        "${cfg.package}/bin/teiki" "run" name "--json"
      ];
      RunAtLoad = task.runAtLoad;
      ProcessType = if task.lowPriority then "Background" else "Adaptive";
      LowPriorityIO = task.lowPriority;
      Nice = if task.lowPriority then 10 else 0;
      StandardOutPath = "${cfg.logDir}/${name}.log";
      StandardErrorPath = "${cfg.logDir}/${name}.err";
    } // scheduleAttrs;
  };

  mkLinuxService = name: task: {
    Unit = {
      Description = task.description;
    } // optionalAttrs (task.after != []) {
      After = task.after;
    };
    Service = {
      Type = "oneshot";
      ExecStart = "${cfg.package}/bin/teiki run ${name} --json";
    };
  };

  mkLinuxTimer = name: task: let
    timerSpec =
      if task.schedule.interval != null then {
        OnBootSec = task.bootDelay;
        OnUnitActiveSec = "${toString task.schedule.interval}s";
      }
      else if task.schedule.calendar != null then {
        OnCalendar = calendarToSystemd task.schedule.calendar;
      }
      else if task.schedule.cron != null then {
        OnCalendar = task.schedule.cron;
      }
      else {};
  in {
    Unit.Description = "${task.description} timer";
    Timer = timerSpec // {
      Unit = "teiki-${name}.service";
      Persistent = task.persistent;
    } // optionalAttrs (task.randomDelay != null) {
      RandomizedDelaySec = task.randomDelay;
    };
    Install.WantedBy = ["timers.target"];
  };

  # ── Task submodule ────────────────────────────────────────────
  taskSubmodule = types.submodule {
    options = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Whether this task is enabled.";
      };

      description = mkOption {
        type = types.str;
        description = "Human-readable description of what this task does.";
      };

      command = mkOption {
        type = types.str;
        description = "Command to execute (binary name or absolute path).";
      };

      args = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Arguments passed to the command.";
      };

      env = mkOption {
        type = types.attrsOf types.str;
        default = {};
        description = "Environment variables for the task.";
      };

      extraPath = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Extra directories to prepend to PATH.";
      };

      schedule = {
        interval = mkOption {
          type = types.nullOr types.ints.positive;
          default = null;
          description = "Run every N seconds.";
        };

        calendar = mkOption {
          type = types.nullOr types.attrs;
          default = null;
          description = ''
            Calendar-based schedule using launchd-style attrs:
              { Hour = 3; Minute = 0; }              — daily at 3:00
              { Weekday = 7; Hour = 3; Minute = 0; }  — Sundays at 3:00
            Converted to systemd OnCalendar on Linux.
          '';
        };

        cron = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Systemd OnCalendar expression (Linux only, passed through).";
        };
      };

      platforms = mkOption {
        type = types.listOf (types.enum ["darwin" "linux"]);
        default = cfg.defaults.platforms;
        description = "Platforms this task runs on.";
      };

      lowPriority = mkOption {
        type = types.bool;
        default = cfg.defaults.lowPriority;
        description = "Run as low-priority background task.";
      };

      runAtLoad = mkOption {
        type = types.bool;
        default = false;
        description = "Darwin: run immediately when the agent loads.";
      };

      workingDirectory = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Working directory for the command.";
      };

      timeoutSecs = mkOption {
        type = types.ints.unsigned;
        default = cfg.defaults.timeoutSecs;
        description = "Timeout in seconds (0 = no timeout).";
      };

      tags = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Tags for filtering and grouping (teiki list --tag).";
      };

      notifyOnFailure = mkOption {
        type = types.nullOr types.str;
        default = cfg.defaults.notifyOnFailure;
        description = "Webhook URL to POST on task failure.";
      };

      # Linux-only scheduling options
      after = mkOption {
        type = types.listOf types.str;
        default = [];
        description = "Linux: systemd After= dependencies.";
      };

      bootDelay = mkOption {
        type = types.str;
        default = "30s";
        description = "Linux: delay before first run after boot.";
      };

      randomDelay = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Linux: RandomizedDelaySec for splaying.";
      };

      persistent = mkOption {
        type = types.bool;
        default = true;
        description = "Linux: run missed events after wake/boot.";
      };
    };
  };

in {
  options.blackmatter.components.scheduledTasks = {
    enable = mkEnableOption "teiki cross-platform scheduled task management";

    package = mkOption {
      type = types.package;
      description = "The teiki binary package.";
    };

    logDir = mkOption {
      type = types.str;
      default = if isDarwin
        then "${homeDir}/Library/Logs"
        else "${homeDir}/.local/share/teiki/logs";
      description = "Directory for task stdout/stderr logs.";
    };

    defaults = {
      lowPriority = mkOption {
        type = types.bool;
        default = true;
        description = "Default low_priority for all tasks.";
      };

      timeoutSecs = mkOption {
        type = types.ints.unsigned;
        default = 3600;
        description = "Default timeout for all tasks.";
      };

      platforms = mkOption {
        type = types.listOf (types.enum ["darwin" "linux"]);
        default = ["darwin" "linux"];
        description = "Default platforms for all tasks.";
      };

      notifyOnFailure = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Default webhook URL for failure notifications.";
      };
    };

    tasks = mkOption {
      type = types.attrsOf taskSubmodule;
      default = {};
      description = "Scheduled task definitions.";
    };
  };

  config = mkIf (cfg.enable && enabledTasks != {}) (mkMerge [
    # Assertions
    {
      assertions = mapAttrsToList (name: task: {
        assertion =
          (task.schedule.interval != null)
          || (task.schedule.calendar != null)
          || (task.schedule.cron != null);
        message = "teiki task '${name}' must have at least one of schedule.interval, schedule.calendar, or schedule.cron";
      }) enabledTasks;
    }

    # Write YAML config for the teiki binary
    {
      xdg.configFile."teiki/teiki.yaml".text =
        builtins.toJSON yamlConfig;

      home.packages = [ cfg.package ];
    }

    # Darwin: generate launchd agents
    (mkIf isDarwin {
      launchd.agents = mapAttrs' (name: task:
        nameValuePair "teiki-${name}" (mkDarwinAgent name task)
      ) enabledTasks;
    })

    # Linux: generate systemd user services + timers
    (mkIf (!isDarwin) {
      systemd.user.services = mapAttrs' (name: task:
        nameValuePair "teiki-${name}" (mkLinuxService name task)
      ) enabledTasks;

      systemd.user.timers = mapAttrs' (name: task:
        nameValuePair "teiki-${name}" (mkLinuxTimer name task)
      ) enabledTasks;

      home.activation.teiki-log-dir = lib.hm.dag.entryAfter ["writeBoundary"] ''
        run mkdir -p "${cfg.logDir}"
      '';
    })
  ]);
}
