//! Product-level feature controller over the direct Mahayana Runtime Host.
//!
//! The implementation remains in a dedicated module while migration-only
//! Clippy findings are tracked at module scope. The expectations are narrow:
//! they do not disable warnings for the crate or skip any tests.

#[expect(clippy::collapsible_if, clippy::unneeded_wildcard_pattern)]
#[path = "implementation.rs"]
mod implementation;

pub use implementation::*;
