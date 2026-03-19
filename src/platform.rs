use serde::{Deserialize, Serialize};
use std::fmt;

/// Target platform for task execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Darwin,
    Linux,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Darwin => f.write_str("darwin"),
            Self::Linux => f.write_str("linux"),
        }
    }
}

/// Detects the platform at runtime. Trait-based for testability.
pub trait PlatformDetector: Send + Sync {
    fn current(&self) -> Platform;
}

/// Compile-time platform detection (production default).
#[derive(Debug, Clone, Copy, Default)]
pub struct NativePlatform;

impl PlatformDetector for NativePlatform {
    fn current(&self) -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Darwin
        } else {
            Platform::Linux
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Mock platform detector for cross-platform test coverage.
    pub struct MockPlatform(pub Platform);

    impl PlatformDetector for MockPlatform {
        fn current(&self) -> Platform {
            self.0
        }
    }

    #[test]
    fn display_darwin() {
        assert_eq!(Platform::Darwin.to_string(), "darwin");
    }

    #[test]
    fn display_linux() {
        assert_eq!(Platform::Linux.to_string(), "linux");
    }

    #[test]
    fn native_returns_consistent() {
        let d = NativePlatform;
        let a = d.current();
        let b = d.current();
        assert_eq!(a, b);
    }

    #[test]
    fn mock_returns_injected() {
        let d = MockPlatform(Platform::Linux);
        assert_eq!(d.current(), Platform::Linux);
    }

    #[test]
    fn serde_roundtrip() {
        let json = serde_json::to_string(&Platform::Darwin).unwrap();
        assert_eq!(json, "\"darwin\"");
        let parsed: Platform = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Platform::Darwin);
    }

    #[test]
    fn eq_and_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Platform::Darwin);
        set.insert(Platform::Darwin);
        set.insert(Platform::Linux);
        assert_eq!(set.len(), 2);
    }
}
