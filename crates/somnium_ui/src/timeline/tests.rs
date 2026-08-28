use super::*;
use crate::message::{MessageDirection, Modifiers, MouseButton, UiMessage, WidgetMessage};
use crate::widget::WidgetBuilder;
use glam::Vec2;
use somnium_ecs::curve::{Curve, CurveKey};

fn animation_surface(duration: f32) -> (TimelineSurface, TrackId) {
    let mut surface = TimelineSurface::new(catalogues::animation(), duration);
    let track = surface
        .add_track("animation.clip", "Body", None)
        .expect("built-in animation archetype");
    (surface, track)
}

#[test]
fn catalogues_prove_the_surface_is_not_animation_specific() {
    let animation = catalogues::animation();
    let ui = catalogues::ui_motion();

    assert!(animation.get("animation.clip").is_some());
    assert!(ui.get("ui.motion").is_some());
    assert_ne!(animation.id, ui.id);
    assert_eq!(
        ui.get("ui.motion").unwrap().lanes.len(),
        3,
        "a non-animation consumer supplies its own lanes"
    );
}

#[test]
fn tracks_groups_media_markers_and_keys_share_one_document() {
    let mut surface = TimelineSurface::new(catalogues::animation(), 8.0);
    surface.view.snap = 0.25;
    let group = surface.add_group("Character", None).unwrap();
    let track = surface
        .add_track("animation.clip", "Locomotion", Some(group))
        .unwrap();
    let clip = surface
        .add_media(track, "animation-clip", "walk.anim", 1.0, 3.0)
        .unwrap();
    let marker = surface.add_marker(1.13, "Left foot").unwrap();
    let key = surface
        .add_keyframe(track, 0, CurveKey::new(2.13, 0.5))
        .unwrap();
    assert!(surface.move_media(clip, 1.13));
    assert!(surface.resize_media(clip, 1.13, 2.24));
    assert!(surface.move_marker(marker, 1.63));
    assert!(surface.move_keyframe(track, 0, key, 3.13));

    let document = surface.document();
    assert_eq!(document.groups()[0].id, group);
    assert_eq!(document.track(track).unwrap().group, Some(group));
    assert_eq!(document.media_clip(clip).unwrap().source, "walk.anim");
    assert_eq!(document.media_clip(clip).unwrap().start, 1.25);
    assert_eq!(document.media_clip(clip).unwrap().duration, 2.25);
    assert_eq!(
        document
            .markers()
            .iter()
            .find(|item| item.id == marker)
            .unwrap()
            .time,
        1.75
    );
    assert!(
        document.track(track).unwrap().channels[0]
            .curve
            .keys()
            .iter()
            .any(|key| (key.t - 3.25).abs() < f32::EPSILON)
    );
    assert_eq!(surface.scrub(7.88), 8.0);
    assert!(matches!(
        surface.add_media(track, "audio-clip", "wrong.wav", 0.0, 1.0),
        Err(TimelineError::UnsupportedMedia)
    ));
}

#[test]
fn removing_a_track_cascades_its_media() {
    let (mut surface, track) = animation_surface(4.0);
    surface
        .add_media(track, "animation-clip", "idle.anim", 0.0, 4.0)
        .unwrap();
    assert!(surface.select_channel(track, 0));

    assert!(surface.remove_track(track));
    assert!(surface.document().tracks().is_empty());
    assert!(surface.document().media().is_empty());
    assert_eq!(surface.selection, TimelineSelection::default());
    assert!(!surface.remove_track(track));
    assert!(surface.undo());
    assert_eq!(surface.document().tracks().len(), 1);
    assert_eq!(surface.document().media().len(), 1);
}

#[test]
fn clip_ranges_remain_inside_the_document_at_the_end_boundary() {
    let (mut surface, track) = animation_surface(4.0);
    let clip = surface
        .add_media(track, "animation-clip", "late.anim", 99.0, 2.0)
        .unwrap();
    let authored = surface.document().media_clip(clip).unwrap();
    assert!(authored.start < 4.0);
    assert!(authored.start + authored.duration <= 4.0);

    let mut document = surface.document().clone();
    assert!(document.set_duration(0.0005));
    let shrunk = document.media_clip(clip).unwrap();
    assert!(shrunk.duration > 0.0);
    assert!(shrunk.start + shrunk.duration <= document.duration());
}

#[test]
fn curve_drag_is_one_undo_step_and_discrete_edits_are_undoable() {
    let (mut surface, track) = animation_surface(4.0);
    assert!(surface.select_channel(track, 0));
    let original = surface.selected_curve().unwrap().clone();
    let live_a = Curve::from_keys(vec![CurveKey::new(0.0, 0.2), CurveKey::new(4.0, 0.8)]);
    let live_b = Curve::from_keys(vec![CurveKey::new(0.0, 0.3), CurveKey::new(4.0, 0.7)]);

    assert!(surface.set_selected_curve(live_a, true));
    assert!(surface.set_selected_curve(live_b.clone(), true));
    assert!(surface.set_selected_curve(live_b, false));
    assert_eq!(
        surface.history_labels().last(),
        Some(&"Edit Timeline Curve")
    );
    assert!(surface.undo());
    assert_eq!(surface.selected_curve(), Some(&original));
    assert!(surface.redo());

    let discrete = Curve::from_keys(vec![CurveKey::new(0.0, 0.1)]);
    assert!(surface.set_selected_curve(discrete, false));
    assert!(
        surface.undo(),
        "a delete/preset-style non-live edit is recorded"
    );
}

#[test]
fn view_zoom_preserves_the_anchor_and_time_mapping() {
    let mut view = TimelineView::default();
    let left = 152.0;
    let anchor = 3.0;
    let before = view.time_to_x(anchor, left);
    view.zoom_at(2.0, anchor);

    assert!((view.time_to_x(anchor, left) - before).abs() < 1e-4);
    let x = view.time_to_x(4.25, left);
    assert!((view.x_to_time(x, left) - 4.25).abs() < 1e-4);
    view.pan_pixels(-10_000.0, 6.0);
    assert_eq!(view.origin, 6.0);
}

#[test]
fn asset_round_trip_is_deterministic_and_schema_checked() {
    let catalogue = catalogues::animation();
    let mut surface = TimelineSurface::new(catalogue.clone(), 6.0);
    let group = surface.add_group("Actor", None).unwrap();
    let track = surface
        .add_track("animation.clip", "Base", Some(group))
        .unwrap();
    surface
        .add_media(track, "animation-clip", "run.anim", 1.0, 2.0)
        .unwrap();
    surface.add_marker(2.0, "Impact").unwrap();

    let json = to_json(surface.document()).unwrap();
    let loaded = from_json(&json, &catalogue).unwrap();
    assert_eq!(loaded, *surface.document());
    assert_eq!(to_json(&loaded).unwrap(), json);

    let future = json.replacen("\"version\": 1", "\"version\": 999", 1);
    assert!(matches!(
        from_json(&future, &catalogue),
        Err(TimelineAssetError::FutureVersion(999))
    ));
    let wrong_lane = json.replacen("\"lane\": \"speed\"", "\"lane\": \"unknown\"", 1);
    assert!(matches!(
        from_json(&wrong_lane, &catalogue),
        Err(TimelineAssetError::UnknownLane { .. })
    ));
}

#[test]
fn embedded_curve_editor_is_a_retained_child_with_an_automatic_value_route() {
    let (surface, _track) = animation_surface(4.0);
    let document = surface.document().clone();
    let mut ui = crate::ui::UserInterface::new(640.0, 360.0);
    let root = ui.root();
    let handles = TimelineEditorBuilder::new(
        WidgetBuilder::new().with_width(640.0).with_height(360.0),
        catalogues::animation(),
    )
    .with_document(document)
    .build(&mut ui, root)
    .unwrap();
    ui.update();
    ui.perform_layout();

    assert_eq!(ui.parent_of(handles.curve_editor), Some(handles.editor));
    let bounds = ui.screen_bounds(handles.curve_editor);
    assert!(bounds.w > 0.0 && bounds.h > 0.0);
    let pos = Vec2::new(bounds.x + bounds.w * 0.5, bounds.y + bounds.h * 0.5);
    for _ in 0..2 {
        ui.send(UiMessage::new(
            handles.curve_editor,
            MessageDirection::ToWidget,
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                mods: Modifiers::default(),
            },
        ));
    }
    let outgoing = ui.update();
    let changed = outgoing.iter().find_map(|message| {
        (message.destination == handles.editor).then(|| {
            let TimelineEditorMessage::Changed(document) =
                message.data::<TimelineEditorMessage>()?
            else {
                return None;
            };
            Some(document)
        })?
    });

    let changed = changed.expect("the child edit routes through its retained timeline owner");
    assert_eq!(changed.tracks()[0].channels[0].curve.len(), 3);
    ui.draw();
    assert!(!ui.draw_ctx.instances.is_empty());
}
