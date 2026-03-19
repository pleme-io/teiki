//! teiki (定期) — cross-platform scheduled task management.
//!
//! Library crate exposing all traits, types, and implementations for
//! embedding teiki's task execution in other Rust applications.
//!
//! # Traits
//!
//! | Trait | Purpose | Production | Test |
//! |---|---|---|---|
//! | [`ConfigSource`] | Load config | [`ShikumiSource`] | [`StaticSource`] |
//! | [`TaskRunner`] | Execute tasks | [`ProcessRunner`] | [`MockRunner`] |
//! | [`NotifierFactory`] | Failure webhooks | `HttpNotifierFactory` | [`RecordingNotifierFactory`] |
//! | [`PlatformDetector`] | Detect OS | [`NativePlatform`] | [`MockPlatform`] |

pub mod app;
pub mod config;
pub mod error;
pub mod executor;
pub mod outcome;
pub mod platform;

// ── Production types ───────────────────────────────────────────
pub use app::{App, TaskListEntry, ValidationResult};
pub use config::{Config, ConfigSource, ShikumiSource, TaskConfig, TaskDefaults, Schedule};
pub use error::{TeikiError, Result};
pub use executor::{ExecSpec, TaskRunner, NotifierFactory, ProcessRunner, NoopNotifierFactory, build_command};
pub use outcome::TaskOutcome;
pub use platform::{Platform, PlatformDetector, NativePlatform};

// ── Test/mock types (public — usable by downstream crates) ─────
pub use config::tests::{StaticSource, FailingSource};
pub use executor::{MockRunner, RecordingNotifierFactory};
pub use platform::tests::MockPlatform;

#[cfg(feature = "webhooks")]
pub use executor::HttpNotifierFactory;
