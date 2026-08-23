//! Editor drag-and-drop state and semantic drop contract.
//!
//! The state machine is deliberately independent of widgets: sources arm it,
//! targets publish an acceptance, and the shell emits exactly one
//! [`DropRequest`] when the pointer is released.

use glam::Vec2;
use somnium_asset::database::AssetId;
use somnium_ecs::Entity;
use somnium_ecs::reflect::{FieldId, StableId};
use std::path::PathBuf;

pub const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropEffect {
    Move,
    Copy,
    Link,
    Forbidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragPayload {
    Assets(Vec<AssetId>),
    Entities(Vec<Entity>),
    ExternalFiles(Vec<PathBuf>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum DropTarget {
    Viewport {
        entity: Option<Entity>,
        terrain_hit: Option<[f32; 3]>,
    },
    Outliner(Option<Entity>),
    AssetField {
        entity: Entity,
        component: StableId,
        field: FieldId,
        kind_mask: u64,
    },
    DrawerFolder(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropAcceptance {
    pub accepted: Vec<usize>,
    pub effect: DropEffect,
    pub reason: Option<String>,
    pub target: DropTarget,
}

impl DropAcceptance {
    #[must_use]
    pub fn rejected(target: DropTarget, reason: impl Into<String>) -> Self {
        Self {
            accepted: Vec::new(),
            effect: DropEffect::Forbidden,
            reason: Some(reason.into()),
            target,
        }
    }

    #[must_use]
    pub fn can_drop(&self) -> bool {
        !self.accepted.is_empty() && self.effect != DropEffect::Forbidden
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DropRequest {
    SpawnModels { assets: Vec<AssetId>, at: [f32; 3] },
    AssignMaterial { asset: AssetId, entities: Vec<Entity> },
    AttachScripts { assets: Vec<AssetId>, entity: Entity },
    SetAssetField {
        asset: AssetId,
        entity: Entity,
        component: StableId,
        field: FieldId,
    },
    LoadScene { asset: AssetId },
    Reparent { entities: Vec<Entity>, parent: Option<Entity> },
    ImportExternal { files: Vec<PathBuf>, folder: PathBuf },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedDrop {
    pub payload: DragPayload,
    pub acceptance: DropAcceptance,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Idle,
    Armed { origin: Vec2, payload: DragPayload },
    Dragging { payload: DragPayload, acceptance: Option<DropAcceptance> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DragDropState {
    state: State,
}

impl Default for DragDropState {
    fn default() -> Self { Self { state: State::Idle } }
}

impl DragDropState {
    pub fn arm(&mut self, origin: Vec2, payload: DragPayload) {
        self.state = State::Armed { origin, payload };
    }

    pub fn begin_external(&mut self, payload: DragPayload) {
        self.state = State::Dragging { payload, acceptance: None };
    }

    /// Returns true only on the frame the 4 logical-pixel threshold is crossed.
    pub fn pointer_moved(&mut self, position: Vec2) -> bool {
        let State::Armed { origin, payload } = &self.state else { return false };
        if position.distance(*origin) < DRAG_THRESHOLD { return false; }
        let payload = payload.clone();
        self.state = State::Dragging { payload, acceptance: None };
        true
    }

    pub fn set_acceptance(&mut self, acceptance: Option<DropAcceptance>) {
        if let State::Dragging { acceptance: current, .. } = &mut self.state {
            *current = acceptance;
        }
    }

    #[must_use]
    pub fn acceptance(&self) -> Option<&DropAcceptance> {
        match &self.state { State::Dragging { acceptance, .. } => acceptance.as_ref(), _ => None }
    }

    #[must_use]
    pub fn payload(&self) -> Option<&DragPayload> {
        match &self.state {
            State::Armed { payload, .. } | State::Dragging { payload, .. } => Some(payload),
            State::Idle => None,
        }
    }

    #[must_use]
    pub fn is_dragging(&self) -> bool { matches!(self.state, State::Dragging { .. }) }

    pub fn cancel(&mut self) -> bool {
        let active = !matches!(self.state, State::Idle);
        self.state = State::Idle;
        active
    }

    pub fn release(&mut self) -> Option<CompletedDrop> {
        let state = std::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Dragging { payload, acceptance: Some(acceptance) } if acceptance.can_drop() => {
                Some(CompletedDrop { payload, acceptance })
            }
            _ => None,
        }
    }
}

/// Walk a hit node toward the root and return the first registered target.
pub fn resolve_ancestor<T: Copy + Eq>(
    mut hit: T,
    none: T,
    mut parent: impl FnMut(T) -> T,
    mut accepts: impl FnMut(T) -> bool,
) -> Option<T> {
    while hit != none {
        if accepts(hit) { return Some(hit); }
        hit = parent(hit);
    }
    None
}

/// Convert an accepted UI drop into the editor's semantic route. This is the
/// only extension/kind dispatch table; widgets never infer authoring actions.
pub fn semantic_request(
    db: &somnium_asset::database::AssetDbSnapshot,
    payload: &DragPayload,
    acceptance: &DropAcceptance,
) -> Result<DropRequest, String> {
    let accepted_assets = || -> Vec<AssetId> {
        match payload {
            DragPayload::Assets(items) => acceptance.accepted.iter().filter_map(|i| items.get(*i).copied()).collect(),
            _ => Vec::new(),
        }
    };
    match (payload, &acceptance.target) {
        (DragPayload::ExternalFiles(files), DropTarget::DrawerFolder(folder)) => Ok(DropRequest::ImportExternal {
            files: acceptance.accepted.iter().filter_map(|i| files.get(*i).cloned()).collect(), folder: folder.clone(),
        }),
        (DragPayload::Entities(entities), DropTarget::Outliner(parent)) => Ok(DropRequest::Reparent {
            entities: acceptance.accepted.iter().filter_map(|i| entities.get(*i).copied()).collect(), parent: *parent,
        }),
        (DragPayload::Assets(_), DropTarget::AssetField { entity, component, field, kind_mask }) => {
            let asset = accepted_assets().into_iter().next().ok_or("No accepted texture")?;
            if db.get(asset).is_none_or(|r| r.kind.bit() & *kind_mask == 0) { return Err("This asset kind is not accepted by the field".into()); }
            Ok(DropRequest::SetAssetField { asset, entity: *entity, component: *component, field: *field })
        }
        (DragPayload::Assets(_), DropTarget::Outliner(Some(entity))) => {
            let assets = accepted_assets();
            let Some(first) = assets.first().copied() else { return Err("No accepted asset".into()); };
            match db.get(first).map(|r| r.kind) {
                Some(somnium_asset::database::AssetKind::Material) => Ok(DropRequest::AssignMaterial { asset: first, entities: vec![*entity] }),
                Some(somnium_asset::database::AssetKind::Script) => Ok(DropRequest::AttachScripts { assets, entity: *entity }),
                _ => Err("This asset cannot be dropped on an entity".into()),
            }
        }
        (DragPayload::Assets(_), DropTarget::Viewport { entity, terrain_hit }) => {
            let assets = accepted_assets();
            let Some(first) = assets.first().copied() else { return Err("No accepted asset".into()); };
            match db.get(first).map(|r| r.kind) {
                Some(somnium_asset::database::AssetKind::Mesh) => Ok(DropRequest::SpawnModels { assets, at: terrain_hit.ok_or("Point at terrain to place this model")? }),
                Some(somnium_asset::database::AssetKind::Material) => Ok(DropRequest::AssignMaterial { asset: first, entities: vec![entity.ok_or("Point at an entity to assign material")?] }),
                Some(somnium_asset::database::AssetKind::Script) => Ok(DropRequest::AttachScripts { assets, entity: entity.ok_or("Point at an entity to attach script")? }),
                Some(somnium_asset::database::AssetKind::Scene) => Ok(DropRequest::LoadScene { asset: first }),
                _ => Err("This asset cannot be dropped in the viewport".into()),
            }
        }
        _ => Err("Payload and target are incompatible".into()),
    }
}

#[must_use]
pub fn acceptance_for(
    db: &somnium_asset::database::AssetDbSnapshot,
    payload: &DragPayload,
    target: DropTarget,
) -> DropAcceptance {
    let total = match payload { DragPayload::Assets(v) => v.len(), DragPayload::Entities(v) => v.len(), DragPayload::ExternalFiles(v) => v.len() };
    let (accepted, effect) = match payload {
        DragPayload::ExternalFiles(_) if matches!(target, DropTarget::DrawerFolder(_)) => ((0..total).collect(), DropEffect::Copy),
        DragPayload::Entities(items) if matches!(target, DropTarget::Outliner(_)) => {
            let parent = match target { DropTarget::Outliner(p) => p, _ => None };
            ((0..items.len()).filter(|i| Some(items[*i]) != parent).collect(), DropEffect::Move)
        }
        DragPayload::Assets(items) => {
            let compatible = |kind: somnium_asset::database::AssetKind| match target {
                DropTarget::AssetField { kind_mask, .. } => kind.bit() & kind_mask != 0,
                DropTarget::Outliner(Some(_)) => matches!(kind, somnium_asset::database::AssetKind::Material | somnium_asset::database::AssetKind::Script),
                DropTarget::Viewport { entity, terrain_hit } => match kind {
                    somnium_asset::database::AssetKind::Mesh => terrain_hit.is_some(),
                    somnium_asset::database::AssetKind::Material | somnium_asset::database::AssetKind::Script => entity.is_some(),
                    somnium_asset::database::AssetKind::Scene => true,
                    _ => false,
                },
                _ => false,
            };
            let first_kind = items.iter().filter_map(|id| db.get(*id).map(|r| r.kind)).find(|k| compatible(*k));
            let accepted = first_kind.map_or_else(Vec::new, |first| items.iter().enumerate().filter_map(|(i, id)| {
                let kind = db.get(*id)?.kind;
                (compatible(kind) && kind == first && !(kind == somnium_asset::database::AssetKind::Material && i > 0)).then_some(i)
            }).collect());
            let effect = match first_kind { Some(somnium_asset::database::AssetKind::Mesh) => DropEffect::Copy, _ => DropEffect::Link };
            (accepted, effect)
        }
        _ => (Vec::new(), DropEffect::Forbidden),
    };
    if accepted.is_empty() { return DropAcceptance::rejected(target, "Payload is not accepted here"); }
    let reason = (accepted.len() != total).then(|| format!("{} of {} · {effect:?}", accepted.len(), total));
    DropAcceptance { accepted, effect, reason, target }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> DragPayload { DragPayload::ExternalFiles(vec![PathBuf::from("a.png")]) }
    fn accepted() -> DropAcceptance {
        DropAcceptance { accepted: vec![0], effect: DropEffect::Copy, reason: None,
            target: DropTarget::DrawerFolder(PathBuf::from("textures")) }
    }

    #[test]
    fn threshold_is_four_logical_pixels() {
        let mut d = DragDropState::default(); d.arm(Vec2::ZERO, payload());
        assert!(!d.pointer_moved(Vec2::new(3.99, 0.0)));
        assert!(d.pointer_moved(Vec2::new(4.0, 0.0)));
    }

    #[test]
    fn escape_cancel_prevents_completion() {
        let mut d = DragDropState::default(); d.arm(Vec2::ZERO, payload());
        d.pointer_moved(Vec2::new(5.0, 0.0)); d.set_acceptance(Some(accepted()));
        assert!(d.cancel()); assert!(d.release().is_none());
    }

    #[test]
    fn rejected_or_unarmed_release_never_completes() {
        let mut d = DragDropState::default();
        assert!(d.release().is_none());
        d.arm(Vec2::ZERO, payload()); d.pointer_moved(Vec2::new(5.0, 0.0));
        d.set_acceptance(Some(DropAcceptance::rejected(
            DropTarget::DrawerFolder(PathBuf::new()), "unsupported")));
        assert!(d.release().is_none());
    }

    #[test]
    fn partial_acceptance_preserves_reason_and_completes_once() {
        let mut d = DragDropState::default(); d.arm(Vec2::ZERO, payload());
        d.pointer_moved(Vec2::new(5.0, 0.0));
        let mut a = accepted(); a.reason = Some("1 unsupported item".into());
        d.set_acceptance(Some(a.clone()));
        assert_eq!(d.release().unwrap().acceptance, a);
        assert!(d.release().is_none());
    }

    fn route_fixture() -> (PathBuf, somnium_asset::database::AssetDbSnapshot) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "somnium_drag_routes_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("ship.glb"), b"glb").unwrap();
        std::fs::write(root.join("hull.glb"), b"glb").unwrap();
        std::fs::write(root.join("polished.sommat"), b"{}").unwrap();
        std::fs::write(root.join("spin.luau"), b"return {}").unwrap();
        std::fs::write(root.join("rock.png"), b"png").unwrap();
        std::fs::write(root.join("level.somnium"), b"{}").unwrap();
        let db = somnium_asset::database::AssetDb::scan(&root).unwrap();
        (root, db)
    }

    #[derive(Debug, Clone, Copy)]
    struct Marker;
    impl somnium_ecs::Component for Marker {}

    fn id(name: &str) -> AssetId {
        AssetId::from_relative_path(name)
    }

    /// Every route resolves the *same* request the highlight promised, and the
    /// one that cannot be satisfied says why rather than silently doing
    /// nothing. Cursor, adorner and execution all read this one function.
    #[test]
    fn seven_routes_resolve_to_exactly_one_semantic_request() {
        let (root, db) = route_fixture();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((Marker,));
        let drop = |payload: DragPayload, target: DropTarget| {
            let acceptance = acceptance_for(&db, &payload, target);
            semantic_request(&db, &payload, &acceptance)
        };

        // .glb into the viewport, at the terrain hit.
        assert_eq!(
            drop(
                DragPayload::Assets(vec![id("ship.glb"), id("hull.glb")]),
                DropTarget::Viewport { entity: None, terrain_hit: Some([1.0, 2.0, 3.0]) },
            ),
            Ok(DropRequest::SpawnModels {
                assets: vec![id("ship.glb"), id("hull.glb")],
                at: [1.0, 2.0, 3.0],
            })
        );

        // .sommat onto an Outliner row.
        assert_eq!(
            drop(
                DragPayload::Assets(vec![id("polished.sommat")]),
                DropTarget::Outliner(Some(entity)),
            ),
            Ok(DropRequest::AssignMaterial { asset: id("polished.sommat"), entities: vec![entity] })
        );

        // .luau onto a viewport entity. The hovered entity is the target, not
        // the current selection: the test names the distinction because the
        // bug it prevents is invisible whenever the two happen to agree.
        assert_eq!(
            drop(
                DragPayload::Assets(vec![id("spin.luau")]),
                DropTarget::Viewport { entity: Some(entity), terrain_hit: None },
            ),
            Ok(DropRequest::AttachScripts { assets: vec![id("spin.luau")], entity })
        );

        // A texture onto a material texture slot in Details.
        let field = DropTarget::AssetField {
            entity,
            component: StableId::new("somnium.asset.Material"),
            field: FieldId(3),
            kind_mask: somnium_asset::database::AssetKind::Texture.bit(),
        };
        assert_eq!(
            drop(DragPayload::Assets(vec![id("rock.png")]), field.clone()),
            Ok(DropRequest::SetAssetField {
                asset: id("rock.png"),
                entity,
                component: StableId::new("somnium.asset.Material"),
                field: FieldId(3),
            })
        );
        // The same slot refuses a mesh.
        assert!(drop(DragPayload::Assets(vec![id("ship.glb")]), field).is_err());

        // .somnium into the viewport.
        assert_eq!(
            drop(
                DragPayload::Assets(vec![id("level.somnium")]),
                DropTarget::Viewport { entity: None, terrain_hit: None },
            ),
            Ok(DropRequest::LoadScene { asset: id("level.somnium") })
        );

        // Outliner row onto Outliner row.
        let child = world.spawn((Marker,));
        assert_eq!(
            drop(DragPayload::Entities(vec![child]), DropTarget::Outliner(Some(entity))),
            Ok(DropRequest::Reparent { entities: vec![child], parent: Some(entity) })
        );

        // OS file onto a drawer folder.
        assert_eq!(
            drop(
                DragPayload::ExternalFiles(vec![PathBuf::from("C:/downloads/tree.png")]),
                DropTarget::DrawerFolder(PathBuf::from("textures")),
            ),
            Ok(DropRequest::ImportExternal {
                files: vec![PathBuf::from("C:/downloads/tree.png")],
                folder: PathBuf::from("textures"),
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A mesh needs terrain under the pointer, and saying so is the adorner's
    /// job. Rejecting before the button comes up is the whole point.
    #[test]
    fn mesh_without_a_terrain_hit_is_rejected_with_a_reason() {
        let (root, db) = route_fixture();
        let payload = DragPayload::Assets(vec![id("ship.glb")]);
        let target = DropTarget::Viewport { entity: None, terrain_hit: None };
        let acceptance = acceptance_for(&db, &payload, target);
        assert_eq!(acceptance.effect, DropEffect::Forbidden);
        assert!(!acceptance.can_drop());
        assert!(acceptance.reason.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Five assets onto a target that takes two drops two, and the count the
    /// adorner shows is the count the execution uses.
    #[test]
    fn partial_acceptance_reports_the_exact_subset_it_will_execute() {
        let (root, db) = route_fixture();
        let payload = DragPayload::Assets(vec![
            id("ship.glb"),
            id("hull.glb"),
            id("rock.png"),
            id("polished.sommat"),
            id("spin.luau"),
        ]);
        let acceptance = acceptance_for(
            &db,
            &payload,
            DropTarget::Viewport { entity: None, terrain_hit: Some([0.0; 3]) },
        );
        assert_eq!(acceptance.accepted, vec![0, 1]);
        assert_eq!(acceptance.effect, DropEffect::Copy);
        assert_eq!(acceptance.reason.as_deref(), Some("2 of 5 \u{b7} Copy"));
        let request = semantic_request(&db, &payload, &acceptance).unwrap();
        let DropRequest::SpawnModels { assets, .. } = request else {
            panic!("a mesh drop must spawn models");
        };
        assert_eq!(assets.len(), 2, "execution must use the advertised subset");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Reparenting an entity onto itself is a no-op the acceptance rejects, so
    /// no highlight ever promises it.
    #[test]
    fn self_reparent_is_rejected_before_the_highlight() {
        let (root, db) = route_fixture();
        let entity = somnium_ecs::World::new().spawn((Marker,));
        let acceptance = acceptance_for(
            &db,
            &DragPayload::Entities(vec![entity]),
            DropTarget::Outliner(Some(entity)),
        );
        assert!(!acceptance.can_drop());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ancestor_resolution_uses_nearest_registered_parent() {
        let parents = [0, 0, 1, 2, 3];
        assert_eq!(resolve_ancestor(4, 0, |n| parents[n], |n| n == 2), Some(2));
    }
}
