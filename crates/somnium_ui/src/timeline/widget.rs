//! Retained timeline control with an embedded CONTROL-K curve editor.

use super::{
    GroupId, MarkerId, MediaId, TimelineCatalogue, TimelineDocument, TimelineError,
    TimelineSurface, TrackId,
};
use crate::draw::DrawingContext;
use crate::message::{
    MessageDirection, MouseButton, NodeHandle, UiMessage, WHEEL_DELTA_PER_LINE, WidgetMessage,
};
use crate::node::{Control, CursorKind, LayoutCtx, UiNode};
use crate::types::Rect;
use crate::ui::UserInterface;
use crate::widget::{Widget, WidgetBuilder};
use crate::widgets::curve_editor::{CurveEditorBuilder, CurveEditorMessage};
use glam::Vec2;
use somnium_ecs::curve::{Curve, CurveKey};

const LABEL_WIDTH: f32 = 168.0;
const RULER_HEIGHT: f32 = 30.0;
const GROUP_HEIGHT: f32 = 24.0;
const TRACK_HEIGHT: f32 = 34.0;
const CURVE_HEIGHT: f32 = 118.0;

#[derive(Clone)]
pub enum TimelineEditorMessage {
    SetDocument(TimelineDocument),
    AddGroup {
        title: String,
        parent: Option<GroupId>,
    },
    AddTrack {
        archetype: String,
        title: String,
        group: Option<GroupId>,
    },
    AddMedia {
        track: TrackId,
        kind: String,
        source: String,
        start: f32,
        duration: f32,
    },
    MoveMedia {
        media: MediaId,
        start: f32,
    },
    ResizeMedia {
        media: MediaId,
        start: f32,
        duration: f32,
    },
    AddMarker {
        time: f32,
        label: String,
    },
    MoveMarker {
        marker: MarkerId,
        time: f32,
    },
    AddKeyframe {
        track: TrackId,
        channel: usize,
        key: CurveKey,
    },
    MoveKeyframe {
        track: TrackId,
        channel: usize,
        key: usize,
        time: f32,
    },
    RemoveTrack(TrackId),
    SelectChannel {
        track: TrackId,
        channel: usize,
    },
    SetSelectedCurve {
        curve: Curve,
        live: bool,
    },
    SetPlayhead(f32),
    SetSnap(f32),
    Zoom {
        factor: f32,
        anchor_time: f32,
    },
    Command(String),
    #[doc(hidden)]
    BindCurveEditor(NodeHandle),
    Changed(TimelineDocument),
    PlayheadChanged(f32),
}

impl TimelineEditorMessage {
    fn to(destination: NodeHandle, data: Self) -> UiMessage {
        UiMessage::new(destination, MessageDirection::ToWidget, data)
    }

    #[must_use]
    pub fn set_document(destination: NodeHandle, document: TimelineDocument) -> UiMessage {
        Self::to(destination, Self::SetDocument(document))
    }

    #[must_use]
    pub fn add_group(
        destination: NodeHandle,
        title: impl Into<String>,
        parent: Option<GroupId>,
    ) -> UiMessage {
        Self::to(
            destination,
            Self::AddGroup {
                title: title.into(),
                parent,
            },
        )
    }

    #[must_use]
    pub fn add_track(
        destination: NodeHandle,
        archetype: impl Into<String>,
        title: impl Into<String>,
        group: Option<GroupId>,
    ) -> UiMessage {
        Self::to(
            destination,
            Self::AddTrack {
                archetype: archetype.into(),
                title: title.into(),
                group,
            },
        )
    }

    #[must_use]
    pub fn add_media(
        destination: NodeHandle,
        track: TrackId,
        kind: impl Into<String>,
        source: impl Into<String>,
        start: f32,
        duration: f32,
    ) -> UiMessage {
        Self::to(
            destination,
            Self::AddMedia {
                track,
                kind: kind.into(),
                source: source.into(),
                start,
                duration,
            },
        )
    }

    #[must_use]
    pub fn move_media(destination: NodeHandle, media: MediaId, start: f32) -> UiMessage {
        Self::to(destination, Self::MoveMedia { media, start })
    }

    #[must_use]
    pub fn resize_media(
        destination: NodeHandle,
        media: MediaId,
        start: f32,
        duration: f32,
    ) -> UiMessage {
        Self::to(
            destination,
            Self::ResizeMedia {
                media,
                start,
                duration,
            },
        )
    }

    #[must_use]
    pub fn add_marker(destination: NodeHandle, time: f32, label: impl Into<String>) -> UiMessage {
        Self::to(
            destination,
            Self::AddMarker {
                time,
                label: label.into(),
            },
        )
    }

    #[must_use]
    pub fn move_marker(destination: NodeHandle, marker: MarkerId, time: f32) -> UiMessage {
        Self::to(destination, Self::MoveMarker { marker, time })
    }

    #[must_use]
    pub fn add_keyframe(
        destination: NodeHandle,
        track: TrackId,
        channel: usize,
        key: CurveKey,
    ) -> UiMessage {
        Self::to(
            destination,
            Self::AddKeyframe {
                track,
                channel,
                key,
            },
        )
    }

    #[must_use]
    pub fn move_keyframe(
        destination: NodeHandle,
        track: TrackId,
        channel: usize,
        key: usize,
        time: f32,
    ) -> UiMessage {
        Self::to(
            destination,
            Self::MoveKeyframe {
                track,
                channel,
                key,
                time,
            },
        )
    }

    #[must_use]
    pub fn remove_track(destination: NodeHandle, track: TrackId) -> UiMessage {
        Self::to(destination, Self::RemoveTrack(track))
    }

    #[must_use]
    pub fn select_channel(destination: NodeHandle, track: TrackId, channel: usize) -> UiMessage {
        Self::to(destination, Self::SelectChannel { track, channel })
    }

    #[must_use]
    pub fn set_selected_curve(destination: NodeHandle, curve: Curve, live: bool) -> UiMessage {
        Self::to(destination, Self::SetSelectedCurve { curve, live })
    }

    #[must_use]
    pub fn set_playhead(destination: NodeHandle, time: f32) -> UiMessage {
        Self::to(destination, Self::SetPlayhead(time))
    }

    #[must_use]
    pub fn set_snap(destination: NodeHandle, seconds: f32) -> UiMessage {
        Self::to(destination, Self::SetSnap(seconds))
    }

    #[must_use]
    pub fn zoom(destination: NodeHandle, factor: f32, anchor_time: f32) -> UiMessage {
        Self::to(
            destination,
            Self::Zoom {
                factor,
                anchor_time,
            },
        )
    }

    #[must_use]
    pub fn command(destination: NodeHandle, id: impl Into<String>) -> UiMessage {
        Self::to(destination, Self::Command(id.into()))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TimelineEditorHandles {
    pub editor: NodeHandle,
    pub curve_editor: NodeHandle,
}

enum Gesture {
    None,
    Scrub,
    Pan { last_x: f32 },
}

pub struct TimelineEditor {
    surface: TimelineSurface,
    curve_editor: NodeHandle,
    font_id: u8,
    gesture: Gesture,
}

impl TimelineEditor {
    /// The curve strip is for a channel's keys; with no channel to show it folds away and the lanes take its room.
    fn curve_height(&self) -> f32 {
        if self.surface.selected_curve().is_some() {
            CURVE_HEIGHT
        } else {
            0.0
        }
    }

    fn timeline_bounds(&self, widget: &Widget) -> Rect {
        let bounds = widget.screen_bounds();
        Rect::new(
            bounds.x,
            bounds.y,
            bounds.w,
            (bounds.h - self.curve_height()).max(RULER_HEIGHT),
        )
    }

    fn content_left(widget: &Widget) -> f32 {
        widget.screen_bounds().x + LABEL_WIDTH
    }

    fn time_at(&self, widget: &Widget, x: f32) -> f32 {
        self.surface.view.x_to_time(x, Self::content_left(widget))
    }

    fn emit_document(&self, widget: &Widget, emit: &mut Vec<UiMessage>) {
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            TimelineEditorMessage::Changed(self.surface.document().clone()),
        ));
    }

    fn emit_playhead(&self, widget: &Widget, emit: &mut Vec<UiMessage>) {
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            TimelineEditorMessage::PlayheadChanged(self.surface.playhead),
        ));
    }

    fn push_selected_curve(&self, emit: &mut Vec<UiMessage>) {
        if self.curve_editor.is_some() {
            if let Some(curve) = self.surface.selected_curve() {
                emit.push(CurveEditorMessage::set_value(
                    self.curve_editor,
                    curve.clone(),
                ));
            }
        }
    }

    fn handle_timeline_message(
        &mut self,
        widget: &mut Widget,
        message: &TimelineEditorMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        let changed = match message {
            TimelineEditorMessage::SetDocument(document) => {
                if self.surface.set_document(document.clone()).is_ok() {
                    self.push_selected_curve(emit);
                    widget.invalidate_layout();
                }
                false
            }
            TimelineEditorMessage::AddGroup { title, parent } => {
                self.surface.add_group(title.clone(), *parent).is_ok()
            }
            TimelineEditorMessage::AddTrack {
                archetype,
                title,
                group,
            } => self
                .surface
                .add_track(archetype, title.clone(), *group)
                .is_ok(),
            TimelineEditorMessage::AddMedia {
                track,
                kind,
                source,
                start,
                duration,
            } => self
                .surface
                .add_media(*track, kind, source.clone(), *start, *duration)
                .is_ok(),
            TimelineEditorMessage::MoveMedia { media, start } => {
                self.surface.move_media(*media, *start)
            }
            TimelineEditorMessage::ResizeMedia {
                media,
                start,
                duration,
            } => self.surface.resize_media(*media, *start, *duration),
            TimelineEditorMessage::AddMarker { time, label } => {
                self.surface.add_marker(*time, label.clone()).is_ok()
            }
            TimelineEditorMessage::MoveMarker { marker, time } => {
                self.surface.move_marker(*marker, *time)
            }
            TimelineEditorMessage::AddKeyframe {
                track,
                channel,
                key,
            } => self.surface.add_keyframe(*track, *channel, *key).is_ok(),
            TimelineEditorMessage::MoveKeyframe {
                track,
                channel,
                key,
                time,
            } => self.surface.move_keyframe(*track, *channel, *key, *time),
            TimelineEditorMessage::RemoveTrack(track) => {
                let changed = self.surface.remove_track(*track);
                if changed {
                    self.push_selected_curve(emit);
                }
                changed
            }
            TimelineEditorMessage::SelectChannel { track, channel } => {
                if self.surface.select_channel(*track, *channel) {
                    self.push_selected_curve(emit);
                }
                false
            }
            TimelineEditorMessage::SetSelectedCurve { curve, live } => {
                let changed = self.surface.set_selected_curve(curve.clone(), *live);
                if changed {
                    self.emit_document(widget, emit);
                }
                false
            }
            TimelineEditorMessage::SetPlayhead(time) => {
                self.surface.scrub(*time);
                self.emit_playhead(widget, emit);
                false
            }
            TimelineEditorMessage::SetSnap(seconds) => {
                if seconds.is_finite() && *seconds >= 0.0 {
                    self.surface.view.snap = *seconds;
                }
                false
            }
            TimelineEditorMessage::Zoom {
                factor,
                anchor_time,
            } => {
                self.surface.view.zoom_at(*factor, *anchor_time);
                false
            }
            TimelineEditorMessage::Command(id) => {
                let changed = match id.as_str() {
                    "editor.edit.undo" => self.surface.undo(),
                    "editor.edit.redo" => self.surface.redo(),
                    _ => false,
                };
                if changed {
                    self.push_selected_curve(emit);
                }
                changed
            }
            TimelineEditorMessage::BindCurveEditor(handle) => {
                self.curve_editor = *handle;
                self.push_selected_curve(emit);
                false
            }
            TimelineEditorMessage::Changed(_) | TimelineEditorMessage::PlayheadChanged(_) => false,
        };
        if changed {
            widget.invalidate_layout();
            self.emit_document(widget, emit);
        }
    }

    fn draw_ruler(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let theme = crate::theme::active();
        let bounds = self.timeline_bounds(widget);
        let left = Self::content_left(widget);
        ctx.push_rect_filled(
            Rect::new(bounds.x, bounds.y, bounds.w, RULER_HEIGHT),
            theme.semantic.surface.header.bytes(),
        );
        ctx.push_rect_filled(
            Rect::new(bounds.x, bounds.y + RULER_HEIGHT - 1.0, bounds.w, 1.0),
            theme.semantic.border.subtle.bytes(),
        );
        ctx.push_text(
            "Tracks",
            Vec2::new(bounds.x + 10.0, bounds.y + 9.0),
            self.font_id,
            11.0,
            theme.semantic.text.muted.bytes(),
        );
        let seconds_per_major =
            (90.0 / self.surface.view.pixels_per_second).max(self.surface.view.snap.max(0.001));
        let power = 10.0_f32.powf(seconds_per_major.log10().floor());
        let major = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|step| step * power)
            .find(|step| *step >= seconds_per_major)
            .unwrap_or(10.0 * power);
        let minor = major / 5.0;
        ctx.push_clip_rect(Rect::new(left, bounds.y, (bounds.w - LABEL_WIDTH).max(0.0), RULER_HEIGHT));
        let first = (self.surface.view.origin / minor).floor() * minor;
        let mut step = 0;
        let mut time = first.max(0.0);
        while time <= self.surface.document().duration() + minor * 0.5 {
            let x = self.surface.view.time_to_x(time, left);
            if x > bounds.x + bounds.w {
                break;
            }
            let is_major = ((time / major).round() * major - time).abs() < minor * 0.25;
            let tick = if is_major { 10.0 } else { 4.0 };
            ctx.push_rect_filled(
                Rect::new(x, bounds.y + RULER_HEIGHT - 1.0 - tick, 1.0, tick),
                if is_major {
                    theme.semantic.border.default.bytes()
                } else {
                    theme.semantic.border.subtle.bytes()
                },
            );
            if is_major {
                ctx.push_text(
                    &seconds_label(time, major),
                    Vec2::new(x + 4.0, bounds.y + 6.0),
                    self.font_id,
                    10.0,
                    theme.semantic.text.secondary.bytes(),
                );
            }
            step += 1;
            time = first.max(0.0) + step as f32 * minor;
        }
        ctx.pop_clip_rect();
    }
}

/// A ruler figure as a person writes it: "2 s", "0.5 s", "0.25 s", to the precision the spacing needs.
fn seconds_label(time: f32, major: f32) -> String {
    let places = if major >= 1.0 {
        0
    } else if major >= 0.1 {
        1
    } else {
        2
    };
    format!("{time:.places$} s")
}

/// A key on a track: a diamond, as in every editor a designer has used.
fn key_diamond(centre: Vec2, radius: f32) -> crate::path::Path {
    let mut path = crate::path::Path::new();
    path.move_to(centre + Vec2::new(0.0, -radius))
        .line_to(centre + Vec2::new(radius, 0.0))
        .line_to(centre + Vec2::new(0.0, radius))
        .line_to(centre + Vec2::new(-radius, 0.0))
        .close();
    path
}

impl Control for TimelineEditor {
    fn authoring(
        &mut self,
        widget: &Widget,
        params: &serde_json::Value,
        emit: &mut Vec<UiMessage>,
    ) -> Result<serde_json::Value, String> {
        if params["operation"].as_str().is_some_and(|s| s != "query")
            && !matches!(self.gesture, Gesture::None)
        {
            return Err("Finish the designer's timeline gesture first".into());
        }
        let result = crate::semantic_authoring::timeline(&mut self.surface, params)?;
        if params["operation"].as_str().is_some_and(|s| s != "query") {
            self.emit_document(widget, emit);
            self.emit_playhead(widget, emit);
            self.push_selected_curve(emit);
        }
        Ok(result)
    }
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        if let Some(&curve) = widget.children.first() {
            ctx.measure_child(curve, Vec2::new(available.x, CURVE_HEIGHT));
        }
        Vec2::new(available.x.max(420.0), available.y.max(260.0))
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        if let Some(&curve) = widget.children.first() {
            // Folded: parked under the control's foot, where its clip shows none of it and no click reaches it.
            let height = self.curve_height();
            ctx.arrange_child(
                curve,
                Rect::new(
                    widget.actual_local_position.x,
                    widget.actual_local_position.y + (final_size.y - height).max(0.0),
                    final_size.x,
                    if height > 0.0 { height.min(final_size.y) } else { CURVE_HEIGHT },
                ),
            );
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        use crate::shaped::ShapedInstance;
        let theme = crate::theme::active();
        let bounds = self.timeline_bounds(widget);
        let left = Self::content_left(widget);
        // A key at nought sits on the column's edge: let the lanes reach a few pixels under it so it is whole.
        let lane = Rect::new(left - 6.0, bounds.y, (bounds.w - LABEL_WIDTH + 6.0).max(0.0), bounds.h);
        ctx.push_rect_filled(bounds, theme.semantic.surface.canvas.bytes());
        // The names column, set apart from the lanes so a track's name never sits on its keys.
        ctx.push_rect_filled(
            Rect::new(bounds.x, bounds.y, LABEL_WIDTH, bounds.h),
            theme.semantic.surface.panel.bytes(),
        );
        self.draw_ruler(widget, ctx);

        let document = self.surface.document();
        let mut y = bounds.y + RULER_HEIGHT;
        for group in document.groups() {
            ctx.push_rect_filled(
                Rect::new(bounds.x, y, bounds.w, GROUP_HEIGHT),
                theme.semantic.surface.header.bytes(),
            );
            ctx.push_text(
                &group.title,
                Vec2::new(bounds.x + 10.0, y + 6.0),
                self.font_id,
                11.0,
                theme.semantic.text.secondary.bytes(),
            );
            y += GROUP_HEIGHT;
        }
        for (row, track) in document.tracks().iter().enumerate() {
            if y + TRACK_HEIGHT > bounds.y + bounds.h {
                break;
            }
            let fill = if row % 2 == 0 {
                theme.semantic.surface.raised.bytes()
            } else {
                theme.semantic.surface.panel.bytes()
            };
            ctx.push_rect_filled(Rect::new(left, y, lane.w, TRACK_HEIGHT), fill);
            ctx.push_rect_filled(
                Rect::new(bounds.x, y + TRACK_HEIGHT - 1.0, bounds.w, 1.0),
                theme.semantic.border.subtle.bytes(),
            );
            ctx.push_clip_rect(Rect::new(bounds.x, y, LABEL_WIDTH - 8.0, TRACK_HEIGHT));
            ctx.push_text(
                &track.title,
                Vec2::new(bounds.x + 10.0, y + 10.0),
                self.font_id,
                12.0,
                theme.semantic.text.primary.bytes(),
            );
            ctx.pop_clip_rect();
            ctx.push_clip_rect(lane);
            for clip in document.media().iter().filter(|clip| clip.track == track.id) {
                let x = self.surface.view.time_to_x(clip.start, left);
                let w = clip.duration * self.surface.view.pixels_per_second;
                let rect = Rect::new(x, y + 4.0, w.max(3.0), TRACK_HEIGHT - 8.0);
                ctx.push_round_rect(rect, theme.geometry.radius_input, theme.semantic.accent.default.bytes());
                ctx.push_clip_rect(rect);
                ctx.push_text(
                    &clip.source,
                    Vec2::new(rect.x + 6.0, rect.y + 6.0),
                    self.font_id,
                    11.0,
                    theme.semantic.text.inverse.bytes(),
                );
                ctx.pop_clip_rect();
            }
            for channel in &track.channels {
                for key in channel.curve.keys() {
                    let x = self.surface.view.time_to_x(key.t, left);
                    ctx.push_path(
                        &key_diamond(Vec2::new(x, y + TRACK_HEIGHT * 0.5), 5.0),
                        ShapedInstance::identity(theme.semantic.status.warning.bytes()),
                    );
                }
            }
            ctx.pop_clip_rect();
            y += TRACK_HEIGHT;
        }
        // The column's edge.
        ctx.push_rect_filled(
            Rect::new(left - 1.0, bounds.y, 1.0, bounds.h),
            theme.semantic.border.default.bytes(),
        );
        if document.tracks().is_empty() && document.groups().is_empty() {
            ctx.push_text(
                "No tracks yet. Choose Edit Events with a rig selected to see its markers here.",
                Vec2::new(left + 16.0, bounds.y + RULER_HEIGHT + 16.0),
                self.font_id,
                12.0,
                theme.semantic.text.muted.bytes(),
            );
        }

        ctx.push_clip_rect(lane);
        // Markers: a faint line down the lanes and a flag in the ruler.
        for marker in document.markers() {
            let x = self.surface.view.time_to_x(marker.time, left);
            ctx.push_rect_filled(
                Rect::new(x, bounds.y + RULER_HEIGHT, 1.0, (bounds.h - RULER_HEIGHT).max(0.0)),
                theme.semantic.border.subtle.bytes(),
            );
            ctx.push_path(
                &key_diamond(Vec2::new(x, bounds.y + RULER_HEIGHT - 6.0), 4.0),
                ShapedInstance::identity(theme.semantic.status.warning.bytes()),
            );
        }
        // The playhead, with its time on its head so nobody has to read it off the ruler.
        let playhead = self.surface.view.time_to_x(self.surface.playhead, left);
        let colour = theme.semantic.status.error.bytes();
        ctx.push_rect_filled(Rect::new(playhead - 1.0, bounds.y, 2.0, bounds.h), colour);
        let label = format!("{:.2} s", self.surface.playhead);
        let label_w = ctx.font_atlas.measure_text(&label, 10.0, self.font_id).x + 10.0;
        let head = Rect::new((playhead - label_w * 0.5).max(left), bounds.y + 3.0, label_w, 16.0);
        ctx.push_round_rect(head, theme.geometry.radius_input, colour);
        ctx.push_text(
            &label,
            Vec2::new(head.x + 5.0, head.y + 2.0),
            self.font_id,
            10.0,
            theme.semantic.text.inverse.bytes(),
        );
        ctx.pop_clip_rect();
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        message: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if message.direction == MessageDirection::ToWidget {
            if let Some(CurveEditorMessage::Value { curve, live }) =
                message.data::<CurveEditorMessage>()
            {
                let changed = self.surface.set_selected_curve(curve.clone(), *live);
                if changed {
                    widget.invalidate_layout();
                    self.emit_document(widget, emit);
                }
                message.handled = true;
                return;
            }
        }
        if message.direction == MessageDirection::ToWidget {
            if let Some(data) = message.data::<TimelineEditorMessage>() {
                self.handle_timeline_message(widget, &data.clone(), emit);
                message.handled = true;
                return;
            }
        }
        let Some(input) = message.data::<WidgetMessage>() else {
            return;
        };
        let timeline = self.timeline_bounds(widget);
        match input {
            WidgetMessage::MouseDown { pos, button, .. }
                if timeline.contains(*pos) && *button == MouseButton::Left =>
            {
                self.gesture = Gesture::Scrub;
                self.surface.scrub(self.time_at(widget, pos.x));
                self.emit_playhead(widget, emit);
                message.handled = true;
            }
            WidgetMessage::MouseDown { pos, button, .. }
                if timeline.contains(*pos) && *button == MouseButton::Middle =>
            {
                self.gesture = Gesture::Pan { last_x: pos.x };
                message.handled = true;
            }
            WidgetMessage::MouseMove { pos, .. } => match &mut self.gesture {
                Gesture::Scrub => {
                    self.surface.scrub(self.time_at(widget, pos.x));
                    self.emit_playhead(widget, emit);
                    message.handled = true;
                }
                Gesture::Pan { last_x } => {
                    let delta = pos.x - *last_x;
                    *last_x = pos.x;
                    self.surface
                        .view
                        .pan_pixels(delta, self.surface.document().duration());
                    message.handled = true;
                }
                Gesture::None => {}
            },
            WidgetMessage::MouseUp { .. } if !matches!(self.gesture, Gesture::None) => {
                self.gesture = Gesture::None;
                message.handled = true;
            }
            WidgetMessage::MouseWheel { pos, delta, .. } if timeline.contains(*pos) => {
                let anchor = self.time_at(widget, pos.x);
                let factor = 1.1_f32.powf(*delta / WHEEL_DELTA_PER_LINE);
                self.surface.view.zoom_at(factor, anchor);
                message.handled = true;
            }
            _ => {}
        }
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        if self.timeline_bounds(widget).contains(pos) {
            CursorKind::Pointer
        } else {
            CursorKind::Default
        }
    }

    fn gesture_active(&self) -> bool {
        !matches!(self.gesture, Gesture::None)
    }

    fn cancel_gesture(&mut self, _widget: &mut Widget, _emit: &mut Vec<UiMessage>) -> bool {
        let active = !matches!(self.gesture, Gesture::None);
        self.gesture = Gesture::None;
        active
    }
}

pub struct TimelineEditorBuilder {
    widget: WidgetBuilder,
    catalogue: TimelineCatalogue,
    document: Option<TimelineDocument>,
    font_id: u8,
}

impl TimelineEditorBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder, catalogue: TimelineCatalogue) -> Self {
        Self {
            widget,
            catalogue,
            document: None,
            font_id: 0,
        }
    }

    #[must_use]
    pub fn with_document(mut self, document: TimelineDocument) -> Self {
        self.document = Some(document);
        self
    }

    #[must_use]
    pub fn with_font(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    /// Install the composite control because the embedded curve editor needs
    /// a real child handle in the retained tree.
    pub fn build(
        self,
        ui: &mut UserInterface,
        parent: NodeHandle,
    ) -> Result<TimelineEditorHandles, TimelineError> {
        let mut surface = if let Some(document) = self.document {
            TimelineSurface::with_document(self.catalogue, document)?
        } else {
            TimelineSurface::new(self.catalogue, 10.0)
        };
        if let Some((track, channel)) = surface
            .document()
            .tracks()
            .iter()
            .find_map(|track| (!track.channels.is_empty()).then_some((track.id, 0)))
        {
            surface.select_channel(track, channel);
        }
        let curve = surface
            .selected_curve()
            .cloned()
            .unwrap_or_else(Curve::empty);
        let duration = surface.document().duration();
        let editor = UiNode::new(
            self.widget.build(),
            Box::new(TimelineEditor {
                surface,
                curve_editor: NodeHandle::NONE,
                font_id: self.font_id,
                gesture: Gesture::None,
            }),
        );
        let editor = ui.add_node(editor, parent);
        let curve_editor = ui.add_node(
            CurveEditorBuilder::new(
                WidgetBuilder::new()
                    .with_height(CURVE_HEIGHT)
                    .with_tooltip("Selected channel curve — drag keys; Ctrl snaps; wheel zooms"),
            )
            .with_curve(curve)
            .with_domain(0.0, duration)
            .with_range(-1.0, 1.0)
            .with_font_id(self.font_id)
            .with_height(CURVE_HEIGHT)
            .with_value_target(editor)
            .build(),
            editor,
        );
        ui.send(TimelineEditorMessage::to(
            editor,
            TimelineEditorMessage::BindCurveEditor(curve_editor),
        ));
        Ok(TimelineEditorHandles {
            editor,
            curve_editor,
        })
    }
}

#[allow(dead_code)]
fn _media_id_keeps_public_api_visible(_: MediaId) {}
