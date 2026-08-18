//! The editor shell: the three command scopes, the splitter grid, the
//! viewport region, the panels and the status bar.
//! Phase 26-Zeta-I — editor construction, split out of `lib.rs`.
//!
//! `lib.rs` keeps `UiManager`: the state machine, the OS-event routing and the
//! `EditorEvent` seam with `app.rs`. Everything in this module tree only
//! *builds* widget trees and hands back handles, so a change to how a surface
//! looks no longer means editing the same 6,000-line file as a change to how
//! the editor behaves.

#![allow(clippy::too_many_arguments)]

use crate::{
    icons::IconId,
    message::NodeHandle,
    theme,
    types::{HorizontalAlignment, Thickness, VerticalAlignment},
    typography::TextRole,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        button::ButtonBuilder,
        color_picker::ColorPickerBuilder,
        combo_box::ComboBoxBuilder,
        command_palette::CommandPaletteBuilder,
        context_menu::ContextMenuBuilder,
        grid::{Column, GridBuilder, Row},
        image::ImageBuilder,
        menu::MenuBuilder,
        popup::{PopupBuilder, PopupPlacement},
        scroll_viewer::ScrollViewerBuilder,
        search_box::{SearchBoxBuilder, TooltipBuilder},
        text_box::TextBoxBuilder,
        slider::SliderBuilder,
        splitter::{SplitterBuilder, SplitterOrientation},
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        toast::ToastHostBuilder,
        tree_view::TreeViewBuilder,
    },
};

// Glob the crate root for the shared handle bundles and name tables, and the
// sibling `parts` module for the small builders. Explicit imports above shadow
// the globs, so this cannot silently change which `TextBuilder` is in scope.
#[allow(unused_imports)]
use crate::editor::parts::*;
use crate::*;
use glam::Vec2;

// ── Editor layout builder ─────────────────────────────────────────────────────

pub(crate) fn build_editor_layout(
    ui: &mut UserInterface,
    font_id: u8,
    layout: crate::layout_persist::ChromeLayout,
) -> EditorLayout {
    let root = ui.root();

    // Zeta shell budget: application 36 | mode 32 | viewport context 32.
    // Menus now live in the application band; the retired menu row remains at
    // index 1 so every existing GridMessage row index stays stable.
    let outer_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(theme::TITLEBAR_HEIGHT))
        .add_row(Row::strict(0.0))
        .add_row(Row::strict(theme::TOOLBAR_HEIGHT))
        // Retired: the viewport context scope now floats over the render.
        // The row stays at index 3 so existing GridMessage row indices hold.
        .add_row(Row::strict(0.0))
        .add_row(Row::stretch())
        .add_row(Row::strict(theme::BOTTOM_DRAWER_HEIGHT))
        .add_row(Row::strict(theme::STATUS_HEIGHT))
        .add_column(Column::stretch())
        .build();
    let outer_h = ui.add_node(outer_grid, root);

    // ── Row 0: custom title bar ──────────────────────────────────────────────
    let title_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_VOID)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let title_bar_h = ui.add_node(title_bar, outer_h);
    let title_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::auto())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let title_grid_h = ui.add_node(title_grid, title_bar_h);

    let title_drag = BorderBuilder::new(
        WidgetBuilder::new()
            .with_column(0)
            .with_background(theme::TRANSPARENT)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let title_drag = ui.add_node(title_drag, title_grid_h);
    let title_left =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
    let title_left_h = ui.add_node(title_left, title_drag);
    let mark = ImageBuilder::new(
        WidgetBuilder::new()
            .with_width(theme::ICON_MARK + 12.0)
            .with_height(theme::TITLEBAR_HEIGHT)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness {
                left: 10.0,
                top: 0.0,
                right: 2.0,
                bottom: 0.0,
            }),
    )
    .with_icon(IconId::EngineMark)
    .with_size(theme::ICON_MARK)
    .with_tint(theme::ACCENT)
    .build();
    ui.add_node(mark, title_left_h);
    let title_lbl = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness {
                left: 4.0,
                top: 0.0,
                right: 12.0,
                bottom: 0.0,
            }),
    )
    .with_role(TextRole::BodyStrong)
    .with_text("Somnium Engine")
    .build();
    ui.add_node(title_lbl, title_left_h);

    let title_right = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_column(2)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let title_right_h = ui.add_node(title_right, title_grid_h);
    let help_button = icon_tool_button(ui, title_right_h, IconId::HelpCircle, "Help (F1)");
    let fps_node = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(8.0, 0.0)),
    )
    .with_role(TextRole::Mono)
    .with_text("— fps")
    .build();
    let fps_text = ui.add_node(fps_node, title_right_h);
    let win_min = window_chrome_button(ui, title_right_h, IconId::Minimize, "Minimize");
    let win_max = window_chrome_button(ui, title_right_h, IconId::Maximize, "Maximize");
    let win_close = window_chrome_button(ui, title_right_h, IconId::Close, "Close");

    // ── Application menus — folded into the title/application band ──────────
    let menu_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(1)
            .with_background(theme::TRANSPARENT)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let menu_bar_h = ui.add_node(menu_bar, title_grid_h);

    // Menu bar grid: the application commands consume the available centre.
    let menu_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::auto())
        .add_column(Column::stretch())
        .build();
    let menu_grid_h = ui.add_node(menu_grid, menu_bar_h);

    // Horizontal stack for menu items (col 0)
    let menu_stack = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let menu_stack_h = ui.add_node(menu_stack, menu_grid_h);

    // "File" — Menu so clicks are captured (holds Import).
    let file_btn_node =
        MenuBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let file_button = ui.add_node(file_btn_node, menu_stack_h);
    let file_lbl = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(9.0, 0.0)),
    )
    .with_role(TextRole::Body)
    .with_text("File")
    .build();
    ui.add_node(file_lbl, file_button);

    let edit_button = menu_button(ui, menu_stack_h, "Edit", font_id);

    // "Create" — Menu so clicks are captured
    let create_btn_node = MenuBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness {
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            })
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let create_button = ui.add_node(create_btn_node, menu_stack_h);
    let create_lbl = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(9.0, 0.0)),
    )
    .with_role(TextRole::Body)
    .with_text("Create")
    .build();
    ui.add_node(create_lbl, create_button);

    let view_button = menu_button(ui, menu_stack_h, "View", font_id);
    let window_button = menu_button(ui, menu_stack_h, "Window", font_id);
    let help_menu_button = menu_button(ui, menu_stack_h, "Help", font_id);

    let palette_button_node = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(1)
            .with_width(320.0)
            .with_height(26.0)
            .with_horizontal_alignment(HorizontalAlignment::Right)
            .with_margin(Thickness {
                left: 12.0,
                top: 5.0,
                right: 12.0,
                bottom: 5.0,
            })
            .with_background(theme::BG_INPUT)
            .with_tooltip("Search commands, entities, assets (Ctrl+P)"),
    )
    .build();
    let palette_button = ui.add_node(palette_button_node, menu_grid_h);
    let palette_label = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 10.0,
        top: 5.0,
        right: 10.0,
        bottom: 0.0,
    }))
    .with_text("Search commands, entities, assets     Ctrl+P")
    .with_font_size(theme::NOCTURNE.typography.caption)
    .with_font_id(font_id)
    .with_color(theme::TEXT_MUTED)
    .build();
    ui.add_node(palette_label, palette_button);

    // ── Row 2: main toolbar ──────────────────────────────────────────────────
    let main_tb = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let main_tb_h = ui.add_node(main_tb, outer_h);
    let main_tb_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
    let main_tb_stack_h = ui.add_node(main_tb_stack, main_tb_h);
    // Nocturne Atelier §01: "icon-only controls without tooltips" is a
    // forbidden motif, and phase_26 §2.4 already required recognition over
    // recall for the mode commands. Save and the three editing modes carry
    // their names; only the transport triple stays glyph-only, because ▶ ❚❚ ■
    // are the one set of symbols every DCC user already reads, and the mode
    // scope says "Stopped"/"Playing" beside them in words regardless.
    let (save_button, save_label) = labeled_icon_button(
        ui,
        main_tb_stack_h,
        IconId::Save,
        "Save",
        "Save scene (Ctrl+S)",
        font_id,
        theme::NOCTURNE.density.row_chrome,
    );
    scope_separator(ui, main_tb_stack_h);
    let (select_button, select_label) = labeled_icon_button(
        ui,
        main_tb_stack_h,
        IconId::Select,
        "Select",
        "Select and transform entities",
        font_id,
        theme::NOCTURNE.density.row_chrome,
    );
    let (landscape_button, landscape_label) = labeled_icon_button(
        ui,
        main_tb_stack_h,
        IconId::Landscape,
        "Landscape",
        "Terrain sculpt and paint (F6)",
        font_id,
        theme::NOCTURNE.density.row_chrome,
    );
    let (foliage_toolbar_button, foliage_mode_label) = labeled_icon_button(
        ui,
        main_tb_stack_h,
        IconId::Foliage,
        "Foliage",
        "Foliage placement (F8)",
        font_id,
        theme::NOCTURNE.density.row_chrome,
    );
    scope_separator(ui, main_tb_stack_h);
    let play_button = icon_tool_button(ui, main_tb_stack_h, IconId::Play, "Play (simulate)");
    let immersive_button = icon_tool_button(
        ui,
        main_tb_stack_h,
        IconId::ImmersivePlay,
        "Immersive play (Esc to exit)",
    );
    let pause_button = icon_tool_button(ui, main_tb_stack_h, IconId::Pause, "Pause");
    let stop_button = icon_tool_button(ui, main_tb_stack_h, IconId::Stop, "Stop");
    let play_label_n = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(10.0, 0.0)),
    )
    .with_role(TextRole::Caption)
    .with_text("Stopped")
    .build();
    let play_label = ui.add_node(play_label_n, main_tb_stack_h);
    let pause_label = play_label;
    let stop_label = play_label;

    // ── Row 4: resizable columns — tools | viewport | details ────────────────
    let tools_split = SplitterBuilder::new(
        WidgetBuilder::new()
            .with_row(4)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(SplitterOrientation::Horizontal)
    .with_first_size(layout.tools)
    .with_min_first(120.0)
    .with_min_second(240.0)
    .build();
    let inner_h = ui.add_node(tools_split, outer_h);

    // Left toolbar strip
    let toolbar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 1.0,
        top: 0.0,
        bottom: 0.0,
    })
    .build();
    let toolbar_h = ui.add_node(toolbar, inner_h);

    // Redline §06: Details is min 240 / default 340. 180 was narrow enough that
    // every property row hit the stacking rule and the panel looked broken.
    let content_split =
        SplitterBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(SplitterOrientation::Horizontal)
            .with_first_size(layout.viewport)
            .with_min_first(200.0)
            .with_min_second(240.0)
            .build();
    let content_split_h = ui.add_node(content_split, inner_h);

    // Terrain tool palette (Phase 14F): label + 6 brush mode buttons.
    // Active only while a terrain entity is selected (F6 toggles edit mode).
    let tool_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let tool_stack_h = ui.add_node(tool_stack, toolbar_h);

    let ter_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 8.0,
        right: 0.0,
        bottom: 4.0,
    }))
    .with_role(TextRole::SectionCaps)
    .with_text("Sculpt")
    .build();
    ui.add_node(ter_lbl, tool_stack_h);

    const TERRAIN_TOOLS: &[(IconId, &str, u8)] = &[
        (IconId::SculptRaise, "Raise", 0),
        (IconId::SculptLower, "Lower", 1),
        (IconId::SculptSmooth, "Smooth", 2),
        (IconId::SculptFlatten, "Flatten", 3),
        (IconId::SculptNoise, "Noise", 4),
        (IconId::PaintLayer, "Paint", 5),
    ];
    let mut terrain_tool_items = Vec::with_capacity(TERRAIN_TOOLS.len());
    for &(icon, label, tool) in TERRAIN_TOOLS {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(28.0)
                .with_margin(Thickness {
                    left: 4.0,
                    top: 2.0,
                    right: 4.0,
                    bottom: 0.0,
                })
                .with_background(theme::BG_RAISED),
        )
        .build();
        let btn_h = ui.add_node(btn, tool_stack_h);
        let row = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
        let row_h = ui.add_node(row, btn_h);
        let img = ImageBuilder::new(
            WidgetBuilder::new()
                .with_vertical_alignment(VerticalAlignment::Center)
                .with_margin(Thickness {
                    left: 6.0,
                    top: 0.0,
                    right: 6.0,
                    bottom: 0.0,
                }),
        )
        .with_icon(icon)
        .with_size(16.0)
        .with_tint(theme::TEXT_PRIMARY)
        .build();
        ui.add_node(img, row_h);
        let lbl = TextBuilder::new(
            WidgetBuilder::new()
                .with_vertical_alignment(VerticalAlignment::Center)
                .with_margin(Thickness {
                    left: 0.0,
                    top: 0.0,
                    right: 6.0,
                    bottom: 0.0,
                }),
        )
        .with_role(TextRole::Label)
        .with_text(label)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        let lbl_h = ui.add_node(lbl, row_h);
        terrain_tool_items.push((btn_h, lbl_h, tool));
    }

    // Viewport area (col 1) — transparent, no hit-test. Mouse events in this region
    // will hit-test to this handle, which the UI knows to NOT consume.
    let viewport_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::TRANSPARENT)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let viewport_handle = ui.add_node(viewport_border, content_split_h);

    // ── Viewport context scope — floats over the render ─────────────────────
    //
    // Zeta redline §06: the third command scope is not a fourth horizontal
    // band. It is a 32 px bar inset 12 px over the viewport, so the scene
    // starts 68 px from the top instead of 100, and camera / shading / snap /
    // profiler controls sit next to the thing they change. Being a child of
    // the viewport also means it is hit-tested normally while the transparent
    // region around it still passes clicks through to the 3D pick.
    let vp_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_height(theme::NOCTURNE.density.toolbar)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_margin(Thickness::uniform(12.0))
            // Translucent over the render so the scene stays readable behind
            // the bar; the hairline is what keeps it legible on a bright sky.
            .with_background(theme::with_alpha(theme::BG_VOID, 0xB8))
            .with_foreground(theme::BORDER_MEDIUM),
    )
    .with_stroke_thickness(Thickness::uniform(theme::NOCTURNE.geometry.stroke_hairline))
    .build();
    let vp_bar_h = ui.add_node(vp_bar, viewport_handle);

    let vp_stack = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Horizontal)
        .build();
    let vp_stack_h = ui.add_node(vp_stack, vp_bar_h);

    let cam_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 10.0,
        top: 6.0,
        right: 6.0,
        bottom: 0.0,
    }))
    .with_role(TextRole::Label)
    .with_text("Camera Speed")
    .build();
    ui.add_node(cam_lbl, vp_stack_h);

    let cam_slider_node = SliderBuilder::new(
        WidgetBuilder::new()
            .with_width(140.0)
            .with_tooltip("Camera speed - also RMB + scroll wheel in the viewport")
            .with_margin(Thickness {
                left: 0.0,
                top: 4.0,
                right: 8.0,
                bottom: 0.0,
            }),
    )
    .with_value(0.5)
    .build();
    let camera_speed_slider = ui.add_node(cam_slider_node, vp_stack_h);

    // Numeric readout, updated as the slider (or RMB+wheel) changes speed.
    let cam_val = TextBuilder::new(
        WidgetBuilder::new()
            .with_width(84.0)
            .with_margin(Thickness {
                left: 0.0,
                top: 6.0,
                right: 0.0,
                bottom: 0.0,
            }),
    )
    .with_role(TextRole::MonoStrong)
    .with_text("5.0 m/s")
    .build();
    let camera_speed_label = ui.add_node(cam_val, vp_stack_h);

    // Play/Pause/Stop live on the main toolbar (Phase 26-C).

    // Phase 29: the profiler switch lives on the viewport toolbar rather than
    // in a menu, because it is a thing you flick on and off while looking at
    // the scene — the same reason UE5 puts its stat toggles there.
    let (profiler_toggle, profiler_toggle_lbl) = labeled_icon_button(
        ui,
        vp_stack_h,
        IconId::Profiler,
        "Profiler",
        "Toggle GPU profiler overlay",
        font_id,
        22.0,
    );

    let res_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 14.0,
        top: 6.0,
        right: 6.0,
        bottom: 0.0,
    }))
    .with_role(TextRole::Label)
    .with_text("Resolution")
    .build();
    ui.add_node(res_lbl, vp_stack_h);

    let viewport_res_combo_node = ComboBoxBuilder::new(
        WidgetBuilder::new()
            .with_width(118.0)
            .with_margin(Thickness {
                left: 0.0,
                top: 2.0,
                right: 8.0,
                bottom: 0.0,
            }),
    )
    .with_items(VIEWPORT_RESOLUTION_NAMES)
    .with_selected(0)
    .with_font_id(font_id)
    .build();
    let viewport_res_combo = ui.add_node(viewport_res_combo_node, vp_stack_h);

    // ── Profiler overlay (Phase 29) ──────────────────────────────────────────
    // A child of the viewport, pinned top-left, so it floats over the render
    // instead of stealing layout from it. Rows are built once and rewritten
    // each frame: allocating twenty text nodes per frame to display a frame
    // timing would be its own entry in the table.
    let prof_panel = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(300.0)
            // Below the floating context bar (12 inset + 32 bar + 12 gap).
            .with_margin(Thickness {
                left: 12.0,
                top: 56.0,
                right: 0.0,
                bottom: 0.0,
            })
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let profiler_panel = ui.add_node(prof_panel, viewport_handle);

    let prof_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let prof_stack_h = ui.add_node(prof_stack, profiler_panel);

    let prof_hdr = TextBuilder::new(WidgetBuilder::new().with_height(18.0).with_margin(
        Thickness {
            left: 8.0,
            top: 4.0,
            right: 0.0,
            bottom: 2.0,
        },
    ))
    .with_role(TextRole::SectionCaps)
    .with_text("GPU PROFILER")
    .build();
    ui.add_node(prof_hdr, prof_stack_h);

    let mut profiler_names = Vec::with_capacity(PROFILER_ROWS);
    let mut profiler_values = Vec::with_capacity(PROFILER_ROWS);
    for _ in 0..PROFILER_ROWS {
        let row = StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_height(15.0)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Horizontal)
        .build();
        let row_h = ui.add_node(row, prof_stack_h);

        let name = TextBuilder::new(WidgetBuilder::new().with_width(190.0).with_margin(
            Thickness {
                left: 8.0,
                top: 1.0,
                right: 0.0,
                bottom: 0.0,
            },
        ))
        .with_text("")
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_SECONDARY)
        .build();
        profiler_names.push(ui.add_node(name, row_h));

        let value = TextBuilder::new(WidgetBuilder::new().with_width(92.0).with_margin(
            Thickness {
                left: 0.0,
                top: 1.0,
                right: 0.0,
                bottom: 0.0,
            },
        ))
        .with_text("")
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        profiler_values.push(ui.add_node(value, row_h));
    }
    ui.set_visibility(profiler_panel, false);

    // Right panel: two sections (outliner top, inspector bottom)
    let right_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 1.0,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
    })
    .build();
    let right_h = ui.add_node(right_border, content_split_h);

    let right_split =
        SplitterBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(SplitterOrientation::Vertical)
            .with_first_size(layout.outliner)
            .with_min_first(80.0)
            .with_min_second(120.0)
            .build();
    let right_split_h = ui.add_node(right_split, right_h);

    let out_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(24.0))
        .add_row(Row::strict(22.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let out_grid_h = ui.add_node(out_grid, right_split_h);

    let ins_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(24.0))
        .add_row(Row::strict(22.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let ins_grid_h = ui.add_node(ins_grid, right_split_h);

    // Outliner header
    let out_hdr = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let out_hdr_h = ui.add_node(out_hdr, out_grid_h);
    let out_hdr_txt = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 5.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_role(TextRole::SectionCaps)
    .with_text("OUTLINER")
    .build();
    ui.add_node(out_hdr_txt, out_hdr_h);

    let outliner_search = {
        let n = SearchBoxBuilder::new(
            WidgetBuilder::new()
                .with_row(1)
                .with_column(0)
                .with_background(theme::BG_INPUT),
        )
        .with_font_id(font_id)
        .build();
        ui.add_node(n, out_grid_h)
    };

    let out_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_DARK),
    )
    .build();
    let outliner_scroll = ui.add_node(out_scroll, out_grid_h);

    let outliner_tree = {
        let t = TreeViewBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_font_id(font_id)
            .build();
        ui.add_node(t, outliner_scroll)
    };
    let outliner_stack = outliner_tree;

    // Inspector header
    let ins_hdr = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
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
    let ins_hdr_h = ui.add_node(ins_hdr, ins_grid_h);
    let ins_hdr_txt = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 5.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_role(TextRole::SectionCaps)
    .with_text("DETAILS")
    .build();
    ui.add_node(ins_hdr_txt, ins_hdr_h);

    let inspector_search = {
        let n = SearchBoxBuilder::new(
            WidgetBuilder::new()
                .with_row(1)
                .with_column(0)
                .with_background(theme::BG_INPUT),
        )
        .with_font_id(font_id)
        .build();
        ui.add_node(n, ins_grid_h)
    };

    let ins_content = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_DARK),
    )
    .build();
    let ins_content_h = ui.add_node(ins_content, ins_grid_h);

    let inspector_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let inspector_stack = ui.add_node(inspector_stack, ins_content_h);

    let inspector_handles = build_inspector(ui, inspector_stack, font_id);

    // ── Row 5: docked Content Drawer / Output Log ────────────────────────────
    let bottom = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(5)
            .with_column(0)
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 1.0,
        bottom: 0.0,
    })
    .build();
    let bottom_h = ui.add_node(bottom, outer_h);

    let bottom_swap = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let bottom_swap_h = ui.add_node(bottom_swap, bottom_h);

    let (content_drawer, content_search, content_breadcrumb, content_engine_toggle, content_list) =
        build_content_drawer(ui, bottom_swap_h, font_id);

    let log_panel = GridBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::TRANSPARENT)
            .with_visibility(false),
    )
    .add_row(Row::strict(22.0))
    .add_row(Row::stretch())
    .add_column(Column::stretch())
    .build();
    let log_panel = ui.add_node(log_panel, bottom_swap_h);

    let log_hdr_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let log_hdr_h = ui.add_node(log_hdr_border, log_panel);

    let log_header = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 4.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_role(TextRole::SectionCaps)
    .with_text("Output Log")
    .build();
    ui.add_node(log_header, log_hdr_h);

    let log_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::BG_DARK),
    )
    .build();
    let log_scroll_h = ui.add_node(log_scroll, log_panel);

    let log_stack_node =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let log_stack = ui.add_node(log_stack_node, log_scroll_h);

    // ── Row 6: status bar ────────────────────────────────────────────────────
    let status_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(6)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 1.0,
        bottom: 0.0,
    })
    .build();
    let status_h = ui.add_node(status_bar, outer_h);
    // Two clusters: drawer/log entry points and live state on the left, scene
    // statistics right-aligned. Redline §06 drops the right cluster's items
    // right to left as width runs out; FPS and dirty state never drop, which is
    // why they sit at the two ends rather than in the middle.
    let status_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::auto())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let status_grid_h = ui.add_node(status_grid, status_h);
    let status_stack = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let status_stack_h = ui.add_node(status_stack, status_grid_h);
    let (drawer_button, _) = labeled_icon_button(
        ui,
        status_stack_h,
        IconId::ContentDrawer,
        "Content Drawer",
        "Show or hide the Content Drawer (Ctrl+Space)",
        font_id,
        theme::STATUS_HEIGHT,
    );
    let (log_button, _) = labeled_icon_button(
        ui,
        status_stack_h,
        IconId::OutputLog,
        "Output Log",
        "Show or hide the Output Log",
        font_id,
        theme::STATUS_HEIGHT,
    );
    scope_separator(ui, status_stack_h);
    // Save state. Sentinel colour is set by `set_scene_dirty`; the *word*
    // carries the meaning so the state is not colour-only (§10.3).
    let status_dirty_n = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(6.0, 0.0)),
    )
    .with_role(TextRole::Caption)
    .with_text("Saved")
    .build();
    let status_dirty = ui.add_node(status_dirty_n, status_stack_h);
    scope_separator(ui, status_stack_h);
    let status_sel_n = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(6.0, 0.0)),
    )
    .with_role(TextRole::Caption)
    .with_text("No selection")
    .build();
    let status_selection = ui.add_node(status_sel_n, status_stack_h);
    // Which panel the drawer row is showing. Kept from the pre-Zeta status bar
    // because Ctrl+Space toggling between two panels needs a readout.
    // Empty at startup because the drawer row starts open; `apply_bottom_panel`
    // fills it with "Ready" once both panels are closed.
    let status_lbl = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(8.0, 0.0)),
    )
    .with_role(TextRole::Caption)
    .with_text("")
    .build();
    let status_text = ui.add_node(status_lbl, status_stack_h);

    let status_stats_stack = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_column(2)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let status_stats_stack_h = ui.add_node(status_stats_stack, status_grid_h);
    let status_stats_n = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(10.0, 0.0))
            .with_tooltip("Scene objects and frame rate"),
    )
    .with_role(TextRole::Mono)
    .with_text("— objects · — fps")
    .build();
    let status_stats = ui.add_node(status_stats_n, status_stats_stack_h);

    // ── Popup overlays (children of root, drawn on top) ───────────────────────
    let (create_popup, create_popup_items) = build_create_popup(ui, root, font_id);
    let (file_popup, file_items) = popup_items(
        ui,
        root,
        font_id,
        &["New Scene", "Save Scene", "Import Model..."],
    );
    let file_new_item = file_items[0];
    let file_save_item = file_items[1];
    let file_import_item = file_items[2];
    let (edit_popup, edit_items) =
        popup_items(ui, root, font_id, &["Undo", "Redo", "Delete", "Duplicate"]);
    let edit_undo = edit_items[0];
    let edit_redo = edit_items[1];
    let edit_delete = edit_items[2];
    let edit_dup = edit_items[3];
    let (view_popup, view_items) = popup_items(ui, root, font_id, &["Profiler", "Content Drawer"]);
    let view_profiler = view_items[0];
    let view_content = view_items[1];
    // Window menu is now the workspace switcher (Zeta-F). "Reset workspace"
    // last, after a separator's worth of distance, because it is the
    // destructive one.
    let (window_popup, window_items) = popup_items(
        ui,
        root,
        font_id,
        &[
            "Show Content Drawer",
            "Workspace: Layout",
            "Workspace: Terrain",
            "Workspace: Foliage",
            "Workspace: Lighting",
            "Workspace: Materials",
            "Workspace: Debug",
            "Workspace: Play",
            "Reset workspace",
        ],
    );
    let window_dock_content = window_items[0];
    let window_workspace_items: Vec<NodeHandle> = window_items[1..8].to_vec();
    let window_reset_item = window_items[8];
    let (help_menu_popup, help_items) = popup_items(
        ui,
        root,
        font_id,
        &["Help Overlay (F1)", "Shortcuts", "About"],
    );
    let help_open_item = help_items[0];
    let help_shortcuts = help_items[1];
    let help_about = help_items[2];

    let (help_overlay, help_body, help_toc, help_close) = build_help_overlay(ui, root, font_id);

    let tooltip_node = TooltipBuilder::new(
        WidgetBuilder::new()
            .with_visibility(false)
            .with_desired_position(Vec2::new(0.0, 0.0)),
    )
    .with_font_id(font_id)
    .build();
    let tooltip = ui.add_node(tooltip_node, root);

    // ── Content Drawer context menu (right-click) ────────────────────────────
    //
    // Wrapped in a `Popup` so clicking away closes it, and placed with
    // `AnchorBelow` *without* an anchor — which is the placement's
    // "obey the child's desired position" path, and is how the menu
    // lands at the cursor. `UiManager` sets that position when it opens.
    //
    // The item list is rebuilt per open: what you may do depends on
    // whether you right-clicked a file, a folder or empty space.
    let content_menu_popup_node =
        PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let content_menu_popup = ui.add_node(content_menu_popup_node, root);
    let content_menu_node = ContextMenuBuilder::new(
        WidgetBuilder::new()
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_desired_position(Vec2::ZERO),
    )
    .with_items(Vec::new())
    .with_font_id(font_id)
    .build();
    let content_menu = ui.add_node(content_menu_node, content_menu_popup);


    let palette_items = vec![
        PaletteItem {
            label: "New Scene".into(),
            hint: "Ctrl+N".into(),
        },
        PaletteItem {
            label: "Save Scene".into(),
            hint: "Ctrl+S".into(),
        },
        PaletteItem {
            label: "Import Model…".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Undo".into(),
            hint: "Ctrl+Z".into(),
        },
        PaletteItem {
            label: "Redo".into(),
            hint: "Ctrl+Y".into(),
        },
        PaletteItem {
            label: "Delete".into(),
            hint: "Del".into(),
        },
        PaletteItem {
            label: "Duplicate".into(),
            hint: "Ctrl+D".into(),
        },
        PaletteItem {
            label: "Play".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Pause".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Stop".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Toggle Profiler".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Content Drawer".into(),
            hint: "Ctrl+Space".into(),
        },
        PaletteItem {
            label: "Help".into(),
            hint: "F1".into(),
        },
        PaletteItem {
            label: "Create Cube".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Create Directional Light".into(),
            hint: String::new(),
        },
    ];
    let palette_popup_node = PopupBuilder::new(
        WidgetBuilder::new().with_background(theme::NOCTURNE.semantic.surface.modal_scrim.bytes()),
    )
    .with_placement(PopupPlacement::Center)
    .build();
    let palette_popup = ui.add_node(palette_popup_node, root);
    let palette_widget_node = CommandPaletteBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_HEADER)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center),
    )
    .with_font_id(font_id)
    .with_items(palette_items)
    .build();
    let palette_widget = ui.add_node(palette_widget_node, palette_popup);

    let toast_node = ToastHostBuilder::new(WidgetBuilder::new())
        .with_font_id(font_id)
        .build();
    let toast_host = ui.add_node(toast_node, root);

    let unsaved_popup_node = PopupBuilder::new(
        WidgetBuilder::new().with_background(theme::NOCTURNE.semantic.surface.modal_scrim.bytes()),
    )
    .with_placement(PopupPlacement::Center)
    .build();
    let unsaved_popup = ui.add_node(unsaved_popup_node, root);
    let unsaved_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(340.0)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let unsaved_border_h = ui.add_node(unsaved_border, unsaved_popup);
    let unsaved_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let unsaved_stack_h = ui.add_node(unsaved_stack, unsaved_border_h);
    let unsaved_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(12.0)))
        .with_role(TextRole::Body)
        .with_text("Save changes to the current scene?")
        .build();
    ui.add_node(unsaved_lbl, unsaved_stack_h);
    let unsaved_row =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
    let unsaved_row_h = ui.add_node(unsaved_row, unsaved_stack_h);
    let mk_modal_btn = |ui: &mut UserInterface, label: &str, parent: NodeHandle| {
        let b = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_width(100.0)
                .with_height(24.0)
                .with_margin(Thickness::axes(8.0, 8.0))
                .with_background(theme::BG_RAISED),
        )
        .build();
        let h = ui.add_node(b, parent);
        let t = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
            .with_text(label)
            .with_font_id(font_id)
            .with_font_size(12.0)
            .with_color(theme::TEXT_PRIMARY)
            .build();
        ui.add_node(t, h);
        h
    };
    let unsaved_save = mk_modal_btn(ui, "Save", unsaved_row_h);
    let unsaved_discard = mk_modal_btn(ui, "Don't Save", unsaved_row_h);
    let unsaved_cancel = mk_modal_btn(ui, "Cancel", unsaved_row_h);

    // ── Name prompt (Content Drawer) ─────────────────────────────────────────
    //
    // "New Folder", "New Script" and "Rename" all need a name, so one
    // modal serves all three: the caller sets the title and the initial
    // text, and confirming reports whatever is in the box. A creation
    // flow that named things `NewFolder1` and made you find a rename
    // afterwards would be the wrong shape.
    let name_popup_node = PopupBuilder::new(
        WidgetBuilder::new().with_background(theme::NOCTURNE.semantic.surface.modal_scrim.bytes()),
    )
    .with_placement(PopupPlacement::Center)
    .build();
    let name_popup = ui.add_node(name_popup_node, root);
    let name_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(360.0)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let name_border_h = ui.add_node(name_border, name_popup);
    let name_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let name_stack_h = ui.add_node(name_stack, name_border_h);
    let name_title_node =
        TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(12.0)))
            .with_role(TextRole::Body)
            .with_text("New Folder")
            .build();
    let name_title = ui.add_node(name_title_node, name_stack_h);
    let name_input_node = TextBoxBuilder::new(
        WidgetBuilder::new()
            .with_height(26.0)
            .with_margin(Thickness::axes(12.0, 4.0))
            .with_background(theme::BG_INPUT),
    )
    .with_font_id(font_id)
    .build();
    let name_input = ui.add_node(name_input_node, name_stack_h);
    let name_row = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Horizontal)
        .build();
    let name_row_h = ui.add_node(name_row, name_stack_h);
    let name_ok = mk_modal_btn(ui, "Create", name_row_h);
    let name_cancel = mk_modal_btn(ui, "Cancel", name_row_h);

    let color_popup_node =
        PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_placement(PopupPlacement::AnchorBelow)
            .build();
    let color_popup = ui.add_node(color_popup_node, root);
    let color_picker_node = ColorPickerBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_HEADER)
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top),
    )
    .with_font_id(font_id)
    .build();
    let color_picker = ui.add_node(color_picker_node, color_popup);

    let foliage_kind_combo = inspector_handles.foliage_kind_button;
    let post_tonemap_combo = inspector_handles.post_tonemap_button;
    let foliage_kind_popup =
        attach_combo_popup(ui, foliage_kind_combo, &FOLIAGE_KIND_NAMES, font_id);
    let post_tonemap_popup = attach_combo_popup(ui, post_tonemap_combo, &TONEMAP_NAMES, font_id);
    let viewport_res_popup =
        attach_combo_popup(ui, viewport_res_combo, &VIEWPORT_RESOLUTION_NAMES, font_id);

    EditorLayout {
        outliner_scroll,
        outliner_stack,
        inspector_stack,
        log_stack,
        create_button,
        create_popup,
        create_popup_items,
        file_button,
        file_popup,
        file_import_item,
        file_new_item,
        file_save_item,
        camera_speed_slider,
        camera_speed_label,
        viewport_res_combo,
        play_button,
        play_label,
        immersive_button,
        pause_button,
        pause_label,
        stop_button,
        stop_label,
        select_button,
        landscape_button,
        foliage_toolbar_button,
        terrain_tool_items,
        inspector_handles,
        viewport_handle,
        profiler_panel,
        profiler_toggle,
        profiler_toggle_lbl,
        profiler_names,
        profiler_values,
        outer_grid: outer_h,
        menu_bar_h,
        mode_labels: [
            save_label,
            select_label,
            landscape_label,
            foliage_mode_label,
        ],
        status_dirty,
        status_selection,
        status_stats,
        vp_bar_h,
        inner_h,
        content_split_h,
        right_split_h,
        toolbar_h,
        right_h,
        bottom_h,
        fps_text,
        help_button,
        help_overlay,
        help_body,
        tooltip,
        edit_button,
        view_button,
        window_button,
        help_menu_button,
        edit_popup,
        view_popup,
        window_popup,
        help_menu_popup,
        edit_undo,
        edit_redo,
        edit_delete,
        edit_dup,
        view_profiler,
        view_content,
        window_dock_content,
        window_workspace_items,
        window_reset_item,
        help_open_item,
        help_shortcuts,
        help_about,
        status_text,
        drawer_button,
        log_button,
        content_drawer,
        content_search,
        content_breadcrumb,
        content_engine_toggle,
        content_list,
        outliner_tree,
        outliner_search,
        inspector_search,
        foliage_kind_combo,
        post_tonemap_combo,
        foliage_kind_popup,
        post_tonemap_popup,
        viewport_res_popup,
        save_button,
        palette_button,
        palette_popup,
        palette_widget,
        toast_host,
        unsaved_popup,
        unsaved_save,
        unsaved_discard,
        unsaved_cancel,
        content_menu_popup,
        content_menu,
        name_popup,
        name_title,
        name_input,
        name_ok,
        name_cancel,
        color_popup,
        color_picker,
        title_drag,
        win_min,
        win_max,
        win_close,
        help_toc,
        help_close,
        log_panel,
    }
}
