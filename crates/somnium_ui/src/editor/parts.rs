//! Small shared builders — menu buttons, popups, toggles, separators.
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
        combo_box::{ComboBoxMessage, ComboDropdownBuilder},
        image::ImageBuilder,
        menu::MenuBuilder,
        popup::{PopupBuilder, PopupPlacement},
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
    },
};

// Glob the crate root for the shared handle bundles and name tables, and the
// sibling `parts` module for the small builders. Explicit imports above shadow
// the globs, so this cannot silently change which `TextBuilder` is in scope.

pub(crate) fn attach_combo_popup(
    ui: &mut UserInterface,
    combo: NodeHandle,
    items: &[&str],
    font_id: u8,
) -> NodeHandle {
    let popup = PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_anchor(combo)
        .with_placement(PopupPlacement::AnchorBelow)
        .build();
    let popup_h = ui.add_node(popup, ui.root());
    let list = ComboDropdownBuilder::new(WidgetBuilder::new())
        .with_items(items.iter().copied())
        .with_combo(combo)
        .with_popup(popup_h)
        .with_font_id(font_id)
        .build();
    let list_h = ui.add_node(list, popup_h);
    ui.send(ComboBoxMessage::bind_popup(combo, popup_h, list_h));
    popup_h
}

pub(crate) fn menu_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    label: &str,
    font_id: u8,
) -> NodeHandle {
    let _ = font_id;
    let node = MenuBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let h = ui.add_node(node, parent);
    let lbl = TextBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(9.0, 0.0)),
    )
    .with_role(TextRole::Body)
    .with_text(label)
    .build();
    ui.add_node(lbl, h);
    h
}

pub(crate) fn command_popup_items(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
    menu: crate::commands::Menu,
) -> (NodeHandle, NodeHandle, Vec<(NodeHandle, &'static str)>) {
    let commands = crate::commands::registry().menu(menu);
    let popup = PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let popup_h = ui.add_node(popup, root);
    let border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(200.0)
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let border_h = ui.add_node(border, popup_h);
    let stack = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Vertical)
        .build();
    let stack_h = ui.add_node(stack, border_h);
    let mut handles = Vec::with_capacity(commands.len());
    for command in commands {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(22.0)
                .with_background(theme::TRANSPARENT),
        )
        .build();
        let bh = ui.add_node(btn, stack_h);
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
            .with_text(command.menu_label())
            .with_font_size(12.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_PRIMARY)
            .build();
        ui.add_node(lbl, bh);
        handles.push((bh, command.id));
    }
    (popup_h, stack_h, handles)
}

/// A hairline between two groups inside one command scope.
///
/// The scopes are separated vertically by their own bands; within a band the
/// groups (save · modes · transport) need a seam, not a gap, or the strip reads
/// as one undifferentiated row of glyphs.
pub(crate) fn scope_separator(ui: &mut UserInterface, parent: NodeHandle) -> NodeHandle {
    let sep = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(theme::NOCTURNE.geometry.stroke_hairline)
            .with_height(theme::NOCTURNE.density.icon_action)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(theme::NOCTURNE.geometry.inset_panel, 0.0))
            .with_hit_test_visibility(false)
            .with_background(theme::BORDER_MEDIUM)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    ui.add_node(sep, parent)
}

pub(crate) fn icon_tool_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    icon: IconId,
    tooltip: &str,
) -> NodeHandle {
    let mut wb = WidgetBuilder::new()
        .with_width(36.0)
        .with_height(theme::TOOLBAR_HEIGHT)
        .with_margin(Thickness::axes(2.0, 2.0))
        .with_background(theme::BG_RAISED);
    if !tooltip.is_empty() {
        wb = wb.with_tooltip(tooltip);
    }
    let btn = ButtonBuilder::new(wb).build();
    let h = ui.add_node(btn, parent);
    let img = ImageBuilder::new(
        WidgetBuilder::new()
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center),
    )
    .with_icon(icon)
    .with_size(theme::ICON_TOOL)
    .with_tint(theme::TEXT_PRIMARY)
    .build();
    ui.add_node(img, h);
    h
}

pub(crate) fn window_chrome_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    icon: IconId,
    tooltip: &str,
) -> NodeHandle {
    let btn = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_width(46.0)
            .with_height(theme::TITLEBAR_HEIGHT)
            .with_tooltip(tooltip)
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let h = ui.add_node(btn, parent);
    let img = ImageBuilder::new(WidgetBuilder::new())
        .with_icon(icon)
        .with_size(16.0)
        .with_tint(theme::TEXT_PRIMARY)
        .build();
    ui.add_node(img, h);
    h
}

pub(crate) fn labeled_icon_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    icon: IconId,
    label: &str,
    tooltip: &str,
    font_id: u8,
    height: f32,
) -> (NodeHandle, NodeHandle) {
    // The label's face comes from `TextRole::Label` now, not from the threaded
    // id; the parameter stays so the ~20 call sites did not all have to change.
    let _ = font_id;
    let btn = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_height(height)
            .with_margin(Thickness::axes(2.0, 1.0))
            .with_tooltip(tooltip)
            .with_background(theme::BG_RAISED),
    )
    .build();
    let h = ui.add_node(btn, parent);
    let row = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Horizontal)
        .build();
    let row_h = ui.add_node(row, h);
    // Centre both the glyph and the word on the button's axis rather than
    // computing a top margin from an assumed line height. The assumption was
    // wrong for the Zeta type roles — Inter's line box is 1.21 em, not the
    // 14 px this used to guess — which is why chrome labels sat a pixel or two
    // high.
    let img = ImageBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness {
                left: 8.0,
                top: 0.0,
                right: 5.0,
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
                right: 9.0,
                bottom: 0.0,
            }),
    )
    .with_role(TextRole::Label)
    .with_text(label)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    let lbl_h = ui.add_node(lbl, row_h);
    (h, lbl_h)
}

/// Build a centred empty state into `parent`.
///
/// Phase 27-G. Returns the container so the caller can hide it when the panel
/// gains content, rather than rebuilding the subtree on every refresh.
pub(crate) fn build_empty_state(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
    state: crate::metaphor::EmptyState,
) -> NodeHandle {
    let t = theme::active();
    let column = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::TRANSPARENT)
            .with_margin(Thickness {
                left: 16.0,
                top: 28.0,
                right: 16.0,
                bottom: 16.0,
            }),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let column_h = ui.add_node(column, parent);

    // The mark is muted, not accented: an empty panel is a neutral condition,
    // and an accent here would read as a warning.
    let icon = ImageBuilder::new(
        WidgetBuilder::new()
            .with_width(32.0)
            .with_height(32.0)
            .with_horizontal_alignment(HorizontalAlignment::Center),
    )
    .with_icon(state.icon)
    .with_size(32.0)
    .with_tint(t.semantic.text.disabled.bytes())
    .build();
    ui.add_node(icon, column_h);

    let headline = TextBuilder::new(
        WidgetBuilder::new()
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_margin(Thickness {
                left: 0.0,
                top: 10.0,
                right: 0.0,
                bottom: 0.0,
            }),
    )
    .with_text(state.headline)
    .with_font_size(t.typography.body)
    .with_font_id(font_id)
    .with_color(t.semantic.text.secondary.bytes())
    .build();
    ui.add_node(headline, column_h);

    let body = TextBuilder::new(
        WidgetBuilder::new()
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_margin(Thickness {
                left: 8.0,
                top: 6.0,
                right: 8.0,
                bottom: 0.0,
            }),
    )
    .with_text(state.body)
    .with_font_size(t.typography.caption)
    .with_font_id(font_id)
    .with_color(t.semantic.text.muted.bytes())
    .with_wrap(true)
    .build();
    ui.add_node(body, column_h);

    let action = TextBuilder::new(
        WidgetBuilder::new()
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_margin(Thickness {
                left: 8.0,
                top: 10.0,
                right: 8.0,
                bottom: 0.0,
            }),
    )
    .with_text(state.action)
    .with_font_size(t.typography.caption)
    .with_font_id(font_id)
    .with_color(t.semantic.text.link.bytes())
    .with_wrap(true)
    .build();
    ui.add_node(action, column_h);

    column_h
}

/// One extra menu row that is not a registry command.
///
/// The recent-scenes tail is the only caller, and it is deliberately narrow:
/// everything else in a menu comes from CONTROL-A2's registry, and a general
/// "add an arbitrary row" helper would be an invitation to bypass it.
pub(crate) fn menu_entry(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
    label: &str,
    tooltip: &str,
    enabled: bool,
) -> NodeHandle {
    let button = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_height(22.0)
            .with_enabled(enabled)
            .with_tooltip(tooltip)
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let button = ui.add_node(button, parent);
    let text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
        .with_text(label)
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_color(if enabled {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_DISABLED
        })
        .build();
    ui.add_node(text, button);
    button
}
