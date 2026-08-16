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
        check_box::CheckBoxBuilder,
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

pub(crate) fn make_palette_button(
    ui: &mut UserInterface,
    text: &str,
    font_id: u8,
    parent: NodeHandle,
) -> (NodeHandle, NodeHandle) {
    let btn = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_height(20.0)
            .with_width(52.0)
            .with_margin(Thickness {
                left: 2.0,
                top: 1.0,
                right: 2.0,
                bottom: 1.0,
            })
            .with_background(theme::BG_DARK),
    )
    .build();
    let btn_h = ui.add_node(btn, parent);
    let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 3.0,
        top: 3.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text(&format!(" {text}"))
    .with_font_size(10.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    let lbl_h = ui.add_node(lbl, btn_h);
    (btn_h, lbl_h)
}

/// Build a checkbox toggle row (Phase 26-B). Both handles are the CheckBox
/// so existing inspector fields that stored a separate label still compile.
pub(crate) fn make_toggle(
    ui: &mut UserInterface,
    text: &str,
    font_id: u8,
    parent: NodeHandle,
) -> (NodeHandle, NodeHandle) {
    make_toggle_checked(ui, text, font_id, parent, false)
}

pub(crate) fn make_toggle_checked(
    ui: &mut UserInterface,
    text: &str,
    font_id: u8,
    parent: NodeHandle,
    checked: bool,
) -> (NodeHandle, NodeHandle) {
    let cb = CheckBoxBuilder::new(
        WidgetBuilder::new()
            .with_height(22.0)
            .with_margin(Thickness {
                left: 6.0,
                top: 2.0,
                right: 6.0,
                bottom: 0.0,
            })
            .with_background(theme::TRANSPARENT),
    )
    .with_checked(checked)
    .with_label(text)
    .with_font_id(font_id)
    .with_font_size(11.0)
    .build();
    let h = ui.add_node(cb, parent);
    (h, h)
}

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

pub(crate) fn popup_items(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
    items: &[&str],
) -> (NodeHandle, Vec<NodeHandle>) {
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
    let mut handles = Vec::with_capacity(items.len());
    for item in items {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(22.0)
                .with_background(theme::TRANSPARENT),
        )
        .build();
        let bh = ui.add_node(btn, stack_h);
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
            .with_text(*item)
            .with_font_size(12.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_PRIMARY)
            .build();
        ui.add_node(lbl, bh);
        handles.push(bh);
    }
    (popup_h, handles)
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
