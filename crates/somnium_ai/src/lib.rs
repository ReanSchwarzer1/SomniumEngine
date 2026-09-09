//! MORROWIND-X/Y: deterministic, headless navigation and decision making.
//!
//! Cooking accepts world-space triangles, so Jolt and a graphics device never
//! enter this module. Workers produce immutable cell tiles; owners install them
//! on their simulation thread. Behavior assets are immutable and each agent
//! owns its execution state and blackboard.

pub mod behavior;
pub mod navigation;
pub mod perception;
pub mod script;
