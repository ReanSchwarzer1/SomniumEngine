//! Where the objectives are, over the game's view: a mark above each one on screen, and a pointer at the screen's
//! edge, turned toward it, for each one off it.
//!
//! The game sends its camera ([`ObjectiveMarkersMessage::SetView`]: world to clip, as the renderer draws, and the
//! player's position for the distance) and the targets (from `Story::markers`, placed in the world by
//! `MarkerDef::world_position`). An isometric camera's orthographic projection and a perspective one both work; a
//! target behind a perspective camera points the other way. Distances are game metres.
//!
//! The look is restrained on purpose: one colour, thin marks drawn in VectorCraft (`assets/story`), the tracked
//! objective a little brighter, nothing that pulses. A game can hand in its own marks
//! ([`ObjectiveMarkersMessage::SetGlyphs`]). Markers never take the pointer.

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    vector_glyph::{OBJECTIVE_MARK_SVG, OBJECTIVE_POINTER_SVG, VectorGlyph},
    widget::{Widget, WidgetBuilder},
};
use glam::{Mat4, Vec2, Vec3};

/// One objective to mark.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectiveTarget {
    /// The game's name for it (`quest/objective`).
    pub id: String,
    /// Where it is, in world metres.
    pub world: Vec3,
    /// The objective the journal tracks: drawn a little brighter, and the only one with its distance.
    pub tracked: bool,
}

/// Messages to the markers.
#[derive(Clone, Debug)]
pub enum ObjectiveMarkersMessage {
    /// The camera this frame: world to clip space, and where the player stands.
    SetView {
        view_proj: Mat4,
        player: Vec3,
    },
    SetTargets(Vec<ObjectiveTarget>),
    /// The game's own marks: SVG for the mark (its anchor is the bottom centre) and for the pointer (facing +x,
    /// anchored at its right edge's middle). `None` keeps the engine's.
    SetGlyphs {
        mark: Option<String>,
        pointer: Option<String>,
    },
    /// The marks' colour.
    SetColor([u8; 4]),
}

/// Where a target goes on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// On screen, at this point.
    Over(Vec2),
    /// Off screen: a pointer at this point on the inset edge, facing `angle` (radians, 0 is +x, y down).
    Edge { at: Vec2, angle: f32 },
}

/// Project `world` through `view_proj` onto `bounds`; anything outside `bounds` shrunk by `inset` goes to its edge.
#[must_use]
pub fn place(view_proj: Mat4, world: Vec3, bounds: Rect, inset: f32) -> Placement {
    let clip = view_proj * world.extend(1.0);
    let centre = bounds.pos() + bounds.size() * 0.5;
    let half = (bounds.size() * 0.5 - Vec2::splat(inset)).max(Vec2::splat(1.0));
    // Behind a perspective camera w is negative, and dividing by it would mirror the target across the view;
    // dividing by its size keeps left on the left.
    let behind = clip.w <= 1e-5;
    let w = clip.w.abs().max(1e-5);
    let ndc = Vec2::new(clip.x / w, clip.y / w);
    // NDC y is up; the screen's is down.
    let offset = Vec2::new(ndc.x, -ndc.y) * bounds.size() * 0.5;
    if !behind && offset.x.abs() <= half.x && offset.y.abs() <= half.y {
        return Placement::Over(centre + offset);
    }
    let direction = if offset.length_squared() > 1e-6 {
        offset
    } else {
        Vec2::Y
    };
    let reach = (half.x / direction.x.abs().max(1e-6)).min(half.y / direction.y.abs().max(1e-6));
    Placement::Edge {
        at: centre + direction * reach,
        angle: direction.y.atan2(direction.x),
    }
}

/// `42 m`, `1.2 km`: game metres as a reader wants them.
#[must_use]
pub fn distance_label(metres: f32) -> String {
    if metres < 1000.0 {
        format!("{} m", (metres / 5.0).round() as i64 * 5)
    } else {
        format!("{:.1} km", metres / 1000.0)
    }
}

/// The widget.
pub struct ObjectiveMarkers {
    font_id: u8,
    view_proj: Mat4,
    player: Vec3,
    has_view: bool,
    targets: Vec<ObjectiveTarget>,
    mark: VectorGlyph,
    pointer: VectorGlyph,
    color: Option<[u8; 4]>,
}

impl ObjectiveMarkers {
    fn unit(bounds: Rect) -> f32 {
        (bounds.h / 1080.0).max(0.25)
    }
}

impl Control for ObjectiveMarkers {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        if !self.has_view {
            return;
        }
        let bounds = widget.screen_bounds();
        let unit = Self::unit(bounds);
        let base = self
            .color
            .unwrap_or_else(|| theme::active().semantic.text.primary.bytes());
        let text_px = 15.0 * unit;
        for target in &self.targets {
            let color = theme::with_alpha(base, if target.tracked { 235 } else { 150 });
            let distance = target
                .tracked
                .then(|| distance_label(target.world.distance(self.player)));
            match place(self.view_proj, target.world, bounds, 40.0 * unit) {
                Placement::Over(at) => {
                    let px = if target.tracked { 34.0 } else { 26.0 } * unit;
                    let tip = Vec2::new(self.mark.size().x * 0.5, self.mark.size().y * 0.89);
                    self.mark.draw(ctx, tip, at, px, 0.0, color);
                    if let Some(label) = distance {
                        let w = ctx.font_atlas.measure_text(&label, text_px, self.font_id).x;
                        ctx.push_text(
                            &label,
                            Vec2::new(at.x - w * 0.5, at.y + 6.0 * unit),
                            self.font_id,
                            text_px,
                            color,
                        );
                    }
                }
                Placement::Edge { at, angle } => {
                    let px = 28.0 * unit;
                    let tip = Vec2::new(self.pointer.size().x * 0.69, self.pointer.size().y * 0.5);
                    self.pointer.draw(ctx, tip, at, px, angle, color);
                    if let Some(label) = distance {
                        // Inward of the pointer, so the edge never cuts the number.
                        let inward = -Vec2::from_angle(angle) * 30.0 * unit;
                        let size = ctx.font_atlas.measure_text(&label, text_px, self.font_id);
                        let origin = at + inward - size * 0.5;
                        ctx.push_text(&label, origin, self.font_id, text_px, color);
                    }
                }
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        if msg.direction != MessageDirection::ToWidget {
            return;
        }
        let Some(message) = msg.data::<ObjectiveMarkersMessage>().cloned() else {
            return;
        };
        match message {
            ObjectiveMarkersMessage::SetView { view_proj, player } => {
                self.view_proj = view_proj;
                self.player = player;
                self.has_view = true;
            }
            ObjectiveMarkersMessage::SetTargets(targets) => self.targets = targets,
            ObjectiveMarkersMessage::SetGlyphs { mark, pointer } => {
                if let Some(glyph) = mark.and_then(|svg| VectorGlyph::parse(&svg).ok()) {
                    self.mark = glyph;
                }
                if let Some(glyph) = pointer.and_then(|svg| VectorGlyph::parse(&svg).ok()) {
                    self.pointer = glyph;
                }
            }
            ObjectiveMarkersMessage::SetColor(color) => self.color = Some(color),
        }
        msg.handled = true;
    }
}

/// Builds [`ObjectiveMarkers`], normally stretched over the game's view.
pub struct ObjectiveMarkersBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl ObjectiveMarkersBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, font_id: 0 }
    }

    #[must_use]
    pub fn with_font(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        let mut widget = self.widget.with_background(theme::TRANSPARENT).build();
        widget.hit_self = false;
        UiNode::new(widget, Box::new(Self::markers(self.font_id)))
    }

    fn markers(font_id: u8) -> ObjectiveMarkers {
        ObjectiveMarkers {
            font_id,
            view_proj: Mat4::IDENTITY,
            player: Vec3::ZERO,
            has_view: false,
            targets: Vec::new(),
            mark: VectorGlyph::parse(OBJECTIVE_MARK_SVG).expect("the shipped mark parses"),
            pointer: VectorGlyph::parse(OBJECTIVE_POINTER_SVG).expect("the shipped pointer parses"),
            color: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso() -> Mat4 {
        // An isometric rig: orthographic, looking down at 30 degrees from the south-east, 40 m across.
        let eye = Vec3::new(30.0, 30.0, 30.0);
        Mat4::orthographic_rh(-20.0, 20.0, -11.25, 11.25, 0.1, 200.0)
            * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y)
    }

    #[test]
    fn targets_on_screen_are_marked_where_they_are_and_off_screen_ones_point_from_the_edge() {
        let bounds = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let Placement::Over(at) = place(iso(), Vec3::ZERO, bounds, 40.0) else {
            panic!("the look-at point is on screen")
        };
        assert!((at - Vec2::new(960.0, 540.0)).length() < 0.5, "{at}");
        // Far to the east (+x): off the right of an isometric view from the south-east.
        match place(iso(), Vec3::new(200.0, 0.0, -200.0), bounds, 40.0) {
            Placement::Edge { at, angle } => {
                assert!(
                    (at.x - 1880.0).abs() < 0.5,
                    "pinned to the inset right edge: {at}"
                );
                assert!(angle.abs() < 0.8, "facing roughly right: {angle}");
            }
            other => panic!("{other:?}"),
        }
        // Behind a perspective camera: the pointer faces away from where the projection would mirror it.
        let persp = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 500.0)
            * Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        match place(persp, Vec3::new(-5.0, 0.0, 10.0), bounds, 40.0) {
            Placement::Edge { at, .. } => {
                assert!(at.x < 960.0, "behind and to the left stays left: {at}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(distance_label(42.0), "40 m");
        assert_eq!(distance_label(1150.0), "1.1 km");
    }

    #[test]
    fn markers_take_their_view_and_targets_and_never_the_pointer() {
        let mut markers = ObjectiveMarkersBuilder::markers(0);
        let mut widget = WidgetBuilder::new().build();
        let mut send = |message: ObjectiveMarkersMessage| {
            let mut msg = UiMessage::new(widget.handle, MessageDirection::ToWidget, message);
            markers.handle_routed_message(&mut widget, &mut msg, &mut Vec::new());
            msg.handled
        };
        assert!(send(ObjectiveMarkersMessage::SetView {
            view_proj: iso(),
            player: Vec3::ONE
        }));
        assert!(send(ObjectiveMarkersMessage::SetTargets(vec![
            ObjectiveTarget {
                id: "relight_lamps/light".into(),
                world: Vec3::ZERO,
                tracked: true,
            }
        ])));
        assert!(send(ObjectiveMarkersMessage::SetGlyphs {
            mark: Some("not svg".into()),
            pointer: None
        }));
        assert!(markers.has_view && markers.targets.len() == 1);
        assert_eq!(
            markers.mark.size(),
            Vec2::new(24.0, 24.0),
            "a bad mark keeps the shipped one"
        );
        let built = ObjectiveMarkersBuilder::new(WidgetBuilder::new()).build();
        assert!(!built.widget.hit_self);
    }
}
