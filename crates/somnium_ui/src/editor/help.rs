//! The F1 Help overlay and its markdown-derived page bodies.
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
        grid::{Column, GridBuilder, Row},
        popup::{PopupBuilder, PopupPlacement},
        scroll_viewer::ScrollViewerBuilder,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
    },
};

// Glob the crate root for the shared handle bundles and name tables, and the
// sibling `parts` module for the small builders. Explicit imports above shadow
// the globs, so this cannot silently change which `TextBuilder` is in scope.
#[allow(unused_imports)]
use crate::editor::parts::*;

pub(crate) fn fill_help_body(ui: &mut UserInterface, parent: NodeHandle, font_id: u8, page: u8) {
    ui.clear_children(parent);
    for block in crate::metaphor::help_blocks(page) {
        match block {
            crate::metaphor::HelpBlock::Heading(text) => {
                let n = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 0.0,
                    top: 10.0,
                    right: 8.0,
                    bottom: 6.0,
                }))
                .with_text(text)
                .with_font_size(16.0)
                .with_font_id(font_id)
                .with_color(theme::active().semantic.accent.default.bytes())
                .with_wrap(true)
                .build();
                ui.add_node(n, parent);
            }
            crate::metaphor::HelpBlock::Paragraph(text) => {
                let n = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 0.0,
                    top: 0.0,
                    right: 8.0,
                    bottom: 8.0,
                }))
                .with_text(text)
                .with_font_size(13.0)
                .with_font_id(font_id)
                .with_color(theme::active().semantic.text.primary.bytes())
                .with_wrap(true)
                .build();
                ui.add_node(n, parent);
            }
            crate::metaphor::HelpBlock::Bullet(text) => {
                let n = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 12.0,
                    top: 0.0,
                    right: 8.0,
                    bottom: 4.0,
                }))
                .with_text(format!("• {text}"))
                .with_font_size(13.0)
                .with_font_id(font_id)
                .with_color(theme::active().semantic.text.primary.bytes())
                .with_wrap(true)
                .build();
                ui.add_node(n, parent);
            }
        }
    }
    if page == 0 {
        let heading = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 0.0,
            top: 14.0,
            right: 8.0,
            bottom: 6.0,
        }))
        .with_text("Commands")
        .with_font_size(16.0)
        .with_font_id(font_id)
        .with_color(theme::active().semantic.accent.default.bytes())
        .build();
        ui.add_node(heading, parent);
        for command in crate::commands::registry().commands() {
            let binding = command
                .default_binding
                .map(|chord| format!(" ({chord})"))
                .unwrap_or_default();
            let row = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                left: 12.0,
                top: 0.0,
                right: 8.0,
                bottom: 4.0,
            }))
            .with_text(format!("{}{} — {}", command.label, binding, command.help))
            .with_font_size(12.0)
            .with_font_id(font_id)
            .with_color(theme::active().semantic.text.primary.bytes())
            .with_wrap(true)
            .build();
            ui.add_node(row, parent);
        }
    }
}

pub(crate) fn build_help_overlay(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
) -> (NodeHandle, NodeHandle, Vec<(NodeHandle, u8)>, NodeHandle) {
    let overlay = PopupBuilder::new(WidgetBuilder::new().with_background([0x0E, 0x10, 0x14, 0xE0]))
        .with_placement(PopupPlacement::Center)
        .build();
    let overlay_h = ui.add_node(overlay, root);

    let card = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(860.0)
            .with_height(540.0)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_background(theme::active().semantic.surface.panel.bytes())
            .with_foreground(theme::active().semantic.border.subtle.bytes()),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let card_h = ui.add_node(card, overlay_h);

    let grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(36.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let grid_h = ui.add_node(grid, card_h);

    let header = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::active().semantic.surface.header.bytes())
            .with_foreground(theme::active().semantic.border.subtle.bytes()),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let header_h = ui.add_node(header, grid_h);
    let header_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let header_grid_h = ui.add_node(header_grid, header_h);
    let title = TextBuilder::new(WidgetBuilder::new().with_column(0).with_margin(Thickness {
        left: 12.0,
        top: 10.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_role(TextRole::Title)
    .with_text("Editor Help")
    .build();
    ui.add_node(title, header_grid_h);
    let close_col = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_column(1)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let close_col_h = ui.add_node(close_col, header_grid_h);
    let help_close = window_chrome_button(ui, close_col_h, IconId::Close, "Close Help");

    let body_grid = GridBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .add_row(Row::stretch())
    .add_column(Column::strict(168.0))
    .add_column(Column::stretch())
    .build();
    let body_grid_h = ui.add_node(body_grid, grid_h);

    let toc_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_column(0)
            .with_row(0)
            .with_background(theme::active().semantic.surface.header.bytes())
            .with_foreground(theme::active().semantic.border.subtle.bytes()),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 1.0,
        top: 0.0,
        bottom: 0.0,
    })
    .build();
    let toc_border_h = ui.add_node(toc_border, body_grid_h);
    let toc_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let toc_stack_h = ui.add_node(toc_stack, toc_border_h);
    let mut help_toc = Vec::new();
    for (i, title) in crate::metaphor::help_titles().iter().enumerate() {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(28.0)
                .with_margin(Thickness::axes(6.0, 2.0))
                .with_background(theme::active().semantic.surface.raised.bytes()),
        )
        .build();
        let bh = ui.add_node(btn, toc_stack_h);
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(10.0, 6.0)))
            .with_text(*title)
            .with_font_size(12.0)
            .with_font_id(font_id)
            .with_color(theme::active().semantic.text.primary.bytes())
            .build();
        ui.add_node(lbl, bh);
        help_toc.push((bh, i as u8));
    }

    let scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_column(1)
            .with_row(0)
            .with_background(theme::active().semantic.surface.panel.bytes()),
    )
    .build();
    let scroll_h = ui.add_node(scroll, body_grid_h);
    let body = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::uniform(16.0))
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let body_h = ui.add_node(body, scroll_h);
    fill_help_body(ui, body_h, font_id, 0);
    (overlay_h, body_h, help_toc, help_close)
}
