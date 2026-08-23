//! # Somnium ECS
//!
//! Entity Component System for the **Somnium Engine**.
//!
//! This crate provides a custom, archetype-based ECS with cache-coherent
//! Struct-of-Arrays (`SoA`) component storage, type-safe queries, and
//! ergonomic entity management.
//!
//! ## Core Concepts
//!
//! - **Entity** — A lightweight handle (index + generation) identifying
//!   a game object.
//! - **Component** — Plain data struct (`Send + Sync + 'static`)
//!   attached to entities.
//! - **Archetype** — A group of entities sharing the same component
//!   set, stored in parallel dense arrays.
//! - **World** — The top-level container owning all entities and
//!   component storage.
//!
//! ## Quick Start
//!
//! ```
//! use somnium_ecs::{World, Component};
//!
//! #[derive(Debug, Clone, Copy)]
//! struct Position { x: f32, y: f32 }
//! impl Component for Position {}
//!
//! #[derive(Debug, Clone, Copy)]
//! struct Velocity { dx: f32, dy: f32 }
//! impl Component for Velocity {}
//!
//! let mut world = World::new();
//!
//! // Spawn entities with component tuples.
//! let player = world.spawn((
//!     Position { x: 0.0, y: 0.0 },
//!     Velocity { dx: 1.0, dy: 0.5 },
//! ));
//!
//! // Read components.
//! let pos = world.get::<Position>(player).unwrap();
//! assert_eq!(pos.x, 0.0);
//!
//! // Mutate components.
//! world.get_mut::<Position>(player).unwrap().x += 10.0;
//! assert_eq!(world.get::<Position>(player).unwrap().x, 10.0);
//!
//! // Despawn.
//! world.despawn(player);
//! assert!(!world.is_alive(player));
//! ```
//!
//! ## Reference Architecture
//!
//! The archetype-based storage design is informed by multiple reference
//! codebases:
//!
//! - **Unreal Engine 5 `MassEntity`** (© Epic Games, Inc.) — archetype
//!   tables with chunk-based iteration. See
//!   `example_repo/UnrealEngine-release/.../MassEntity/Public/`.
//!   Key patterns: `FMassArchetypeData`, `FMassArchetypeChunkIterator`,
//!   `FMassEntityManager`, `FMassCommandBuffer`.
//!
//! - **The Forge** (© Confetti FX) — `IApp` lifecycle integration
//!   and `IVisibilityBuffer` data-oriented geometry batching. See
//!   `example_repo/The-Forge-master/Common_3/`.
//!
//! - **bgfx** (© Branimir Karadzic) — `ViewState` per-view data
//!   organisation and `StateCacheLru` for state deduplication. See
//!   `example_repo/bgfx-master/`.
//!
//! - **Unity ML-Agents** (© Unity Technologies) — agent/sensor
//!   component patterns for future AI integration.
//!
//! - **Unity uGUI** (© Unity Technologies) — component-based UI
//!   hierarchy patterns for future UI system.

#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod archetype;
pub mod component;
/// Phase CONTROL-K: authored curves, colour gradients and slider response.
pub mod curve;
pub mod entity;
pub mod persistent;
pub mod reflect;
pub mod world;

// ── Re-exports ─────────────────────────────────────────────────────

pub use archetype::{Archetype, ArchetypeId};
pub use component::{Component, ComponentId, ComponentInfo, ComponentSet};
pub use curve::{Curve, CurveKey, Gradient, GradientStop, Interpolation, SliderCurve};
pub use entity::{Entity, EntityAllocator};
pub use persistent::PersistentId;
pub use reflect::{
    AssetRef, ComponentSchema, FieldFlags, FieldId, FieldSchema, FieldType, ReflectError,
    ReflectField, ReflectObject, ReflectValue, StableId, TypeRegistry,
};
pub use world::{ComponentBundle, EcsError, World};
