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
    editor_event::CreateKind,
    message::NodeHandle,
    theme,
    types::{HorizontalAlignment, Thickness, VerticalAlignment},
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        button::ButtonBuilder,
        check_box::CheckBoxBuilder,
        grid::{Column, GridBuilder, Row},
        popup::PopupBuilder,
        scroll_viewer::ScrollViewerBuilder,
        search_box::{BreadcrumbBuilder, SearchBoxBuilder},
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        wrap_panel::WrapPanelBuilder,
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
) -> (NodeHandle, NodeHandle, NodeHandle, NodeHandle, NodeHandle) {
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

    let crumb = BreadcrumbBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_parts(["Game"])
    .with_font_id(font_id)
    .build();
    let crumb_h = ui.add_node(crumb, grid_h);

    let list_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_CONTENT),
    )
    .build();
    let list_scroll_h = ui.add_node(list_scroll, grid_h);
    let list = WrapPanelBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::uniform(8.0))
            .with_background(theme::TRANSPARENT),
    )
    .with_gap(10.0, 10.0)
    .build();
    let list_h = ui.add_node(list, list_scroll_h);

    (panel_h, search_h, crumb_h, engine_h, list_h)
}

/// Build the Create dropdown popup (initially hidden, child of root).
pub(crate) fn build_create_popup(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
) -> (NodeHandle, Vec<(NodeHandle, CreateKind)>) {
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
        let crate::commands::CommandAction::CreateEntity(kind) = command.action else {
            continue;
        };
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
        items.push((btn_h, kind));
    }

    (popup_h, items)
}
