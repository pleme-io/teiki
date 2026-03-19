use std::fmt;
use std::time::Duration;

/// Result of executing a single task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskOutcome {
    pub task_name: String,
    pub exit_code: i32,
    pub elapsed: Duration,
}

impl TaskOutcome {
    #[must_use]
    pub fn success(name: impl Into<String>, elapsed: Duration) -> Self {
        Self { task_name: name.into(), exit_code: 0, elapsed }
    }

    #[must_use]
    pub fn failure(name: impl Into<String>, exit_code: i32, elapsed: Duration) -> Self {
        Self { task_name: name.into(), exit_code, elapsed }
    }

    #[must_use]
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}

impl fmt::Display for TaskOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.is_success() { "ok" } else { "FAIL" };
        write!(
            f,
            "{} {} (exit {}, {:.1}s)",
            self.task_name,
            status,
            self.exit_code,
            self.elapsed.as_secs_f64()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_has_zero_exit() {
        let o = TaskOutcome::success("test", Duration::from_secs(1));
        assert!(o.is_success());
        assert_eq!(o.exit_code, 0);
    }

    #[test]
    fn failure_has_nonzero_exit() {
        let o = TaskOutcome::failure("test", 1, Duration::from_millis(500));
        assert!(!o.is_success());
    }

    #[test]
    fn eq_compares_all_fields() {
        let a = TaskOutcome::success("x", Duration::from_secs(1));
        let b = TaskOutcome::success("x", Duration::from_secs(1));
        let c = TaskOutcome::success("y", Duration::from_secs(1));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn display_success() {
        let o = TaskOutcome::success("cleanup", Duration::from_millis(1234));
        let s = o.to_string();
        assert!(s.contains("cleanup"));
        assert!(s.contains("ok"));
        assert!(s.contains("exit 0"));
    }

    #[test]
    fn display_failure() {
        let o = TaskOutcome::failure("build", 127, Duration::from_secs(5));
        let s = o.to_string();
        assert!(s.contains("FAIL"));
        assert!(s.contains("exit 127"));
    }
}
