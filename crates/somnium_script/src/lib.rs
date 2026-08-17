//! # Somnium Script
//!
//! The **language-neutral** half of Phase 16 scripting.
//!
//! This crate defines what a script is allowed to see, what it is allowed
//! to ask for, what the engine stores about it, and the trait a runtime
//! implements. It does not know what Luau is, and it must not learn:
//! every scripting-runtime type lives in a backend crate
//! (`somnium_script_luau`) and nowhere else.
//!
//! That rule is the engine's exit strategy, stated as an import
//! constraint. Scenes reference asset ids, instance ids, an API version
//! and neutral property values; engine work happens through
//! [`ScriptCommand`]; the API is generated from one schema. Replacing the
//! language means writing a second [`ScriptBackend`] — not rewriting the
//! ECS, the scene format, the undo stack or the editor.
//!
//! ## The shape of a frame
//!
//! ```text
//!            ┌──────────────── ScriptSnapshot ─────────────────┐
//!            │  time · input · self identity · own components  │
//!            │  pending events · last phase's spawn results    │
//!            └──────────────────────┬─────────────────────────┘
//!                                   │            ┌── WorldView ──┐
//!                                   ▼            │ copy-out reads│
//!                        ScriptBackend::invoke ◄─┴───────────────┘
//!                                   │
//!                                   ▼
//!                            CommandBuffer
//!                                   │  sorted by OrderKey
//!                                   ▼
//!                   validate · apply at the phase boundary
//! ```
//!
//! A script never holds a borrow into engine storage, so nothing it does
//! can invalidate an iteration in progress; and because the merge point
//! already exists and already has a total order, the same gameplay code
//! stays valid if script execution is parallelised later.
//!
//! ## What is deliberately absent
//!
//! No VM. No file watching. No editor types. No `serde`. Those belong to
//! the backend, to `somnium_core`, and to the editor respectively.
//!
//! [`ScriptCommand`]: crate::command::ScriptCommand
//! [`ScriptBackend`]: crate::backend::ScriptBackend

#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod attachment;
pub mod backend;
pub mod command;
pub mod ids;
pub mod order;
pub mod snapshot;
pub mod value;

// ── Re-exports ─────────────────────────────────────────────────────

pub use attachment::{CURRENT_API_VERSION, PropertyBag, ScriptAttachment, ScriptSet};
pub use backend::{
    Budget, Callback, CallbackMask, ComponentUse, CompiledModule, Diagnostic, Diagnostics,
    PhaseCall,
    ScriptBackend, ScriptError, ScriptFieldSchema, ScriptSchema, ScriptSource, Severity,
};
pub use command::{
    CommandBuffer, ForceMode, LogLevel, QueuedCommand, ScriptCommand, SpawnToken,
};
pub use ids::{InstanceUuid, LanguageTag, ScriptAssetId, ScriptInstanceId};
pub use order::OrderKey;
pub use snapshot::{InputSnapshot, ScriptEvent, ScriptSnapshot, TimeSnapshot, WorldView};
pub use value::{ScriptObject, ScriptValue};
