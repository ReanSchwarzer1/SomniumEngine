//! The Content Drawer and the Create popup.
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
    types::{HorizontalAlignment, Thickness, VerticalAlignment},
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        button::ButtonBuilder,
        canvas::CanvasBuilder,
        check_box::CheckBoxBuilder,
        grid::{Column, GridBuilder, Row},
        popup::PopupBuilder,
        scroll_viewer::ScrollViewerBuilder,
        search_box::{BreadcrumbBuilder, SearchBoxBuilder},
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
    },
};

// Glob the crate root for the shared handle bundles and name tables, and the
// sibling `parts` module for the small builders. Explicit imports above shadow
// the globs, so this cannot silently change which `TextBuilder` is in scope.
#[allow(unused_imports)]
use crate::editor::parts::*;
use glam::Vec2;

pub(crate) fn build_content_drawer(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> (
    NodeHandle,
    NodeHandle,
    NodeHandle,
    NodeHandle,
    NodeHandle,
    NodeHandle,
    Vec<(NodeHandle, crate::ContentToolbarAction)>,
) {
    let panel = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_PANEL)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let panel_h = ui.add_node(panel, parent);

    let grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(26.0))
        .add_row(Row::strict(22.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let grid_h = ui.add_node(grid, panel_h);

    let search = SearchBoxBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_INPUT),
    )
    .with_font_id(font_id)
    .build();
    let search_h = ui.add_node(search, grid_h);

    let engine = CheckBoxBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(1)
            .with_margin(Thickness::axes(8.0, 2.0)),
    )
    .with_label("Show Engine Content")
    .with_font_id(font_id)
    .with_font_size(11.0)
    .build();
    let engine_h = ui.add_node(engine, grid_h);

    let toolbar = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let toolbar = ui.add_node(toolbar, grid_h);
    let mut toolbar_actions = Vec::new();
    for (label, action) in [
        ("‹", crate::ContentToolbarAction::Back),
        ("›", crate::ContentToolbarAction::Forward),
        ("↑", crate::ContentToolbarAction::Up),
        (
            "All",
            crate::ContentToolbarAction::Kind(crate::metaphor::ContentFilterKind::All),
        ),
        (
            "Models",
            crate::ContentToolbarAction::Kind(crate::metaphor::ContentFilterKind::Models),
        ),
        (
            "Textures",
            crate::ContentToolbarAction::Kind(crate::metaphor::ContentFilterKind::Textures),
        ),
        (
            "Scripts",
            crate::ContentToolbarAction::Kind(crate::metaphor::ContentFilterKind::Scripts),
        ),
        ("Sort", crate::ContentToolbarAction::Sort),
        ("Size", crate::ContentToolbarAction::Density),
    ] {
        let button = ButtonBuilder::new(WidgetBuilder::new().with_height(22.0)).build();
        let button = ui.add_node(button, toolbar);
        let text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(5.0, 3.0)))
            .with_text(label)
            .with_font_id(font_id)
            .with_font_size(10.0)
            .build();
        ui.add_node(text, button);
        toolbar_actions.push((button, action));
    }
    let crumb = BreadcrumbBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_parts(["Game"])
        .with_font_id(font_id)
        .build();
    let crumb_h = ui.add_node(crumb, toolbar);

    let list_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_CONTENT),
    )
    .build();
    let list_scroll_h = ui.add_node(list_scroll, grid_h);
    // MORROWIND-M. A canvas, not a wrap panel: the drawer places the tiles it
    // built at absolute positions, because with a folder of 40,000 assets only
    // the screenful in view exists as widgets and a flow layout would have to
    // be handed all of them to know where any of them go.
    let list = CanvasBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::uniform(8.0))
            // Top, not the default stretch: the canvas is given an explicit
            // height covering the whole folder, and a stretched child with an
            // explicit height is *centred* in the space it was handed — which
            // for a folder shorter than the drawer would float the tiles in
            // the middle of it.
            .with_vertical_alignment(VerticalAlignment::Top)
            // And so it must not clip: an empty folder is nought rows tall, and
            // a canvas that cropped to its own bounds would build the "this
            // folder is empty" panel and then crop it out of existence. The
            // scroll viewer above still clips, which is the clip that matters.
            .with_clip_to_bounds(false)
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let list_h = ui.add_node(list, list_scroll_h);

    (
        panel_h,
        search_h,
        crumb_h,
        engine_h,
        list_scroll_h,
        list_h,
        toolbar_actions,
    )
}

/// Build the Create dropdown popup (initially hidden, child of root).
pub(crate) fn build_create_popup(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
) -> (NodeHandle, Vec<(NodeHandle, &'static str)>) {
    let popup_backdrop =
        PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let popup_h = ui.add_node(popup_backdrop, root);

    let popup_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_desired_position(Vec2::new(148.0, 28.0))
            .with_width(160.0)
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let popup_border_h = ui.add_node(popup_border, popup_h);

    let popup_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let popup_stack_h = ui.add_node(popup_stack, popup_border_h);

    let commands = crate::commands::registry().menu(crate::commands::Menu::Create);
    let mut items = Vec::with_capacity(commands.len());
    for command in commands {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(22.0)
                .with_background(theme::TRANSPARENT),
        )
        .build();
        let btn_h = ui.add_node(btn, popup_stack_h);

        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 8.0,
            top: 4.0,
            right: 0.0,
            bottom: 0.0,
        }))
        .with_text(command.menu_label())
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        ui.add_node(lbl, btn_h);
        items.push((btn_h, command.id));
    }

    (popup_h, items)
}
