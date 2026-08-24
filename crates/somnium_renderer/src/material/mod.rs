//! Material data.
//!
//! `hlms.rs` used to sit here: 29 lines under a doc comment describing
//! Ogre-Next's HLMS, holding one underscore-prefixed `_pipeline_cache` field
//! no code read, under a trailing comment beginning "In a full implementation,
//! this would...". **MORROWIND-C built what that comment described**, in
//! `somnium_shader`, and deleted the file. See `crate::shaders`.

pub mod pool;
