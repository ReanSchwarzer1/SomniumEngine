//! The editor selection.
//!
//! CONTROL-F turns `Option<Entity>` into an ordered set with a *primary*. The
//! primary stays a plain `Option<Entity>` field so the sixty-odd existing
//! single-selection call sites — the gizmo pivot, the outline pass, the script
//! bridge, every `EditorCommand` signature — keep compiling and keep meaning
//! exactly what they meant before. Multi-selection is additive: it lives in
//! `entities`, and `reconcile` is what stops the two halves drifting apart
//! when something writes the primary directly.

use somnium_ecs::{Entity, World};

/// An ordered, deduplicated selection with a designated primary.
///
/// Invariant, restored by [`Selection::reconcile`]: `primary` is `None` exactly
/// when `entities` is empty, and otherwise `entities` contains `primary`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Selection {
    entities: Vec<Entity>,
    /// The gizmo pivot and the Details subject.
    ///
    /// Public because commands and game code take `&mut Option<Entity>`; every
    /// such write is followed by [`Selection::reconcile`].
    pub primary: Option<Entity>,
    /// Anchor for `Shift`-extended range selection in the Outliner. Cleared by
    /// anything that replaces the selection outright.
    anchor: Option<Entity>,
}

impl Selection {
    /// A one-entity selection: the primary, the set and the anchor all agree.
    #[must_use]
    pub fn single(entity: Entity) -> Self {
        Self {
            entities: vec![entity],
            primary: Some(entity),
            anchor: Some(entity),
        }
    }

    /// Everything selected, in the order it was selected.
    #[must_use]
    pub fn as_slice(&self) -> &[Entity] {
        &self.entities
    }

    /// How many entities are selected.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Whether `entity` is part of the selection.
    #[must_use]
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(&entity)
    }

    /// The `Shift`-range origin, if one has been established.
    #[must_use]
    pub fn anchor(&self) -> Option<Entity> {
        self.anchor
    }

    /// Replace the selection with one entity, or clear it.
    pub fn set_single(&mut self, entity: Option<Entity>) {
        self.entities = entity.into_iter().collect();
        self.primary = entity;
        self.anchor = entity;
    }

    /// `command()`-click: add or remove one entity, keeping the rest.
    ///
    /// Removing the primary promotes the most recently added survivor rather
    /// than clearing, because a `command()`-click is a refinement, not a reset.
    pub fn toggle(&mut self, entity: Entity) {
        if let Some(position) = self.entities.iter().position(|e| *e == entity) {
            self.entities.remove(position);
            if self.primary == Some(entity) {
                self.primary = self.entities.last().copied();
            }
        } else {
            self.entities.push(entity);
            self.primary = Some(entity);
        }
        self.anchor = Some(entity);
        self.reconcile_anchor();
    }

    /// `Shift`-click: select the inclusive range between the anchor and
    /// `entity` within `ordered`, which is the Outliner's flattened row order.
    /// The clicked row becomes the primary; the anchor is deliberately left
    /// alone so a second `Shift`-click re-extends from the same origin.
    pub fn extend_range(&mut self, ordered: &[Entity], entity: Entity) {
        let Some(anchor) = self.anchor.or(self.primary) else {
            self.set_single(Some(entity));
            return;
        };
        let (Some(from), Some(to)) = (
            ordered.iter().position(|e| *e == anchor),
            ordered.iter().position(|e| *e == entity),
        ) else {
            self.set_single(Some(entity));
            return;
        };
        let range = if from <= to { from..=to } else { to..=from };
        self.entities = ordered[range].to_vec();
        self.primary = Some(entity);
    }

    /// Deselect everything.
    pub fn clear(&mut self) {
        self.entities.clear();
        self.primary = None;
        self.anchor = None;
    }

    /// Replace the whole selection, preserving order and dropping duplicates.
    /// The last entity becomes the primary — marquee and paste both end on the
    /// thing the user most recently caused to exist or touch.
    pub fn set_many(&mut self, entities: impl IntoIterator<Item = Entity>) {
        self.entities.clear();
        for entity in entities {
            if !self.entities.contains(&entity) {
                self.entities.push(entity);
            }
        }
        self.primary = self.entities.last().copied();
        self.anchor = self.primary;
    }

    /// Restore the invariant after something wrote [`Selection::primary`]
    /// directly — an undo that respawned an entity, a command that selected
    /// what it created, or game code holding the `&mut Option<Entity>`.
    ///
    /// A primary that is no longer a member means the selection was replaced
    /// by a single-selection call site, so the set collapses to match it. That
    /// is the honest reading: the old members were never told they survived.
    pub fn reconcile(&mut self) {
        match self.primary {
            None => {
                self.entities.clear();
                self.anchor = None;
            }
            Some(primary) if !self.entities.contains(&primary) => {
                self.entities = vec![primary];
                self.anchor = Some(primary);
            }
            Some(_) => self.reconcile_anchor(),
        }
    }

    fn reconcile_anchor(&mut self) {
        if self.primary.is_none() {
            self.entities.clear();
            self.anchor = None;
        } else if self.anchor.is_some_and(|a| !self.entities.contains(&a)) {
            self.anchor = self.primary;
        }
    }

    /// Drop entities the world no longer has. Called after deletes, scene
    /// loads and undo, so a stale handle can never reach a command.
    pub fn retain_alive(&mut self, world: &World) {
        self.entities.retain(|entity| world.is_alive(*entity));
        if self.primary.is_some_and(|e| !world.is_alive(e)) {
            self.primary = self.entities.last().copied();
        }
        self.reconcile_anchor();
    }
}

/// A viewport rubber-band, in logical viewport pixels.
///
/// Kept here rather than in `app.rs` so the hit rule — a world point is caught
/// when its projection lands inside the rectangle, and points behind the camera
/// never are — can be tested without a window, a renderer or a GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marquee {
    /// Where the drag started.
    pub origin: (f32, f32),
    /// Where the pointer is now.
    pub current: (f32, f32),
}

impl Marquee {
    /// Start a rubber-band at one corner.
    #[must_use]
    pub fn new(origin: (f32, f32)) -> Self {
        Self {
            origin,
            current: origin,
        }
    }

    /// `(x, y, width, height)`, normalised so dragging up-left works.
    #[must_use]
    pub fn rect(&self) -> (f32, f32, f32, f32) {
        let x = self.origin.0.min(self.current.0);
        let y = self.origin.1.min(self.current.1);
        (
            x,
            y,
            (self.origin.0 - self.current.0).abs(),
            (self.origin.1 - self.current.1).abs(),
        )
    }

    /// Whether the band has been dragged far enough to mean a marquee rather
    /// than a click. Shares CONTROL-E's four-pixel threshold on purpose: one
    /// number for "the pointer moved deliberately".
    #[must_use]
    pub fn is_dragged(&self) -> bool {
        let (_, _, w, h) = self.rect();
        w.max(h) >= 4.0
    }

    /// Whether a projected point falls inside the band. `ndc` is the point in
    /// clip space after the perspective divide; `w` is the pre-divide `w`, and
    /// a non-positive `w` means the point is behind the camera.
    #[must_use]
    pub fn contains_ndc(&self, ndc: glam::Vec3, w: f32, viewport: (f32, f32)) -> bool {
        if w <= 0.0 {
            return false;
        }
        let x = (ndc.x * 0.5 + 0.5) * viewport.0;
        let y = (0.5 - ndc.y * 0.5) * viewport.1;
        let (rx, ry, rw, rh) = self.rect();
        x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    struct Marker;
    impl somnium_ecs::Component for Marker {}

    fn world_with(count: usize) -> (World, Vec<Entity>) {
        let mut world = World::new();
        let entities = (0..count).map(|_| world.spawn((Marker,))).collect();
        (world, entities)
    }

    /// The shim's whole point: single-selection call sites write the primary
    /// and the set follows, rather than the two silently disagreeing.
    #[test]
    fn writing_the_primary_directly_collapses_the_set() {
        let (_world, e) = world_with(3);
        let mut selection = Selection::default();
        selection.set_many(e.iter().copied());
        assert_eq!(selection.len(), 3);

        selection.primary = Some(e[0]);
        selection.reconcile();
        assert_eq!(selection.len(), 3, "an existing member keeps the set");

        selection.primary = None;
        selection.reconcile();
        assert!(selection.is_empty(), "clearing the primary clears the set");
    }

    #[test]
    fn toggle_adds_removes_and_promotes_a_new_primary() {
        let (_world, e) = world_with(3);
        let mut selection = Selection::single(e[0]);
        selection.toggle(e[1]);
        selection.toggle(e[2]);
        assert_eq!(selection.as_slice(), &e[..]);
        assert_eq!(selection.primary, Some(e[2]));

        selection.toggle(e[2]);
        assert_eq!(selection.as_slice(), &e[..2]);
        assert_eq!(
            selection.primary,
            Some(e[1]),
            "removing the primary promotes a survivor, it does not clear"
        );
    }

    #[test]
    fn shift_extends_an_inclusive_range_in_row_order_from_a_stable_anchor() {
        let (_world, e) = world_with(5);
        let mut selection = Selection::single(e[3]);
        selection.extend_range(&e, e[1]);
        assert_eq!(selection.as_slice(), &e[1..=3]);
        assert_eq!(selection.primary, Some(e[1]));

        // The anchor is unchanged, so re-extending grows from the same origin.
        selection.extend_range(&e, e[4]);
        assert_eq!(selection.as_slice(), &e[3..=4]);
    }

    #[test]
    fn a_dead_entity_never_survives_in_the_selection() {
        let (mut world, e) = world_with(3);
        let mut selection = Selection::default();
        selection.set_many(e.iter().copied());
        world.despawn(e[2]);
        selection.retain_alive(&world);
        assert_eq!(selection.as_slice(), &e[..2]);
        assert_eq!(selection.primary, Some(e[1]));
    }

    /// The band normalises, so dragging up-and-left selects the same box as
    /// dragging down-and-right.
    #[test]
    fn a_marquee_normalises_whichever_way_it_is_dragged() {
        let mut down_right = Marquee::new((10.0, 10.0));
        down_right.current = (50.0, 40.0);
        let mut up_left = Marquee::new((50.0, 40.0));
        up_left.current = (10.0, 10.0);
        assert_eq!(down_right.rect(), up_left.rect());
        assert_eq!(down_right.rect(), (10.0, 10.0, 40.0, 30.0));
    }

    /// A click is not a marquee. Same four pixels CONTROL-E's drag uses.
    #[test]
    fn a_click_sized_band_is_not_a_drag() {
        let mut band = Marquee::new((0.0, 0.0));
        band.current = (3.0, 3.0);
        assert!(!band.is_dragged());
        band.current = (4.0, 0.0);
        assert!(band.is_dragged());
    }

    /// A point behind the camera projects to a mirrored position in front of
    /// it. Testing `w` is what stops a marquee in the top-left corner
    /// selecting everything behind the viewer.
    #[test]
    fn points_behind_the_camera_are_never_caught() {
        let mut band = Marquee::new((0.0, 0.0));
        band.current = (800.0, 600.0);
        let viewport = (800.0, 600.0);
        assert!(band.contains_ndc(glam::Vec3::ZERO, 1.0, viewport));
        assert!(!band.contains_ndc(glam::Vec3::ZERO, -1.0, viewport));
        assert!(!band.contains_ndc(glam::Vec3::ZERO, 0.0, viewport));
    }

    #[test]
    fn the_band_catches_only_what_projects_inside_it() {
        let mut band = Marquee::new((0.0, 0.0));
        band.current = (400.0, 300.0);
        let viewport = (800.0, 600.0);
        // Centre of the screen is (400, 300) — the far corner, inclusive.
        assert!(band.contains_ndc(glam::Vec3::ZERO, 1.0, viewport));
        // Right of centre projects past the band.
        assert!(!band.contains_ndc(glam::Vec3::new(0.5, 0.0, 0.0), 1.0, viewport));
    }

    #[test]
    fn set_many_deduplicates_and_keeps_order() {
        let (_world, e) = world_with(3);
        let mut selection = Selection::default();
        selection.set_many([e[2], e[0], e[2], e[1]]);
        assert_eq!(selection.as_slice(), &[e[2], e[0], e[1]]);
        assert_eq!(selection.primary, Some(e[1]));
    }
}
