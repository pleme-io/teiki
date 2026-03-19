/// Typed errors for library consumers. Prefer these over anyhow in public APIs.
#[derive(Debug, thiserror::Error)]
pub enum TeikiError {
    #[error("task '{name}' not found or not enabled for platform '{platform}'")]
    TaskNotFound { name: String, platform: String },

    #[error("config not found: {0}")]
    ConfigNotFound(String),

    #[error("config parse error: {0}")]
    ConfigParse(String),

    #[error("task '{name}' failed to spawn: {source}")]
    Spawn { name: String, source: std::io::Error },

    #[error("validation failed: {issues:?}")]
    Validation { issues: Vec<String> },
}

pub type Result<T> = std::result::Result<T, TeikiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_not_found_display() {
        let e = TeikiError::TaskNotFound {
            name: "cleanup".into(),
            platform: "linux".into(),
        };
        assert!(e.to_string().contains("cleanup"));
        assert!(e.to_string().contains("linux"));
    }

    #[test]
    fn validation_display() {
        let e = TeikiError::Validation {
            issues: vec!["empty command".into()],
        };
        assert!(e.to_string().contains("empty command"));
    }

    #[test]
    fn config_not_found_display() {
        let e = TeikiError::ConfigNotFound("tried 5 paths".into());
        assert!(e.to_string().contains("tried 5 paths"));
    }

    #[test]
    fn spawn_error_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let e = TeikiError::Spawn { name: "test".into(), source: io_err };
        assert!(e.to_string().contains("test"));
        assert!(e.to_string().contains("not found"));
    }
}
