//! The Details panel: every section, every property row.
//! Phase 26-Zeta-I — editor construction, split out of `lib.rs`.
//!
//! `lib.rs` keeps `UiManager`: the state machine, the OS-event routing and the
//! `EditorEvent` seam with `app.rs`. Everything in this module tree only
//! *builds* widget trees and hands back handles, so a change to how a surface
//! looks no longer means editing the same 6,000-line file as a change to how
//! the editor behaves.

#![allow(clippy::too_many_arguments)]

use crate::{
    message::NodeHandle,
    theme,
    types::Thickness,
    typography::TextRole,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        check_box::CheckBoxBuilder,
        color_picker::ColorSwatchBuilder,
        combo_box::ComboBoxBuilder,
        numeric_field::NumericFieldBuilder,
        property_row::PropertyRowBuilder,
        search_box::build_property_row,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
    },
};

// Glob the crate root for the shared handle bundles and name tables, and the
// sibling `parts` module for the small builders. Explicit imports above shadow
// the globs, so this cannot silently change which `TextBuilder` is in scope.
#[allow(unused_imports)]
use crate::editor::parts::*;
use crate::*;

/// Build the 9 NumericFields for the inspector TRS section.
/// Returns the inspector handle bundle.
pub(crate) fn build_inspector(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> InspectorHandles {
    // `label_w` widens the gutter for the light section's longer labels.
    // Returns `(row, field)`. The row handle is what a caller needs to hide a
    // whole line, label included.
    //
    // Phase 26-Zeta: rows are `PropertyRow`s, so the label column, the 14 px
    // modified gutter and the narrow-panel stacking rule are computed once from
    // the redline rather than repeated as a per-section `label_w`. `label_w` is
    // now ignored; it stays in the signature because every call site passes it
    // and the widths were only ever a workaround for the missing grammar.
    let make_row_rw = |ui: &mut UserInterface,
                       label: &str,
                       label_w: f32,
                       font_id: u8,
                       parent: NodeHandle,
                       drag_step: f32| {
        let _ = (label_w, font_id);
        let row = PropertyRowBuilder::new(
            WidgetBuilder::new()
                .with_clip_to_bounds(false)
                .with_background(theme::TRANSPARENT),
        )
        .with_label(label)
        .build();
        let row_h = ui.add_node(row, parent);

        let field = NumericFieldBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 0.0,
            top: 1.0,
            right: 0.0,
            bottom: 1.0,
        }))
        .with_drag_step(drag_step)
        .build();
        (row_h, ui.add_node(field, row_h))
    };
    let make_color =
        |ui: &mut UserInterface, label: &str, label_w: f32, font_id: u8, parent: NodeHandle| {
            let _ = (label_w, font_id);
            let row = PropertyRowBuilder::new(
                WidgetBuilder::new()
                    .with_clip_to_bounds(false)
                    .with_background(theme::TRANSPARENT),
            )
            .with_label(label)
            .build();
            let row_h = ui.add_node(row, parent);
            let swatch = ColorSwatchBuilder::new(WidgetBuilder::new()).build();
            ui.add_node(swatch, row_h)
        };
    // 0.05 per pixel: a 100-pixel drag moves a position by 5 units, which is
    // the right feel for metres and degrees alike.
    let make_row_w =
        |ui: &mut UserInterface, label: &str, label_w: f32, font_id: u8, parent: NodeHandle| {
            make_row_rw(ui, label, label_w, font_id, parent, 0.05).1
        };
    let make_row_step =
        |ui: &mut UserInterface,
         label: &str,
         label_w: f32,
         font_id: u8,
         parent: NodeHandle,
         step: f32| { make_row_rw(ui, label, label_w, font_id, parent, step).1 };
    let make_row = |ui: &mut UserInterface, label: &str, font_id: u8, parent: NodeHandle| {
        make_row_w(ui, label, 20.0, font_id, parent)
    };

    // Section headers are a 26 px band on `surface.header`, not a floating
    // caption: the band is what separates one group of rows from the next now
    // that the rows themselves have no borders.
    let sec_label = |ui: &mut UserInterface, text: &str, font_id: u8, parent: NodeHandle| {
        let _ = font_id;
        let band = BorderBuilder::new(
            WidgetBuilder::new()
                .with_height(theme::NOCTURNE.density.row_tree)
                .with_background(theme::BG_HEADER)
                .with_foreground(theme::BORDER_DARK),
        )
        .with_stroke_thickness(Thickness {
            left: 0.0,
            right: 0.0,
            top: 1.0,
            bottom: 1.0,
        })
        .build();
        let band_h = ui.add_node(band, parent);
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 10.0,
            top: 7.0,
            right: 0.0,
            bottom: 0.0,
        }))
        .with_role(TextRole::SectionCaps)
        .with_text(text)
        .build();
        ui.add_node(lbl, band_h);
    };

    sec_label(ui, "Position", font_id, parent);
    let pos_x = make_row(ui, "X", font_id, parent);
    let pos_y = make_row(ui, "Y", font_id, parent);
    let pos_z = make_row(ui, "Z", font_id, parent);

    sec_label(ui, "Rotation", font_id, parent);
    let rot_x = make_row(ui, "X", font_id, parent);
    let rot_y = make_row(ui, "Y", font_id, parent);
    let rot_z = make_row(ui, "Z", font_id, parent);

    sec_label(ui, "Scale", font_id, parent);
    let sc_x = make_row(ui, "X", font_id, parent);
    let sc_y = make_row(ui, "Y", font_id, parent);
    let sc_z = make_row(ui, "Z", font_id, parent);

    // ── Light section (Phase 13E) ────────────────────────────────────────────
    // Lives in its own panel so it can be hidden when the selection isn't a
    // light. Angles are shown in degrees; range/angles only apply to
    // point/spot lights (a directional light ignores them).
    let light_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let light_section = ui.add_node(light_panel, parent);

    sec_label(ui, "Light", font_id, light_section);
    let light_intensity = make_row_w(ui, "Intensity", 34.0, font_id, light_section);
    let light_color = make_color(ui, "Color", 34.0, font_id, light_section);
    let light_col_r = NodeHandle::NONE;
    let light_col_g = NodeHandle::NONE;
    let light_col_b = NodeHandle::NONE;
    let light_temp_k = make_row_step(ui, "Kelvin", 34.0, font_id, light_section, 5.0);
    let (light_range_row, light_range) =
        make_row_rw(ui, "Range", 34.0, font_id, light_section, 0.1);
    let (light_inner_row, light_inner) =
        make_row_rw(ui, "Inner angle", 34.0, font_id, light_section, 0.2);
    let (light_outer_row, light_outer) =
        make_row_rw(ui, "Outer angle", 34.0, font_id, light_section, 0.2);
    let (light_moon_row, light_moon_int) =
        make_row_rw(ui, "Moon intensity", 34.0, font_id, light_section, 0.005);
    let light_radius = make_row_step(ui, "Radius", 34.0, font_id, light_section, 0.01);
    let (light_width_row, light_width) =
        make_row_rw(ui, "Half width", 34.0, font_id, light_section, 0.05);
    let (light_height_row, light_height) =
        make_row_rw(ui, "Half height", 34.0, font_id, light_section, 0.05);
    ui.set_visibility(light_width_row, false);
    ui.set_visibility(light_height_row, false);
    ui.set_visibility(light_section, false);

    // ── Camera section (Phase CR-C) ──────────────────────────────────────────
    let camera_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let camera_section = ui.add_node(camera_panel, parent);
    sec_label(ui, "Camera", font_id, camera_section);
    let (camera_frustum_toggle, camera_frustum_label) =
        make_toggle_checked(ui, "Frustum Cull", font_id, camera_section, true);
    // Phase DOOM-F. Off by default, and the floor sits directly under the
    // toggle so the quality being traded away is visible at the moment the
    // trade is made.
    let (camera_dynres_toggle, camera_dynres_label) =
        make_toggle(ui, "Dynamic Resolution", font_id, camera_section);
    let camera_dynres_target = make_row_step(ui, "Target ms", 34.0, font_id, camera_section, 0.5);
    let camera_dynres_floor = make_row_step(ui, "Res floor %", 34.0, font_id, camera_section, 1.0);
    ui.set_visibility(camera_section, false);

    // ── Post-processing section (Phase 15A1) ─────────────────────────────────
    // The engine has no checkbox widget, so a Button whose label carries the
    // tick doubles as one: clicking flips the state and the label is rewritten.
    let post_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let post_section = ui.add_node(post_panel, parent);

    sec_label(ui, "Post FX", font_id, post_section);
    // Phase 24A/24B exposure controls. "EV" is the manual exposure value;
    // "EC" is compensation in stops, which is what you reach for when the meter
    // is right but the shot wants to be a stop darker.
    let (post_auto_exp_toggle, post_auto_exp_label) =
        make_toggle(ui, "Auto Exposure", font_id, post_section);
    let post_exposure = make_row_step(ui, "Exposure EV", 34.0, font_id, post_section, 0.05);
    let post_exp_comp = make_row_step(ui, "Exposure comp.", 34.0, font_id, post_section, 0.05);
    // Phase 24A. With this on, EV above is computed from the three rows under
    // it, so a scene is lit by picking a real exposure triangle instead of a
    // number with no units. Aperture drives the DoF blur either way.
    let (post_phys_toggle, post_phys_label) =
        make_toggle(ui, "Physical Camera", font_id, post_section);
    let post_aperture = make_row_step(ui, "Aperture f/", 34.0, font_id, post_section, 0.05);
    // Shown as the denominator a photographer reads — 100 means 1/100 s —
    // because the seconds themselves are a third of a hundredth and no scrub
    // rate makes that row usable.
    let post_shutter = make_row_step(ui, "Shutter 1/s", 34.0, font_id, post_section, 1.0);
    let post_iso = make_row_step(ui, "ISO", 34.0, font_id, post_section, 2.0);
    let (post_tonemap_button, post_tonemap_label) = {
        let tonemap_combo = ComboBoxBuilder::new(WidgetBuilder::new())
            .with_items(TONEMAP_NAMES)
            .with_font_id(font_id)
            .build();
        let (_, h) = build_property_row(ui, post_section, "Tonemap", font_id, tonemap_combo);
        (h, h)
    };
    // Indirect-light strength. Low values give contrasty shadows, high values a
    // flatter, brighter scene — see `PostProcessComponent::ibl_intensity`.
    let post_ibl = make_row_step(ui, "IBL intensity", 34.0, font_id, post_section, 0.005);
    let (post_vig_toggle, post_vig_label) = make_toggle(ui, "Vignette", font_id, post_section);
    let post_vig_str = make_row_step(ui, "Vignette amount", 34.0, font_id, post_section, 0.01);
    let (post_ca_toggle, post_ca_label) = make_toggle(ui, "Chromatic Ab.", font_id, post_section);
    let post_ca_str = make_row_step(ui, "Aberration amount", 34.0, font_id, post_section, 0.0002);
    let (post_fxaa_toggle, post_fxaa_label) = make_toggle(ui, "FXAA", font_id, post_section);
    // Phase 24AC. Next to FXAA because they are the two filters that run on the
    // finished LDR image, and because the pair is what a reader is comparing.
    let (post_cas_toggle, post_cas_label) = make_toggle(ui, "Sharpen (CAS)", font_id, post_section);
    let post_cas_sharp = make_row_step(ui, "CAS sharpness", 34.0, font_id, post_section, 0.01);
    let post_cas_strength = make_row_step(ui, "CAS amount", 34.0, font_id, post_section, 0.01);
    // Phase 24Z. Below the two AA/sharpen filters because it is the other
    // camera-motion effect, and its shutter is a photographic quantity like the
    // exposure rows at the top.
    let (post_mb_toggle, post_mb_label) = make_toggle(ui, "Motion Blur", font_id, post_section);
    let post_mb_shutter = make_row_step(ui, "Blur shutter", 34.0, font_id, post_section, 0.01);
    let (post_cel_toggle, post_cel_label) = make_toggle(ui, "Cel Shading", font_id, post_section);

    // FSR 3 temporal reconstruct. Default on; owns AA (and RCAS) while enabled.
    let (post_fsr_toggle, post_fsr_label) = make_toggle(ui, "FSR", font_id, post_section);
    let post_fsr_sharp = make_row_step(ui, "FSR sharpness", 34.0, font_id, post_section, 0.01);

    // Phase 24F/24I/24K/24T/24Z. Ordered roughly the way the frame runs, so the
    // list reads as a pipeline rather than an unsorted pile of switches.
    let (post_taa_toggle, post_taa_label) =
        make_toggle(ui, "TAA (FSR owns AA)", font_id, post_section);
    let (post_gtao_toggle, post_gtao_label) = make_toggle(ui, "GTAO", font_id, post_section);
    // Phase DOOM-B/C diagnostics. Both default off and both are measurement
    // tools rather than features: the census costs 0.08 ms and answers "which
    // pixels", the bin path is a working tile classifier that measured slower
    // than the fullscreen draw at every tile size.
    let (post_census_toggle, post_census_label) =
        make_toggle(ui, "Pixel Census", font_id, post_section);
    let (post_bins_toggle, post_bins_label) =
        make_toggle(ui, "Shade Bins", font_id, post_section);
    // Radius is in metres and is the control that decides whether AO reads as
    // contact darkening under an object or as a broad smear across a hillside.
    let post_ao_radius = make_row_step(ui, "AO radius", 34.0, font_id, post_section, 0.02);
    let post_ao_intensity = make_row_step(ui, "AO intensity", 34.0, font_id, post_section, 0.02);
    let (post_restir_toggle, post_restir_label) =
        make_toggle(ui, "RT Direct Light", font_id, post_section);
    // Phase 24L. Directly under the direct-light switch, because it is the
    // other half of the same traced solution and they read as a pair.
    let (post_restir_gi_toggle, post_restir_gi_label) =
        make_toggle(ui, "RT Indirect (GI)", font_id, post_section);
    let (post_rt_reflect_toggle, post_rt_reflect_label) =
        make_toggle(ui, "RT Reflections", font_id, post_section);
    let (post_rt_refract_toggle, post_rt_refract_label) =
        make_toggle(ui, "RT Refraction", font_id, post_section);
    // Phase 24L. Directly under its toggle, matching every other effect that
    // pairs a switch with an amount.
    let post_gi_intensity = make_row_step(ui, "GI intensity", 34.0, font_id, post_section, 0.01);
    let (post_pcss_toggle, post_pcss_label) =
        make_toggle(ui, "Soft Shadows", font_id, post_section);
    let (post_contact_toggle, post_contact_label) =
        make_toggle(ui, "Contact Shadows", font_id, post_section);
    let (post_bloom_toggle, post_bloom_label) = make_toggle(ui, "Bloom", font_id, post_section);
    let post_bloom_amt = make_row_step(ui, "Bloom amount", 34.0, font_id, post_section, 0.002);
    let (post_dof_toggle, post_dof_label) =
        make_toggle(ui, "Depth of Field", font_id, post_section);
    let post_dof_focus = make_row_step(ui, "Focus distance", 34.0, font_id, post_section, 0.1);
    let post_temperature = make_row_step(ui, "Temperature", 34.0, font_id, post_section, 0.01);
    // The other white-balance axis; without it "Temp" can only slide a scene
    // between orange and blue and never correct a green cast.
    let post_tint = make_row_step(ui, "Tint", 34.0, font_id, post_section, 0.01);
    let post_contrast = make_row_step(ui, "Contrast", 34.0, font_id, post_section, 0.01);
    let post_saturation = make_row_step(ui, "Saturation", 34.0, font_id, post_section, 0.01);
    // Phase 24Y lift/gamma/gain: shadows, midtones, highlights.
    let post_lift = make_row_step(ui, "Lift", 34.0, font_id, post_section, 0.005);
    let post_gamma = make_row_step(ui, "Gamma", 34.0, font_id, post_section, 0.01);
    let post_gain = make_row_step(ui, "Gain", 34.0, font_id, post_section, 0.01);
    let post_grain = make_row_step(ui, "Grain", 34.0, font_id, post_section, 0.002);

    // Phases 24U/25I. Aerial perspective is always on with the volume — the
    // atmosphere is not optional — so the toggle covers the whole volume and
    // the density below controls only the fog medium on top of it.
    let (post_vol_toggle, post_vol_label) = make_toggle(ui, "Volumetrics", font_id, post_section);
    let (post_shafts_toggle, post_shafts_label) =
        make_toggle(ui, "Light Shafts", font_id, post_section);
    let post_shaft_amt = make_row_step(ui, "Light shafts", 34.0, font_id, post_section, 0.05);
    // Fog density is tiny — a visible haze is ~1e-3 per metre — so the scrub
    // rate has to be far finer than the other rows or one pixel of drag takes
    // the scene from clear to opaque.
    let post_fog_density = make_row_step(ui, "Fog density", 34.0, font_id, post_section, 0.00005);
    let post_fog_height = make_row_step(ui, "Fog height", 34.0, font_id, post_section, 1.0);
    let post_fog_asym = make_row_step(ui, "Fog anisotropy", 34.0, font_id, post_section, 0.01);
    let (post_world_cache_toggle, post_world_cache_label) =
        make_toggle(ui, "World Cache", font_id, post_section);
    let post_cache_intensity = make_row_step(ui, "Cache blend", 34.0, font_id, post_section, 0.02);
    let post_cache_cell = make_row_step(ui, "Cell size m", 34.0, font_id, post_section, 0.05);
    let (post_specular_toggle, post_specular_label) =
        make_toggle(ui, "RT Specular", font_id, post_section);
    let post_spec_rough = make_row_step(ui, "Specular rough", 34.0, font_id, post_section, 0.01);
    let (post_path_toggle, post_path_label) = make_toggle(ui, "Path Tracer", font_id, post_section);
    let post_path_bounces = make_row_step(ui, "Bounces", 34.0, font_id, post_section, 1.0);
    let (post_sdf_toggle, post_sdf_label) = make_toggle(ui, "Mesh SDF", font_id, post_section);
    let (post_probes_toggle, post_probes_label) = make_toggle(ui, "Probes", font_id, post_section);
    let post_probe_intensity =
        make_row_step(ui, "Probe intensity", 34.0, font_id, post_section, 0.02);
    let (post_analytic_toggle, post_analytic_label) =
        make_toggle(ui, "Analytic Mips", font_id, post_section);
    ui.set_visibility(post_section, false);

    // ── Foliage (Phase 17C) ──────────────────────────────────────────────────
    let foliage_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let foliage_section = ui.add_node(foliage_panel, parent);
    sec_label(ui, "Foliage", font_id, foliage_section);
    let (foliage_toggle, foliage_label) = make_toggle(ui, "Enabled", font_id, foliage_section);
    // Phase 17F: the brush. Paint arms it, Erase flips the stroke, Single
    // places one instance at the cursor — which is how trees get placed.
    let (foliage_paint_toggle, foliage_paint_label) =
        make_toggle(ui, "Foliage Paint", font_id, foliage_section);
    let (foliage_erase_toggle, foliage_erase_label) =
        make_toggle(ui, "Erase", font_id, foliage_section);
    let (foliage_single_toggle, foliage_single_label) =
        make_toggle(ui, "Single", font_id, foliage_section);
    let kind_combo = ComboBoxBuilder::new(WidgetBuilder::new())
        .with_items(FOLIAGE_KIND_NAMES)
        .with_font_id(font_id)
        .build();
    let (_, foliage_kind_button) =
        build_property_row(ui, foliage_section, "Type", font_id, kind_combo);
    let foliage_kind_label = foliage_kind_button;
    // Density is per square metre and lives well under 1, so it needs a far
    // finer drag rate than a position.
    let foliage_density = make_row_step(ui, "Density", 34.0, font_id, foliage_section, 0.02);
    let foliage_seed = make_row_step(ui, "Size", 34.0, font_id, foliage_section, 0.05);
    let foliage_slope = make_row_step(ui, "Max slope", 34.0, font_id, foliage_section, 0.2);
    // Kept only so the engine's field routing still has a handle. Hiding the
    // whole row matters, not just the field — the label lives in the row, and
    // hiding the field alone leaves a stray "Type" caption behind.
    let (foliage_layer_row, foliage_layer) =
        make_row_rw(ui, "Type", 34.0, font_id, foliage_section, 0.02);
    ui.set_visibility(foliage_layer_row, false);
    let foliage_smin = make_row_step(ui, "Scale min", 34.0, font_id, foliage_section, 0.01);
    let foliage_smax = make_row_step(ui, "Scale max", 34.0, font_id, foliage_section, 0.01);
    // Phase 24AE. Metres, so a whole-number step: this is the dial that decides
    // how much of the shadow pass a grass field is allowed to cost, and the
    // profiler's `shadow casters` row is the readout for it.
    let foliage_shadow = make_row_step(ui, "Shadow distance", 34.0, font_id, foliage_section, 1.0);
    let foliage_cull = make_row_step(ui, "Cull distance", 34.0, font_id, foliage_section, 1.0);
    let foliage_lod = make_row_step(ui, "LOD distance", 34.0, font_id, foliage_section, 1.0);
    let foliage_impostor = make_row_step(ui, "Impostor", 34.0, font_id, foliage_section, 1.0);
    ui.set_visibility(foliage_section, false);

    // ── Terrain layers (Phase 17C) ───────────────────────────────────────────
    let terrain_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let terrain_section = ui.add_node(terrain_panel, parent);
    sec_label(ui, "Terrain", font_id, terrain_section);
    let terrain_mode_label = TextBuilder::new(WidgetBuilder::new().with_height(32.0).with_margin(
        Thickness {
            left: 6.0,
            top: 2.0,
            right: 6.0,
            bottom: 2.0,
        },
    ))
    .with_role(TextRole::Caption)
    .with_text("Active: none")
    .build();
    let terrain_mode_label = ui.add_node(terrain_mode_label, terrain_section);
    let (terrain_paint_toggle, terrain_paint_label) =
        make_toggle(ui, "Terrain Paint", font_id, terrain_section);
    let (terrain_hex_toggle, terrain_hex_label) =
        make_toggle(ui, "Hex Tiling", font_id, terrain_section);
    let (terrain_parallax_toggle, terrain_parallax_label) =
        make_toggle(ui, "Parallax", font_id, terrain_section);
    let (terrain_clipmap_toggle, terrain_clipmap_label) =
        make_toggle(ui, "Clipmap", font_id, terrain_section);
    // Phase DOOM-E. Off by default: with only hex and parallax removed the
    // aerial pipeline is invisible and costs 2.3 ms, and with the layer scan cut
    // as well it is a real look change on distant ground. Both numbers are in
    // `dev records/phase DOOM/README.md`.
    let (terrain_aerial_toggle, terrain_aerial_label) =
        make_toggle(ui, "Aerial LOD", font_id, terrain_section);
    let terrain_aerial_dist = make_row_step(ui, "Aerial dist m", 34.0, font_id, terrain_section, 5.0);
    let (terrain_aerial_hero_toggle, terrain_aerial_hero_label) =
        make_toggle(ui, "Aerial 16 layers", font_id, terrain_section);
    let (terrain_morph_toggle, terrain_morph_label) =
        make_toggle(ui, "LOD Morph", font_id, terrain_section);
    let terrain_morph_start = make_row_step(ui, "Morph", 34.0, font_id, terrain_section, 0.02);
    let mut terrain_brush_items = Vec::with_capacity(6);
    for row in 0..2 {
        let row_panel =
            StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Horizontal)
                .build();
        let row_h = ui.add_node(row_panel, terrain_section);
        for col in 0..3 {
            let tool = (row * 3 + col) as u8;
            let (btn, lbl) =
                make_palette_button(ui, TERRAIN_BRUSH_NAMES[tool as usize], font_id, row_h);
            terrain_brush_items.push((btn, lbl, tool));
        }
    }
    // Whole steps: the paint layer is an index, and a fractional drag would be
    // meaningless.
    let terrain_layer = make_row_step(ui, "Paint", 34.0, font_id, terrain_section, 0.02);
    let mut terrain_palette = [NodeHandle::NONE; 32];
    let mut terrain_palette_labels = [NodeHandle::NONE; 32];
    for row in 0..8 {
        let row_panel =
            StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Horizontal)
                .build();
        let row_h = ui.add_node(row_panel, terrain_section);
        for col in 0..4 {
            let i = row * 4 + col;
            let (btn, lbl) = make_palette_button(ui, TERRAIN_LAYER_SHORT[i], font_id, row_h);
            terrain_palette[i] = btn;
            terrain_palette_labels[i] = lbl;
        }
    }
    let terrain_tile = make_row_step(ui, "Tile scale", 34.0, font_id, terrain_section, 0.01);
    // Phase 25H: multiplies the relief depth every layer authors for itself, so
    // one dial covers the whole terrain without flattening the differences
    // between gravel and mud. 0 switches parallax off.
    let terrain_relief = make_row_step(ui, "Relief", 34.0, font_id, terrain_section, 0.05);
    let terrain_wetness = make_row_step(ui, "Wetness", 34.0, font_id, terrain_section, 0.02);
    let terrain_macro = make_row_step(ui, "Macro variation", 34.0, font_id, terrain_section, 0.02);
    let terrain_debug = make_row_step(ui, "Debug view", 34.0, font_id, terrain_section, 1.0);
    ui.set_visibility(terrain_section, false);

    let water_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let water_section = ui.add_node(water_panel, parent);
    sec_label(ui, "Water Body", font_id, water_section);
    let water_surface = make_row_step(ui, "Water level", 34.0, font_id, water_section, 0.05);
    let water_depth = make_row_step(ui, "Depth", 34.0, font_id, water_section, 0.05);
    let water_clarity = make_row_step(ui, "Clarity", 34.0, font_id, water_section, 0.01);
    let water_amplitude = make_row_step(ui, "Wave height", 34.0, font_id, water_section, 0.01);
    let water_roughness = make_row_step(ui, "Roughness", 34.0, font_id, water_section, 0.01);
    let water_ssr = make_row_step(ui, "SSR", 34.0, font_id, water_section, 0.01);
    let water_rt_reflect = make_row_step(ui, "RT Reflect", 34.0, font_id, water_section, 0.01);
    let water_reflect_debug = make_row_step(ui, "Reflect Debug", 34.0, font_id, water_section, 1.0);
    let water_wave_a = make_row_step(ui, "Wave A", 34.0, font_id, water_section, 0.25);
    let water_wave_b = make_row_step(ui, "Wave B", 34.0, font_id, water_section, 0.25);
    let water_speed = make_row_step(ui, "Wave speed", 34.0, font_id, water_section, 0.05);
    let water_steepness = make_row_step(ui, "Steepness", 34.0, font_id, water_section, 0.01);
    let water_wind_speed = make_row_step(ui, "Wind", 34.0, font_id, water_section, 0.5);
    let water_foam_decay = make_row_step(ui, "Foam", 34.0, font_id, water_section, 0.05);
    let water_foam_threshold = make_row_step(ui, "Whitecap", 34.0, font_id, water_section, 0.01);
    let water_spectrum_blend = make_row_step(ui, "Spectrum", 34.0, font_id, water_section, 0.01);
    let water_edge_scale = make_row_step(ui, "Edge fade", 34.0, font_id, water_section, 0.05);
    let water_anisotropy = make_row_step(ui, "Anisotropy", 34.0, font_id, water_section, 0.01);
    let water_caustic = make_row_step(ui, "Caustics", 34.0, font_id, water_section, 0.05);
    let water_deep = make_color(ui, "Deep colour", 34.0, font_id, water_section);
    let water_shallow = make_color(ui, "Shallow colour", 34.0, font_id, water_section);
    let water_edge = make_color(ui, "Edge colour", 34.0, font_id, water_section);
    let water_abs = make_color(ui, "Absorption", 34.0, font_id, water_section);
    let water_abs_mag = make_row_step(ui, "Absorption mag.", 34.0, font_id, water_section, 0.005);
    let water_scatter = make_color(ui, "Scattering", 34.0, font_id, water_section);
    let water_scatter_mag =
        make_row_step(ui, "Scattering mag.", 34.0, font_id, water_section, 0.005);
    let water_dir_ax = make_row_step(ui, "Wave A dir X", 34.0, font_id, water_section, 0.01);
    let water_dir_az = make_row_step(ui, "Wave A dir Z", 34.0, font_id, water_section, 0.01);
    let water_dir_bx = make_row_step(ui, "Wave B dir X", 34.0, font_id, water_section, 0.01);
    let water_dir_bz = make_row_step(ui, "Wave B dir Z", 34.0, font_id, water_section, 0.01);
    let (water_underwater, _) = {
        let cb = CheckBoxBuilder::new(WidgetBuilder::new().with_height(theme::ROW_HEIGHT))
            .with_label("Underwater")
            .with_font_id(font_id)
            .build();
        (ui.add_node(cb, water_section), NodeHandle::NONE)
    };
    ui.set_visibility(water_section, false);

    let particle_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let particle_section = ui.add_node(particle_panel, parent);
    sec_label(ui, "Particles", font_id, particle_section);
    let particle_start = make_color(ui, "Start colour", 34.0, font_id, particle_section);
    let particle_end = make_color(ui, "End colour", 34.0, font_id, particle_section);
    ui.set_visibility(particle_section, false);

    let material_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let material_section = ui.add_node(material_panel, parent);
    sec_label(ui, "Material", font_id, material_section);
    let material_base = make_color(ui, "Base colour", 34.0, font_id, material_section);
    ui.set_visibility(material_section, false);

    // ── Scripts (Phase 16-D) ─────────────────────────────────────────────────
    //
    // The only section in this file with no rows in it.
    //
    // Every other section is a fixed widget tree because the component it
    // edits is a fixed Rust struct. A script's properties are declared by
    // its author and can change on the next save, so the rows are built
    // from the schema at refresh time by `UiManager::update_script_inspector`
    // and this is just the container they go into. Hand-writing a field UI
    // per script is the failure mode Phase 16 exists to avoid — see the
    // plan's fourth goal — and 26-J's reflection inspector is meant to
    // adopt this code path rather than grow a second one.
    let script_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let script_section = ui.add_node(script_panel, parent);
    sec_label(ui, "Scripts", font_id, script_section);
    let script_add = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_height(theme::ROW_HEIGHT)
            .with_margin(Thickness::axes(6.0, 2.0)),
    )
    .build();
    let script_add = ui.add_node(script_add, script_section);
    let add_label = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 4.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text("New Script")
    .with_font_size(12.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    ui.add_node(add_label, script_add);
    // Attachments are appended here so the "New Script" button stays at the
    // top of the section rather than sliding down as scripts are added.
    let script_list =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let script_list = ui.add_node(script_list, script_section);
    ui.set_visibility(script_section, false);

    let vessel_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let vessel_section = ui.add_node(vessel_panel, parent);
    sec_label(ui, "Vessel", font_id, vessel_section);
    let vessel_buoyancy = make_row_step(ui, "Buoyancy", 34.0, font_id, vessel_section, 250.0);
    let vessel_drag = make_row_step(ui, "Drag", 34.0, font_id, vessel_section, 50.0);
    let vessel_angular_drag = make_row_step(ui, "Yaw damping", 34.0, font_id, vessel_section, 50.0);
    let vessel_thrust = make_row_step(ui, "Thrust", 34.0, font_id, vessel_section, 250.0);
    let vessel_draft = make_row_step(ui, "Draft", 34.0, font_id, vessel_section, 0.05);
    let vessel_righting = make_row_step(ui, "Righting", 34.0, font_id, vessel_section, 250.0);
    ui.set_visibility(vessel_section, false);

    InspectorHandles {
        pos_x,
        pos_y,
        pos_z,
        rot_x,
        rot_y,
        rot_z,
        sc_x,
        sc_y,
        sc_z,
        light_section,
        light_intensity,
        light_range,
        light_inner,
        light_outer,
        light_col_r,
        light_col_g,
        light_col_b,
        light_color,
        light_temp_k,
        light_range_row,
        light_inner_row,
        light_outer_row,
        light_moon_row,
        light_moon_int,
        light_radius,
        light_width_row,
        light_width,
        light_height_row,
        light_height,
        camera_section,
        camera_frustum_toggle,
        camera_frustum_label,
        camera_dynres_toggle,
        camera_dynres_label,
        camera_dynres_target,
        camera_dynres_floor,
        terrain_section,
        terrain_mode_label,
        terrain_paint_toggle,
        terrain_paint_label,
        terrain_hex_toggle,
        terrain_hex_label,
        terrain_parallax_toggle,
        terrain_parallax_label,
        terrain_clipmap_toggle,
        terrain_aerial_toggle,
        terrain_aerial_label,
        terrain_aerial_dist,
        terrain_aerial_hero_toggle,
        terrain_aerial_hero_label,
        terrain_clipmap_label,
        terrain_morph_toggle,
        terrain_morph_label,
        terrain_morph_start,
        terrain_brush_items,
        terrain_layer,
        terrain_palette,
        terrain_palette_labels,
        terrain_tile,
        terrain_relief,
        terrain_wetness,
        terrain_macro,
        terrain_debug,
        water_section,
        water_surface,
        water_depth,
        water_clarity,
        water_amplitude,
        water_roughness,
        water_ssr,
        water_rt_reflect,
        water_reflect_debug,
        water_wave_a,
        water_wave_b,
        water_speed,
        water_steepness,
        water_wind_speed,
        water_foam_decay,
        water_foam_threshold,
        water_spectrum_blend,
        water_edge_scale,
        water_anisotropy,
        water_caustic,
        water_deep,
        water_shallow,
        water_edge,
        water_abs,
        water_scatter,
        water_abs_mag,
        water_scatter_mag,
        water_underwater,
        water_dir_ax,
        water_dir_az,
        water_dir_bx,
        water_dir_bz,
        particle_section,
        particle_start,
        particle_end,
        material_section,
        material_base,
        script_section,
        script_add,
        script_list,
        vessel_section,
        vessel_buoyancy,
        vessel_drag,
        vessel_angular_drag,
        vessel_thrust,
        vessel_draft,
        vessel_righting,
        foliage_section,
        foliage_toggle,
        foliage_label,
        foliage_paint_toggle,
        foliage_paint_label,
        foliage_erase_toggle,
        foliage_erase_label,
        foliage_single_toggle,
        foliage_single_label,
        foliage_kind_button,
        foliage_kind_label,
        foliage_density,
        foliage_seed,
        foliage_slope,
        foliage_layer,
        foliage_smin,
        foliage_smax,
        foliage_shadow,
        foliage_cull,
        foliage_lod,
        foliage_impostor,
        post_section,
        post_exposure,
        post_exp_comp,
        post_auto_exp_toggle,
        post_auto_exp_label,
        post_tonemap_button,
        post_tonemap_label,
        post_ibl,
        post_vig_toggle,
        post_vig_str,
        post_ca_toggle,
        post_ca_str,
        post_vig_label,
        post_ca_label,
        post_cel_toggle,
        post_cel_label,
        post_fsr_toggle,
        post_fsr_label,
        post_fsr_sharp,
        post_taa_toggle,
        post_taa_label,
        post_gtao_toggle,
        post_census_toggle,
        post_census_label,
        post_bins_toggle,
        post_bins_label,
        post_gtao_label,
        post_restir_toggle,
        post_restir_label,
        post_bloom_toggle,
        post_bloom_label,
        post_restir_gi_toggle,
        post_rt_reflect_toggle,
        post_rt_reflect_label,
        post_rt_refract_toggle,
        post_rt_refract_label,
        post_restir_gi_label,
        post_pcss_toggle,
        post_pcss_label,
        post_contact_toggle,
        post_contact_label,
        post_gi_intensity,
        post_bloom_amt,
        post_dof_toggle,
        post_dof_label,
        post_dof_focus,
        post_temperature,
        post_contrast,
        post_saturation,
        post_grain,
        post_vol_toggle,
        post_vol_label,
        post_shafts_toggle,
        post_shafts_label,
        post_fog_density,
        post_fog_height,
        post_fog_asym,
        post_world_cache_toggle,
        post_world_cache_label,
        post_cache_intensity,
        post_cache_cell,
        post_specular_toggle,
        post_specular_label,
        post_spec_rough,
        post_path_toggle,
        post_path_label,
        post_path_bounces,
        post_sdf_toggle,
        post_sdf_label,
        post_probes_toggle,
        post_probes_label,
        post_probe_intensity,
        post_analytic_toggle,
        post_analytic_label,
        post_shaft_amt,
        post_phys_toggle,
        post_phys_label,
        post_aperture,
        post_shutter,
        post_iso,
        post_tint,
        post_lift,
        post_gamma,
        post_gain,
        post_ao_radius,
        post_ao_intensity,
        post_fxaa_toggle,
        post_fxaa_label,
        post_cas_toggle,
        post_cas_label,
        post_cas_sharp,
        post_cas_strength,
        post_mb_toggle,
        post_mb_label,
        post_mb_shutter,
    }
}
