//! Phase 26-Zeta-I — editor construction, split by surface.
//!
//! See `shell.rs` for the module tree's rationale. Each submodule builds one
//! surface and returns its handles; none of them owns state.

pub(crate) mod content;
pub(crate) mod help;
pub(crate) mod inspector;
pub(crate) mod parts;
pub(crate) mod shell;
