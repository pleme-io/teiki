//! teiki (定期) — cross-platform scheduled task management.
//!
//! Library crate exposing all traits, types, and implementations for
//! embedding teiki's task execution in other Rust applications.
//!
//! # Traits
//!
//! | Trait | Purpose | Production | Mock |
//! |---|---|---|---|
//! | [`ConfigSource`] | Load config | [`ShikumiSource`] | `StaticSource` (in tests) |
//! | [`TaskRunner`] | Execute tasks | [`ProcessRunner`] | `MockRunner` (in tests) |
//! | [`NotifierFactory`] | Failure webhooks | [`HttpNotifierFactory`] | `RecordingNotifierFactory` (in tests) |
//! | [`PlatformDetector`] | Detect OS | [`NativePlatform`] | `MockPlatform` (in tests) |
//!
//! # Quick Start
//!
//! ```ignore
//! use teiki::{App, ShikumiSource, ProcessRunner, NoopNotifierFactory, NativePlatform};
//!
//! let app = App::new(
//!     ShikumiSource::new(),
//!     ProcessRunner::new(NoopNotifierFactory),
//!     NoopNotifierFactory,
//!     NativePlatform,
//! );
//! let outcome = app.run_task("my-task").await?;
//! ```

pub mod app;
pub mod config;
pub mod error;
pub mod executor;
pub mod outcome;
pub mod platform;

// Re-export key types at crate root for ergonomic imports.
pub use app::{App, TaskListEntry, ValidationResult};
pub use config::{Config, ConfigSource, ShikumiSource, TaskConfig, TaskDefaults, Schedule};
pub use error::{TeikiError, Result};
pub use executor::{
    ExecSpec, TaskRunner, NotifierFactory, ProcessRunner,
    HttpNotifierFactory, NoopNotifierFactory, build_command,
};
pub use outcome::TaskOutcome;
pub use platform::{Platform, PlatformDetector, NativePlatform};
