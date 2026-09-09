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
    types::{Thickness, VerticalAlignment},
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        canvas::CanvasBuilder,
        check_box::CheckBoxBuilder,
        grid::{Column, GridBuilder, Row},
        scroll_viewer::ScrollViewerBuilder,
        search_box::{BreadcrumbBuilder, SearchBoxBuilder},
        stack_panel::{Orientation, StackPanelBuilder},
    },
};

// Glob the crate root for the shared handle bundles and name tables, and the
// sibling `parts` module for the small builders. Explicit imports above shadow
// the globs, so this cannot silently change which `TextBuilder` is in scope.
#[allow(unused_imports)]
use crate::editor::parts::*;

pub(crate) fn build_content_drawer(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
    persona: &mut super::persona::Persona,
) -> (
    NodeHandle,
    NodeHandle,
    NodeHandle,
    NodeHandle,
    NodeHandle,
    NodeHandle,
    Vec<(NodeHandle, crate::ContentToolbarAction)>,
) {
    use super::persona::{action, combo};
    let panel = ui.add_node(
        BorderBuilder::new(
            WidgetBuilder::new()
                .with_row(0)
                .with_background(theme::active().semantic.surface.panel.bytes()),
        )
        .build(),
        parent,
    );
    let grid = ui.add_node(
        GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .add_row(Row::strict(30.0))
            .add_row(Row::strict(30.0))
            .add_row(Row::strict(30.0))
            .add_row(Row::stretch())
            .add_column(Column::stretch())
            .build(),
        panel,
    );
    let nav = ui.add_node(
        GridBuilder::new(
            WidgetBuilder::new()
                .with_row(0)
                .with_background(theme::TRANSPARENT),
        )
        .add_row(Row::stretch())
        .add_column(Column::strict(108.0))
        .add_column(Column::stretch())
        .add_column(Column::strict(84.0))
        .add_column(Column::strict(200.0))
        .build(),
        grid,
    );
    let arrows = ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build(),
        nav,
    );
    let mut actions = Vec::new();
    for (label, verb) in [
        ("‹", crate::ContentToolbarAction::Back),
        ("›", crate::ContentToolbarAction::Forward),
        ("↑", crate::ContentToolbarAction::Up),
    ] {
        let arrow = action(ui, arrows, label, 34.0);
        let text = ui.first_child(arrow);
        if text.is_some() {
            ui.nodes
                .borrow_mut(text.transmute())
                .widget
                .horizontal_alignment = crate::types::HorizontalAlignment::Center;
        }
        actions.push((arrow, verb));
    }
    let crumb = ui.add_node(
        BreadcrumbBuilder::new(
            WidgetBuilder::new()
                .with_column(1)
                .with_background(theme::TRANSPARENT),
        )
        .with_parts(["Game"])
        .with_font_id(font_id)
        .build(),
        nav,
    );
    let favorite_host = ui.add_node(
        StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_column(2)
                .with_background(theme::TRANSPARENT),
        )
        .build(),
        nav,
    );
    persona.favorite = action(ui, favorite_host, "Favorite", 80.0);
    let places_host = ui.add_node(
        StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_column(3)
                .with_background(theme::TRANSPARENT),
        )
        .build(),
        nav,
    );
    (persona.places, persona.places_popup) = combo(ui, places_host, &["Game root"], 188.0, font_id);
    let search_row = ui.add_node(
        GridBuilder::new(
            WidgetBuilder::new()
                .with_row(1)
                .with_background(theme::TRANSPARENT),
        )
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .add_column(Column::strict(170.0))
        .build(),
        grid,
    );
    let search = ui.add_node(
        SearchBoxBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 1.0)))
            .with_font_id(font_id)
            .build(),
        search_row,
    );
    let engine = ui.add_node(
        CheckBoxBuilder::new(WidgetBuilder::new().with_column(1))
            .with_label("Engine content")
            .with_font_id(font_id)
            .with_font_size(12.0)
            .build(),
        search_row,
    );
    let toolbar = ui.add_node(
        StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_row(2)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Horizontal)
        .build(),
        grid,
    );
    for (label, kind) in [
        ("All", crate::metaphor::ContentFilterKind::All),
        ("Models", crate::metaphor::ContentFilterKind::Models),
        ("Textures", crate::metaphor::ContentFilterKind::Textures),
        ("Scripts", crate::metaphor::ContentFilterKind::Scripts),
    ] {
        actions.push((
            action(ui, toolbar, label, 76.0),
            crate::ContentToolbarAction::Kind(kind),
        ));
    }
    (persona.sort, persona.sort_popup) = combo(
        ui,
        toolbar,
        &["Name A–Z", "Type", "Largest first", "Newest first"],
        142.0,
        font_id,
    );
    (persona.size, persona.size_popup) = combo(
        ui,
        toolbar,
        &["Compact tiles", "Comfortable tiles", "Large tiles"],
        164.0,
        font_id,
    );
    ui.send(crate::widgets::combo_box::ComboBoxMessage::set_selected(
        persona.size,
        1,
    ));
    let scroll = ui.add_node(
        ScrollViewerBuilder::new(
            WidgetBuilder::new()
                .with_row(3)
                .with_background(theme::active().semantic.surface.canvas.bytes()),
        )
        .build(),
        grid,
    );
    let list = ui.add_node(
        CanvasBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::uniform(8.0))
                .with_vertical_alignment(VerticalAlignment::Top)
                .with_clip_to_bounds(false)
                .with_background(theme::TRANSPARENT),
        )
        .build(),
        scroll,
    );
    (panel, search, crumb, engine, scroll, list, actions)
}

/// Build the Create dropdown popup (initially hidden, child of root).
pub(crate) fn build_create_popup(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
) -> (NodeHandle, Vec<(NodeHandle, &'static str)>) {
    let (popup, _, items) = command_popup_items(ui, root, font_id, crate::commands::Menu::Create);
    (popup, items)
}
