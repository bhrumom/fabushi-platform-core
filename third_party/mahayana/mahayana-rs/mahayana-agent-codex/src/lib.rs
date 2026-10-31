// Temporary narrow lint scope: the implementation still carries two migration-only
// app-server protocol imports while repository marketplace registration is being
// integrated. Keep this scoped to the implementation module; do not relax crate
// or workspace warning policy.
#[allow(unused_imports)]
#[path = "implementation.rs"]
mod implementation;

pub use implementation::{CodexAgentBackend, CodexAgentConfig};
