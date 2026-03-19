use std::time::Duration;

/// Result of executing a single task.
#[derive(Debug, Clone)]
pub struct TaskOutcome {
    pub task_name: String,
    pub exit_code: i32,
    pub elapsed: Duration,
}

impl TaskOutcome {
    #[must_use]
    pub fn success(name: impl Into<String>, elapsed: Duration) -> Self {
        Self {
            task_name: name.into(),
            exit_code: 0,
            elapsed,
        }
    }

    #[must_use]
    pub fn failure(name: impl Into<String>, exit_code: i32, elapsed: Duration) -> Self {
        Self {
            task_name: name.into(),
            exit_code,
            elapsed,
        }
    }

    #[must_use]
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
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
        assert_eq!(o.task_name, "test");
    }

    #[test]
    fn failure_has_nonzero_exit() {
        let o = TaskOutcome::failure("test", 1, Duration::from_millis(500));
        assert!(!o.is_success());
        assert_eq!(o.exit_code, 1);
    }

    #[test]
    fn clone_preserves_fields() {
        let a = TaskOutcome::success("x", Duration::from_secs(2));
        let b = a.clone();
        assert_eq!(a.task_name, b.task_name);
        assert_eq!(a.elapsed, b.elapsed);
    }
}
