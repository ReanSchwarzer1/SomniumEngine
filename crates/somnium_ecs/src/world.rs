//! The ECS World — the top-level container for all entity and component
//! storage.
//!
//! [`World`] is the single entry point for spawning/despawning entities,
//! querying components, and running systems. It owns the entity
//! allocator, the archetype table, and the entity-location map.
//!
//! ## Reference Architecture
//!
//! - **UE5 `FMassEntityManager`** (© Epic Games, Inc.) — the central
//!   manager that owns all archetypes, handles entity creation/
//!   destruction, and routes queries. See
//!   `example_repo/UnrealEngine-release/.../MassEntity/Public/MassEntityManager.h`.
//!
//! - **The Forge `IApp`** (© Confetti FX) — the `Init/Load/Update`
//!   lifecycle informs how the World integrates with the engine's
//!   `GameApp::on_update` callback.

use std::collections::HashMap;

use crate::archetype::{Archetype, ArchetypeId};
use crate::component::{Component, ComponentId, ComponentInfo, ComponentSet};
use crate::entity::{Entity, EntityAllocator};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Errors
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Failure modes of the structural world operations.
///
/// Structural operations return an error rather than panicking because
/// their callers include scripts, which routinely hold handles to
/// entities that have already been destroyed. A stale handle is an
/// ordinary control-flow outcome, not a bug in the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcsError {
    /// The entity handle is stale: its slot has been freed, or recycled
    /// with a newer generation.
    DeadEntity,
}

impl fmt::Display for EcsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeadEntity => write!(f, "entity handle is stale"),
        }
    }
}

impl std::error::Error for EcsError {}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Entity Location
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Where an entity lives: its archetype and row within that archetype.
#[derive(Debug, Clone, Copy)]
struct EntityLocation {
    archetype_id: ArchetypeId,
    row: usize,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Component Bundle helper
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Trait for a tuple of components that can be spawned together.
///
/// Implemented for tuples up to 8 components.
pub trait ComponentBundle: Send + Sync + 'static {
    /// Collect the component IDs and infos.
    fn component_infos() -> Vec<ComponentInfo>;
    /// Push all values into an archetype's columns (given column lookup).
    fn push_into(self, archetype: &mut Archetype);
}

// Implement for single component.
impl<A: Component> ComponentBundle for (A,) {
    fn component_infos() -> Vec<ComponentInfo> {
        vec![ComponentInfo::of::<A>()]
    }
    fn push_into(self, archetype: &mut Archetype) {
        let col = archetype.column_index(ComponentId::of::<A>()).unwrap();
        archetype.column_mut(col).push(self.0);
    }
}

// Implement for 2-tuple.
impl<A: Component, B: Component> ComponentBundle for (A, B) {
    fn component_infos() -> Vec<ComponentInfo> {
        vec![ComponentInfo::of::<A>(), ComponentInfo::of::<B>()]
    }
    fn push_into(self, archetype: &mut Archetype) {
        let col_a = archetype.column_index(ComponentId::of::<A>()).unwrap();
        archetype.column_mut(col_a).push(self.0);
        let col_b = archetype.column_index(ComponentId::of::<B>()).unwrap();
        archetype.column_mut(col_b).push(self.1);
    }
}

// Implement for 3-tuple.
impl<A: Component, B: Component, C: Component> ComponentBundle for (A, B, C) {
    fn component_infos() -> Vec<ComponentInfo> {
        vec![
            ComponentInfo::of::<A>(),
            ComponentInfo::of::<B>(),
            ComponentInfo::of::<C>(),
        ]
    }
    fn push_into(self, archetype: &mut Archetype) {
        let col_a = archetype.column_index(ComponentId::of::<A>()).unwrap();
        archetype.column_mut(col_a).push(self.0);
        let col_b = archetype.column_index(ComponentId::of::<B>()).unwrap();
        archetype.column_mut(col_b).push(self.1);
        let col_c = archetype.column_index(ComponentId::of::<C>()).unwrap();
        archetype.column_mut(col_c).push(self.2);
    }
}

// Implement for 4-tuple.
impl<A: Component, B: Component, C: Component, D: Component> ComponentBundle for (A, B, C, D) {
    fn component_infos() -> Vec<ComponentInfo> {
        vec![
            ComponentInfo::of::<A>(),
            ComponentInfo::of::<B>(),
            ComponentInfo::of::<C>(),
            ComponentInfo::of::<D>(),
        ]
    }
    fn push_into(self, archetype: &mut Archetype) {
        let col_a = archetype.column_index(ComponentId::of::<A>()).unwrap();
        archetype.column_mut(col_a).push(self.0);
        let col_b = archetype.column_index(ComponentId::of::<B>()).unwrap();
        archetype.column_mut(col_b).push(self.1);
        let col_c = archetype.column_index(ComponentId::of::<C>()).unwrap();
        archetype.column_mut(col_c).push(self.2);
        let col_d = archetype.column_index(ComponentId::of::<D>()).unwrap();
        archetype.column_mut(col_d).push(self.3);
    }
}

// Implement for 5-tuple.
impl<A: Component, B: Component, C: Component, D: Component, E: Component> ComponentBundle
    for (A, B, C, D, E)
{
    fn component_infos() -> Vec<ComponentInfo> {
        vec![
            ComponentInfo::of::<A>(),
            ComponentInfo::of::<B>(),
            ComponentInfo::of::<C>(),
            ComponentInfo::of::<D>(),
            ComponentInfo::of::<E>(),
        ]
    }
    fn push_into(self, archetype: &mut Archetype) {
        let col_a = archetype.column_index(ComponentId::of::<A>()).unwrap();
        archetype.column_mut(col_a).push(self.0);
        let col_b = archetype.column_index(ComponentId::of::<B>()).unwrap();
        archetype.column_mut(col_b).push(self.1);
        let col_c = archetype.column_index(ComponentId::of::<C>()).unwrap();
        archetype.column_mut(col_c).push(self.2);
        let col_d = archetype.column_index(ComponentId::of::<D>()).unwrap();
        archetype.column_mut(col_d).push(self.3);
        let col_e = archetype.column_index(ComponentId::of::<E>()).unwrap();
        archetype.column_mut(col_e).push(self.4);
    }
}

// Implement for 6-tuple.
impl<A: Component, B: Component, C: Component, D: Component, E: Component, F: Component>
    ComponentBundle for (A, B, C, D, E, F)
{
    fn component_infos() -> Vec<ComponentInfo> {
        vec![
            ComponentInfo::of::<A>(),
            ComponentInfo::of::<B>(),
            ComponentInfo::of::<C>(),
            ComponentInfo::of::<D>(),
            ComponentInfo::of::<E>(),
            ComponentInfo::of::<F>(),
        ]
    }
    fn push_into(self, archetype: &mut Archetype) {
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<A>()).unwrap())
            .push(self.0);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<B>()).unwrap())
            .push(self.1);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<C>()).unwrap())
            .push(self.2);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<D>()).unwrap())
            .push(self.3);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<E>()).unwrap())
            .push(self.4);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<F>()).unwrap())
            .push(self.5);
    }
}

// Implement for 7-tuple.
impl<
    A: Component,
    B: Component,
    C: Component,
    D: Component,
    E: Component,
    F: Component,
    G: Component,
> ComponentBundle for (A, B, C, D, E, F, G)
{
    fn component_infos() -> Vec<ComponentInfo> {
        vec![
            ComponentInfo::of::<A>(),
            ComponentInfo::of::<B>(),
            ComponentInfo::of::<C>(),
            ComponentInfo::of::<D>(),
            ComponentInfo::of::<E>(),
            ComponentInfo::of::<F>(),
            ComponentInfo::of::<G>(),
        ]
    }
    fn push_into(self, archetype: &mut Archetype) {
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<A>()).unwrap())
            .push(self.0);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<B>()).unwrap())
            .push(self.1);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<C>()).unwrap())
            .push(self.2);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<D>()).unwrap())
            .push(self.3);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<E>()).unwrap())
            .push(self.4);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<F>()).unwrap())
            .push(self.5);
        archetype
            .column_mut(archetype.column_index(ComponentId::of::<G>()).unwrap())
            .push(self.6);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// World
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The ECS world: owns all entity and component storage.
///
/// ```
/// use somnium_ecs::{World, Component};
///
/// #[derive(Debug, Clone, Copy)]
/// struct Position { x: f32, y: f32 }
/// impl Component for Position {}
///
/// #[derive(Debug, Clone, Copy)]
/// struct Velocity { dx: f32, dy: f32 }
/// impl Component for Velocity {}
///
/// let mut world = World::new();
/// let entity = world.spawn((Position { x: 0.0, y: 0.0 }, Velocity { dx: 1.0, dy: 0.0 }));
///
/// assert!(world.is_alive(entity));
/// assert_eq!(world.get::<Position>(entity).unwrap().x, 0.0);
/// ```
pub struct World {
    /// Entity handle allocator.
    entities: EntityAllocator,

    /// All archetypes, indexed by `ArchetypeId`.
    archetypes: Vec<Archetype>,

    /// Maps `ComponentSet → ArchetypeId` for archetype lookup.
    archetype_map: HashMap<ComponentSet, ArchetypeId>,

    /// Maps entity index → location (archetype + row).
    /// Indexed by `Entity::index()`. Entries for dead entities are stale.
    locations: Vec<Option<EntityLocation>>,
}

impl World {
    /// Create a new, empty world.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entities: EntityAllocator::new(),
            archetypes: Vec::new(),
            archetype_map: HashMap::new(),
            locations: Vec::new(),
        }
    }

    /// Spawn an entity with the given component bundle.
    ///
    /// Returns the new entity handle.
    pub fn spawn<B: ComponentBundle>(&mut self, bundle: B) -> Entity {
        let entity = self.entities.allocate();

        // Build the component set for this bundle.
        let infos = B::component_infos();
        let set = ComponentSet::from_ids(infos.iter().map(|i| i.id).collect());

        // Find or create the archetype.
        let arch_id = self.get_or_create_archetype(&set, &infos);
        let arch = &mut self.archetypes[arch_id.raw() as usize];
        let row = arch.allocate_row(entity);
        bundle.push_into(arch);

        // Record location.
        let loc = EntityLocation {
            archetype_id: arch_id,
            row,
        };
        let idx = entity.index() as usize;
        if idx >= self.locations.len() {
            self.locations.resize(idx + 1, None);
        }
        self.locations[idx] = Some(loc);

        entity
    }

    /// Despawn an entity, removing it from the world.
    ///
    /// Returns `true` if the entity was alive and is now removed.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }

        let idx = entity.index() as usize;
        let Some(loc) = self.locations.get(idx).and_then(|l| *l) else {
            return false;
        };

        // Remove from archetype.
        let arch = &mut self.archetypes[loc.archetype_id.raw() as usize];
        if let Some(swapped_entity) = arch.swap_remove_row(loc.row) {
            // Update the swapped entity's location.
            let swapped_idx = swapped_entity.index() as usize;
            if let Some(ref mut swapped_loc) = self.locations[swapped_idx] {
                swapped_loc.row = loc.row;
            }
        }

        self.locations[idx] = None;
        self.entities.free(entity);
        true
    }

    /// Check whether an entity is alive.
    #[must_use]
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    /// Get an immutable reference to a component on an entity.
    ///
    /// Returns `None` if the entity is dead or doesn't have component `T`.
    #[must_use]
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let loc = self.locations[entity.index() as usize]?;
        let arch = &self.archetypes[loc.archetype_id.raw() as usize];
        let col_idx = arch.column_index(ComponentId::of::<T>())?;
        Some(unsafe { arch.column(col_idx).get::<T>(loc.row) })
    }

    /// Get a mutable reference to a component on an entity.
    ///
    /// Returns `None` if the entity is dead or doesn't have component `T`.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let loc = self.locations[entity.index() as usize]?;
        let arch = &mut self.archetypes[loc.archetype_id.raw() as usize];
        let col_idx = arch.column_index(ComponentId::of::<T>())?;
        Some(unsafe { arch.column_mut(col_idx).get_mut::<T>(loc.row) })
    }

    // ── Structural change: runtime component insert / remove ────────
    //
    // Phase 16-A. Everything below moves an entity between archetypes.
    // A component value is moved, never copied and never dropped in
    // transit; the only drop is the one the caller asked for (replacing
    // an existing component, or removing one). `MovedComponent` owns each
    // value while it is between archetypes so that an early return cannot
    // leak it.

    /// Whether `entity` currently has the component identified by `id`.
    #[must_use]
    pub fn has_component(&self, entity: Entity, id: ComponentId) -> bool {
        self.entities
            .is_alive(entity)
            .then(|| self.locations.get(entity.index() as usize).copied().flatten())
            .flatten()
            .is_some_and(|loc| {
                self.archetypes[loc.archetype_id.raw() as usize]
                    .component_set()
                    .contains(id)
            })
    }

    /// Every component type currently on `entity`, in sorted order.
    ///
    /// Sorted order is part of the contract: reflection, serialization and
    /// script snapshots all walk this list, and they must not vary between
    /// runs.
    #[must_use]
    pub fn component_ids(&self, entity: Entity) -> Option<Vec<ComponentId>> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let loc = self.locations.get(entity.index() as usize).copied().flatten()?;
        Some(
            self.archetypes[loc.archetype_id.raw() as usize]
                .component_set()
                .iter()
                .collect(),
        )
    }

    /// Attach a component to an entity that already exists, migrating it
    /// to the archetype that includes `T`.
    ///
    /// If the entity already has a `T`, the old value is dropped and
    /// replaced in place — no migration happens.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::DeadEntity`] if the handle is stale. The value
    /// is dropped in that case rather than leaked.
    pub fn insert_component<T: Component>(
        &mut self,
        entity: Entity,
        value: T,
    ) -> Result<(), EcsError> {
        let info = ComponentInfo::of::<T>();
        let mut staged = std::mem::ManuallyDrop::new(value);
        let src = std::ptr::from_mut::<T>(&mut staged).cast::<u8>();
        // SAFETY: `src` points to a live `T` that nothing else will touch,
        // and `info` describes exactly `T`.
        let result = unsafe { self.insert_erased(entity, &info, src) };
        if result.is_err() {
            // Ownership never transferred, so it is still ours to drop.
            unsafe { std::mem::ManuallyDrop::drop(&mut staged) };
        }
        result
    }

    /// Type-erased [`Self::insert_component`], for the reflection registry
    /// and for scripts, which name component types by stable id rather
    /// than by Rust type.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::DeadEntity`] if the handle is stale. On error
    /// the value at `src` is **not** consumed and the caller still owns it.
    ///
    /// # Panics
    ///
    /// Panics if the destination archetype is missing a component its own
    /// signature declares, which would mean the archetype table is already
    /// corrupt.
    ///
    /// # Safety
    ///
    /// `src` must point to a valid, initialised value of the component
    /// type `info` describes. On success this call takes ownership of that
    /// value and the caller must not drop or reuse it.
    pub unsafe fn insert_erased(
        &mut self,
        entity: Entity,
        info: &ComponentInfo,
        src: *mut u8,
    ) -> Result<(), EcsError> {
        let loc = self.location_of(entity)?;
        let old_idx = loc.archetype_id.raw() as usize;

        // Already present: drop the old value and overwrite the slot. No
        // archetype change, so no migration.
        if self.archetypes[old_idx].component_set().contains(info.id) {
            let col = self.archetypes[old_idx].column_index(info.id).unwrap();
            let dst = self.archetypes[old_idx].column_mut(col).get_raw_mut(loc.row);
            unsafe {
                if let Some(drop_fn) = info.drop_fn {
                    drop_fn(dst);
                }
                if info.layout.size() > 0 {
                    std::ptr::copy_nonoverlapping(src, dst, info.layout.size());
                }
            }
            return Ok(());
        }

        let new_set = self.archetypes[old_idx].component_set().with(info.id);
        let mut infos: Vec<ComponentInfo> = self.archetypes[old_idx].column_infos().cloned().collect();
        infos.push(info.clone());
        let new_arch_id = self.get_or_create_archetype(&new_set, &infos);

        let (mut moved, swapped) = self.archetypes[old_idx].move_out_row(loc.row);
        self.patch_swapped_location(swapped, loc.row);

        let new_arch = &mut self.archetypes[new_arch_id.raw() as usize];
        let row = new_arch.allocate_row(entity);
        for (col_idx, id) in new_set.iter().enumerate() {
            let ptr = if id == info.id {
                src.cast_const()
            } else {
                moved
                    .iter_mut()
                    .find(|m| m.id == id)
                    .expect("migrating archetype lost a component it declared")
                    .relinquish()
            };
            // SAFETY: every pointer here refers to exactly one initialised
            // value of the column's component type, and each `moved` entry
            // is relinquished at most once because ids are unique in a set.
            unsafe { new_arch.column_mut(col_idx).push_moved(ptr) };
        }

        self.locations[entity.index() as usize] = Some(EntityLocation {
            archetype_id: new_arch_id,
            row,
        });
        Ok(())
    }

    /// Detach a component, migrating the entity to the archetype without
    /// it. The removed value is dropped.
    ///
    /// Returns `false` if the entity did not have the component.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::DeadEntity`] if the handle is stale.
    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Result<bool, EcsError> {
        self.remove_erased(entity, ComponentId::of::<T>())
    }

    /// Type-erased [`Self::remove_component`].
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::DeadEntity`] if the handle is stale.
    ///
    /// # Panics
    ///
    /// Panics if the destination archetype is missing a component its own
    /// signature declares, which would mean the archetype table is already
    /// corrupt.
    pub fn remove_erased(&mut self, entity: Entity, id: ComponentId) -> Result<bool, EcsError> {
        let loc = self.location_of(entity)?;
        let old_idx = loc.archetype_id.raw() as usize;
        if !self.archetypes[old_idx].component_set().contains(id) {
            return Ok(false);
        }

        let new_set = self.archetypes[old_idx].component_set().without(id);
        let infos: Vec<ComponentInfo> = self.archetypes[old_idx]
            .column_infos()
            .filter(|i| i.id != id)
            .cloned()
            .collect();
        let new_arch_id = self.get_or_create_archetype(&new_set, &infos);

        let (mut moved, swapped) = self.archetypes[old_idx].move_out_row(loc.row);
        self.patch_swapped_location(swapped, loc.row);

        let new_arch = &mut self.archetypes[new_arch_id.raw() as usize];
        let row = new_arch.allocate_row(entity);
        for (col_idx, keep) in new_set.iter().enumerate() {
            let ptr = moved
                .iter_mut()
                .find(|m| m.id == keep)
                .expect("migrating archetype lost a component it declared")
                .relinquish();
            // SAFETY: as in `insert_erased`.
            unsafe { new_arch.column_mut(col_idx).push_moved(ptr) };
        }
        // The entry for `id` is still un-relinquished, so dropping `moved`
        // here runs exactly that component's destructor and no other.
        drop(moved);

        self.locations[entity.index() as usize] = Some(EntityLocation {
            archetype_id: new_arch_id,
            row,
        });
        Ok(true)
    }

    /// Number of alive entities.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.entities.alive_count()
    }

    /// Iterate over all alive entities in the world.
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.archetypes
            .iter()
            .flat_map(|arch| arch.entities().iter().copied())
    }

    /// Find an entity by its raw index. Returns None if the entity is dead.
    pub fn find_entity_by_index(&self, index: u32) -> Option<Entity> {
        let loc = self.locations.get(index as usize)?.as_ref()?;
        let arch = &self.archetypes[loc.archetype_id.raw() as usize];
        Some(arch.entities()[loc.row])
    }

    /// Number of archetypes.
    #[must_use]
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// Iterate over all archetypes whose component set is a superset of
    /// `required` and contains none of `excluded`.
    pub fn query_archetypes<'w>(
        &'w self,
        required: &ComponentSet,
        excluded: &ComponentSet,
    ) -> impl Iterator<Item = &'w Archetype> {
        self.archetypes.iter().filter(move |arch| {
            arch.component_set().contains_all(required)
                && (excluded.is_empty() || arch.component_set().contains_none(excluded))
                && !arch.is_empty()
        })
    }

    /// Mutable archetype iteration for queries that need write access.
    pub fn query_archetypes_mut<'w>(
        &'w mut self,
        required: &ComponentSet,
        excluded: &ComponentSet,
    ) -> impl Iterator<Item = &'w mut Archetype> {
        self.archetypes.iter_mut().filter(move |arch| {
            arch.component_set().contains_all(required)
                && (excluded.is_empty() || arch.component_set().contains_none(excluded))
                && !arch.is_empty()
        })
    }

    // ── Internal helpers ────────────────────────────────────────────

    /// Resolve a live entity's location, or fail with [`EcsError::DeadEntity`].
    fn location_of(&self, entity: Entity) -> Result<EntityLocation, EcsError> {
        if !self.entities.is_alive(entity) {
            return Err(EcsError::DeadEntity);
        }
        self.locations
            .get(entity.index() as usize)
            .copied()
            .flatten()
            .ok_or(EcsError::DeadEntity)
    }

    /// After a swap-remove moved some other entity into `row`, record its
    /// new row. Forgetting this is how an archetype migration corrupts an
    /// unrelated entity, so it is one call rather than repeated inline.
    fn patch_swapped_location(&mut self, swapped: Option<Entity>, row: usize) {
        if let Some(swapped) = swapped {
            if let Some(Some(loc)) = self.locations.get_mut(swapped.index() as usize) {
                loc.row = row;
            }
        }
    }

    /// Find or create an archetype for the given component set.
    fn get_or_create_archetype(
        &mut self,
        set: &ComponentSet,
        infos: &[ComponentInfo],
    ) -> ArchetypeId {
        if let Some(&id) = self.archetype_map.get(set) {
            return id;
        }

        let id =
            ArchetypeId(u32::try_from(self.archetypes.len()).expect("archetype count overflow"));

        // Build infos ordered by the sorted component set.
        let mut ordered_infos = Vec::with_capacity(set.len());
        for comp_id in set.iter() {
            let info = infos
                .iter()
                .find(|i| i.id == comp_id)
                .expect("component info missing for ID in set")
                .clone();
            ordered_infos.push(info);
        }

        self.archetypes
            .push(Archetype::new(id, set.clone(), ordered_infos));
        self.archetype_map.insert(set.clone(), id);
        id
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("World")
            .field("entity_count", &self.entity_count())
            .field("archetype_count", &self.archetype_count())
            .finish()
    }
}

use std::fmt;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Pos {
        x: f32,
        y: f32,
    }
    impl Component for Pos {}

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Vel {
        dx: f32,
        dy: f32,
    }
    impl Component for Vel {}

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Health(f32);
    impl Component for Health {}

    #[test]
    fn spawn_and_get() {
        let mut world = World::new();
        let e = world.spawn((Pos { x: 1.0, y: 2.0 }, Vel { dx: 3.0, dy: 4.0 }));
        assert!(world.is_alive(e));
        assert_eq!(world.get::<Pos>(e), Some(&Pos { x: 1.0, y: 2.0 }));
        assert_eq!(world.get::<Vel>(e), Some(&Vel { dx: 3.0, dy: 4.0 }));
        assert_eq!(world.get::<Health>(e), None); // not present
    }

    #[test]
    fn despawn_removes_entity() {
        let mut world = World::new();
        let e = world.spawn((Pos { x: 0.0, y: 0.0 },));
        assert!(world.despawn(e));
        assert!(!world.is_alive(e));
        assert_eq!(world.get::<Pos>(e), None);
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn same_component_set_shares_archetype() {
        let mut world = World::new();
        let _e1 = world.spawn((Pos { x: 0.0, y: 0.0 }, Vel { dx: 0.0, dy: 0.0 }));
        let _e2 = world.spawn((Pos { x: 1.0, y: 1.0 }, Vel { dx: 1.0, dy: 1.0 }));
        assert_eq!(world.archetype_count(), 1);
    }

    #[test]
    fn different_component_sets_different_archetypes() {
        let mut world = World::new();
        let _e1 = world.spawn((Pos { x: 0.0, y: 0.0 },));
        let _e2 = world.spawn((Pos { x: 0.0, y: 0.0 }, Vel { dx: 0.0, dy: 0.0 }));
        assert_eq!(world.archetype_count(), 2);
    }

    #[test]
    fn get_mut_modifies_component() {
        let mut world = World::new();
        let e = world.spawn((Pos { x: 0.0, y: 0.0 },));
        if let Some(pos) = world.get_mut::<Pos>(e) {
            pos.x = 42.0;
        }
        assert_eq!(world.get::<Pos>(e).unwrap().x, 42.0);
    }

    #[test]
    fn query_archetypes_filters_correctly() {
        let mut world = World::new();
        world.spawn((Pos { x: 0.0, y: 0.0 },));
        world.spawn((Pos { x: 1.0, y: 1.0 }, Vel { dx: 0.0, dy: 0.0 }));

        let required = ComponentSet::from_ids(vec![ComponentId::of::<Pos>()]);
        let excluded = ComponentSet::empty();

        // Both archetypes have Pos, so both should match.
        let count = world.query_archetypes(&required, &excluded).count();
        assert_eq!(count, 2);

        // Only the (Pos, Vel) archetype has Vel.
        let required_pv =
            ComponentSet::from_ids(vec![ComponentId::of::<Pos>(), ComponentId::of::<Vel>()]);
        let count = world.query_archetypes(&required_pv, &excluded).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn despawn_with_swap_preserves_other_entities() {
        let mut world = World::new();
        let e0 = world.spawn((Pos { x: 0.0, y: 0.0 },));
        let e1 = world.spawn((Pos { x: 1.0, y: 1.0 },));
        let e2 = world.spawn((Pos { x: 2.0, y: 2.0 },));

        // Despawn e0 — e2 should swap into row 0.
        world.despawn(e0);
        assert_eq!(world.entity_count(), 2);
        assert!(world.is_alive(e1));
        assert!(world.is_alive(e2));
        assert_eq!(world.get::<Pos>(e1).unwrap().x, 1.0);
        assert_eq!(world.get::<Pos>(e2).unwrap().x, 2.0);
    }

    #[test]
    fn spawn_many_entities() {
        let mut world = World::new();
        let mut entities = Vec::new();
        for i in 0..1000 {
            let e = world.spawn((Pos {
                x: i as f32,
                y: 0.0,
            },));
            entities.push(e);
        }
        assert_eq!(world.entity_count(), 1000);
        assert_eq!(world.get::<Pos>(entities[999]).unwrap().x, 999.0);
    }
}
