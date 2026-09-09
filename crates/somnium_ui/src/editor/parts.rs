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
        scroll_viewer::ScrollViewerBuilder,
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
    let picker = picker_popup(ui, combo, 280.0);
    let list = ComboDropdownBuilder::new(WidgetBuilder::new())
        .with_items(items.iter().copied())
        .with_combo(combo)
        .with_popup(picker.popup)
        .with_font_id(font_id)
        .build();
    let list_h = ui.add_node(list, picker.list_host);
    ui.send(ComboBoxMessage::bind_popup(combo, picker.popup, list_h));
    picker.popup
}

/// Shared nonmodal picker: fixed header/footer and a compact, scrolling body.
/// Callers supply the choices and optional search/actions, not overlay paint.
pub(crate) struct PickerPopup {
    pub popup: NodeHandle,
    pub header: NodeHandle,
    pub list_host: NodeHandle,
    pub footer: NodeHandle,
}

pub(crate) fn picker_popup(ui: &mut UserInterface, anchor: NodeHandle, width: f32) -> PickerPopup {
    use crate::widgets::border::Surface;
    let popup = ui.add_node(
        PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_anchor(anchor)
            .with_content_width(width)
            .with_placement(PopupPlacement::AnchorBelow)
            .build(),
        ui.root(),
    );
    let frame = ui.add_node(
        BorderBuilder::new(
            WidgetBuilder::new()
                .with_max_size(glam::Vec2::new(
                    width,
                    theme::active().density.row_dense * 12.0 + 2.0,
                ))
                .with_horizontal_alignment(HorizontalAlignment::Left)
                .with_vertical_alignment(VerticalAlignment::Top),
        )
        .with_surface(Surface::Popup)
        .with_stroke_thickness(Thickness::uniform(1.0))
        .build(),
        popup,
    );
    let grid = ui.add_node(
        crate::node::UiNode::new(
            WidgetBuilder::new()
                .with_background(theme::TRANSPARENT)
                .build(),
            Box::new(PickerLayout),
        ),
        frame,
    );
    let header = ui.add_node(
        StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_row(0)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Vertical)
        .build(),
        grid,
    );
    let list_host = ui.add_node(
        ScrollViewerBuilder::new(
            WidgetBuilder::new()
                .with_row(1)
                .with_background(theme::TRANSPARENT),
        )
        .with_shrink_to_content(true)
        .build(),
        grid,
    );
    let footer = ui.add_node(
        StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_row(2)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Horizontal)
        .build(),
        grid,
    );
    PickerPopup {
        popup,
        header,
        list_host,
        footer,
    }
}

/// Unlike a stretch Grid, this layout shrinks with filtered results while
/// reserving the search and actions before it gives remaining height to the list.
struct PickerLayout;

impl crate::node::Control for PickerLayout {
    fn measure_override(
        &self,
        widget: &crate::widget::Widget,
        ctx: &mut crate::node::LayoutCtx,
        available: glam::Vec2,
    ) -> glam::Vec2 {
        let mut fixed_height = 0.0;
        for index in [0, 2] {
            if let Some(&child) = widget.children.get(index) {
                ctx.measure_child(child, glam::Vec2::new(available.x, f32::INFINITY));
                fixed_height += ctx.desired_size(child).y;
            }
        }
        let mut body_height = 0.0;
        if let Some(&body) = widget.children.get(1) {
            ctx.measure_child(
                body,
                glam::Vec2::new(available.x, (available.y - fixed_height).max(0.0)),
            );
            body_height = ctx.desired_size(body).y;
        }
        glam::Vec2::new(available.x, fixed_height + body_height)
    }

    fn arrange_override(
        &self,
        widget: &crate::widget::Widget,
        ctx: &mut crate::node::LayoutCtx,
        size: glam::Vec2,
    ) -> glam::Vec2 {
        let header_h = widget
            .children
            .first()
            .map(|&h| ctx.desired_size(h).y)
            .unwrap_or(0.0);
        let footer_h = widget
            .children
            .get(2)
            .map(|&h| ctx.desired_size(h).y)
            .unwrap_or(0.0);
        let heights = [header_h, (size.y - header_h - footer_h).max(0.0), footer_h];
        let mut y = widget.actual_local_position.y;
        for (&child, height) in widget.children.iter().zip(heights) {
            ctx.arrange_child(
                child,
                crate::types::Rect::new(widget.actual_local_position.x, y, size.x, height),
            );
            y += height;
        }
        size
    }

    fn handle_routed_message(
        &mut self,
        _: &mut crate::widget::Widget,
        _: &mut crate::message::UiMessage,
        _: &mut Vec<crate::message::UiMessage>,
    ) {
    }
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
            .with_width(280.0)
            .with_max_size(glam::Vec2::new(
                280.0,
                theme::active().density.row_tree * 16.0,
            ))
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_background(theme::active().semantic.surface.header.bytes())
            .with_foreground(theme::active().semantic.border.subtle.bytes()),
    )
    .with_surface(crate::widgets::border::Surface::Popup)
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let border_h = ui.add_node(border, popup_h);
    let scroll_h = ui.add_node(
        ScrollViewerBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_shrink_to_content(true)
            .build(),
        border_h,
    );
    let stack = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Vertical)
        .build();
    let stack_h = ui.add_node(stack, scroll_h);
    let mut handles = Vec::with_capacity(commands.len());
    for command in commands {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(theme::active().density.row_tree)
                .with_background(theme::TRANSPARENT),
        )
        .build();
        let bh = ui.add_node(btn, stack_h);
        let lbl = TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::axes(8.0, 0.0))
                .with_vertical_alignment(VerticalAlignment::Center),
        )
        .with_text(command.menu_label())
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_color(theme::active().semantic.text.primary.bytes())
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
            .with_height(theme::active().density.icon_action)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(theme::NOCTURNE.geometry.inset_panel, 0.0))
            .with_hit_test_visibility(false)
            .with_background(theme::active().semantic.border.default.bytes())
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
        .with_height(theme::active().density.row_chrome)
        .with_vertical_alignment(VerticalAlignment::Center)
        .with_margin(Thickness::axes(2.0, 1.0))
        .with_background(theme::active().semantic.surface.raised.bytes());
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
    .with_tint(theme::active().semantic.text.primary.bytes())
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
            .with_height(theme::active().density.titlebar)
            .with_tooltip(tooltip)
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let h = ui.add_node(btn, parent);
    let img = ImageBuilder::new(WidgetBuilder::new())
        .with_icon(icon)
        .with_size(16.0)
        .with_tint(theme::active().semantic.text.primary.bytes())
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
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_margin(Thickness::axes(2.0, 1.0))
            .with_tooltip(tooltip)
            .with_background(theme::active().semantic.surface.raised.bytes()),
    )
    .build();
    let h = ui.add_node(btn, parent);
    let row = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_background(theme::TRANSPARENT),
    )
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
    .with_tint(theme::active().semantic.text.primary.bytes())
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
    .with_color(theme::active().semantic.text.primary.bytes())
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
            .with_height(theme::active().density.row_tree)
            .with_enabled(enabled)
            .with_tooltip(tooltip)
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let button = ui.add_node(button, parent);
    let text = TextBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::axes(8.0, 0.0))
            .with_vertical_alignment(VerticalAlignment::Center),
    )
    .with_text(label)
    .with_font_size(12.0)
    .with_font_id(font_id)
    .with_color(if enabled {
        theme::active().semantic.text.primary.bytes()
    } else {
        theme::active().semantic.text.disabled.bytes()
    })
    .build();
    ui.add_node(text, button);
    button
}

#[cfg(test)]
mod menu_layout_tests {
    use super::*;
    use crate::{
        commands::Menu,
        message::{MessageDirection, Modifiers, UiMessage, WidgetMessage},
        widgets::popup::PopupMessage,
    };

    #[test]
    fn long_menus_fit_small_windows_and_the_last_command_is_reachable() {
        for height in [360.0, 720.0] {
            for menu in [Menu::Create, Menu::View] {
                let mut ui = UserInterface::new(800.0, height);
                let root = ui.root();
                let bar = ui.add_node(
                    StackPanelBuilder::new(
                        WidgetBuilder::new()
                            .with_height(28.0)
                            .with_vertical_alignment(VerticalAlignment::Top),
                    )
                    .with_orientation(Orientation::Horizontal)
                    .build(),
                    root,
                );
                let anchor = menu_button(&mut ui, bar, "Menu", 0);
                let (popup, stack, rows) = if menu == Menu::Create {
                    let (popup, rows) =
                        crate::editor::content::build_create_popup(&mut ui, root, 0);
                    let stack = ui.parent_of(rows[0].0).unwrap();
                    (popup, stack, rows)
                } else {
                    command_popup_items(&mut ui, root, 0, menu)
                };
                ui.perform_layout();
                ui.send(UiMessage::new(
                    popup,
                    MessageDirection::ToWidget,
                    PopupMessage::SetAnchor(anchor),
                ));
                ui.send(UiMessage::new(
                    popup,
                    MessageDirection::ToWidget,
                    PopupMessage::Open,
                ));
                ui.update();
                ui.perform_layout();
                let panel = ui.screen_bounds(ui.first_child(popup));
                assert!(
                    panel.y >= 0.0 && panel.y + panel.h <= height,
                    "menu extends beyond window: {panel:?}, height {height}"
                );
                assert!(panel.h <= theme::active().density.row_tree * 16.0 + 0.1);
                let last = rows.last().unwrap().0;
                ui.send(UiMessage::new(
                    stack,
                    MessageDirection::ToWidget,
                    WidgetMessage::MouseWheel {
                        pos: glam::Vec2::new(panel.x + 10.0, panel.y + 10.0),
                        delta: -10000.0,
                        mods: Modifiers::default(),
                    },
                ));
                ui.update();
                ui.perform_layout();
                let last_bounds = ui.screen_bounds(last);
                assert!(
                    last_bounds.y >= panel.y && last_bounds.y + last_bounds.h <= panel.y + panel.h,
                    "last menu command must scroll into view: {last_bounds:?} in {panel:?}"
                );
                ui.send(UiMessage::new(
                    stack,
                    MessageDirection::ToWidget,
                    WidgetMessage::MouseWheel {
                        pos: glam::Vec2::ZERO,
                        delta: 10000.0,
                        mods: Modifiers::default(),
                    },
                ));
                ui.update();
                ui.perform_layout();
                ui.set_focus(last);
                ui.perform_layout();
                let focused = ui.screen_bounds(last);
                assert!(
                    focused.y >= panel.y && focused.y + focused.h <= panel.y + panel.h,
                    "keyboard focus must reveal the command"
                );
            }
        }
    }

    #[test]
    fn short_menu_does_not_reserve_sixteen_empty_rows() {
        let mut ui = UserInterface::new(800.0, 720.0);
        let root = ui.root();
        let (popup, _, rows) = command_popup_items(&mut ui, root, 0, Menu::Help);
        ui.send(UiMessage::new(
            popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        ui.update();
        ui.perform_layout();
        let bounds = ui.screen_bounds(ui.first_child(popup));
        assert!(bounds.h <= rows.len() as f32 * theme::active().density.row_tree + 4.0);
    }
}

#[cfg(test)]
mod picker_regressions {
    use super::*;
    use crate::{
        message::{MessageDirection, UiMessage},
        widgets::combo_box::ComboBoxBuilder,
    };

    #[test]
    fn long_picker_is_compact_scrolls_and_selects_at_window_edges() {
        use crate::message::{Modifiers, MouseButton, WidgetMessage};
        for (width, height) in [(1280.0, 720.0), (480.0, 320.0), (240.0, 240.0)] {
            let mut ui = UserInterface::new(width, height);
            let items = (0..200).map(|i| format!("Layer {i}")).collect::<Vec<_>>();
            let labels = items.iter().map(String::as_str).collect::<Vec<_>>();
            let combo = ui.add_node(
                ComboBoxBuilder::new(
                    WidgetBuilder::new()
                        .with_width(120.0)
                        .with_height(24.0)
                        .with_horizontal_alignment(HorizontalAlignment::Left)
                        .with_vertical_alignment(VerticalAlignment::Top)
                        .with_desired_position(glam::Vec2::new(width - 130.0, height - 48.0)),
                )
                .with_items(labels.iter().copied())
                .build(),
                ui.root(),
            );
            let popup = attach_combo_popup(&mut ui, combo, &labels, 0);
            ui.update();
            ui.perform_layout();
            ui.send(UiMessage::new(
                combo,
                MessageDirection::ToWidget,
                WidgetMessage::MouseDown {
                    pos: glam::Vec2::ZERO,
                    button: MouseButton::Left,
                    mods: Modifiers::default(),
                },
            ));
            ui.update();
            ui.perform_layout();
            let panel = ui.first_child(popup);
            let b = ui.screen_bounds(panel);
            assert!(
                b.h <= 320.0 && b.w <= 280.0,
                "picker must be compact: {b:?}"
            );
            assert!(
                b.x >= 0.0 && b.y >= 0.0 && b.x + b.w <= width && b.y + b.h <= height,
                "picker must fit its window: {b:?}"
            );
            let grid = ui.first_child(panel);
            let viewer = ui.nodes.borrow(grid.transmute()).widget.children[1];
            let list = ui.first_child(viewer);
            let view = ui.screen_bounds(viewer);
            // The gutter is a hit-testable control, not obscured by list content.
            let thumb = glam::Vec2::new(view.x + view.w - 5.0, view.y + 3.0);
            assert_eq!(ui.hit_test(thumb), viewer);
            ui.send(UiMessage::new(
                viewer,
                MessageDirection::ToWidget,
                WidgetMessage::MouseDown {
                    pos: thumb,
                    button: MouseButton::Left,
                    mods: Modifiers::default(),
                },
            ));
            ui.send(UiMessage::new(
                viewer,
                MessageDirection::ToWidget,
                WidgetMessage::MouseMove {
                    pos: glam::Vec2::new(thumb.x, view.y + view.h),
                    mods: Modifiers::default(),
                },
            ));
            ui.send(UiMessage::new(
                viewer,
                MessageDirection::ToWidget,
                WidgetMessage::MouseUp {
                    pos: thumb,
                    button: MouseButton::Left,
                    mods: Modifiers::default(),
                },
            ));
            ui.update();
            ui.perform_layout();
            // Editor model snapshots repeat SetSelected every frame. They must
            // not fight a user's drag when the selected value has not changed.
            let scrolled = ui.screen_bounds(list);
            ui.send(UiMessage::new(
                combo,
                MessageDirection::ToWidget,
                ComboBoxMessage::SetSelected(0),
            ));
            ui.update();
            ui.perform_layout();
            assert_eq!(ui.screen_bounds(list), scrolled);
            let list_bounds = ui.screen_bounds(list);
            assert!(
                (list_bounds.y + list_bounds.h - view.y - view.h).abs() < 1.0,
                "drag must reach last row"
            );
            ui.send(UiMessage::new(
                list,
                MessageDirection::ToWidget,
                WidgetMessage::MouseWheel {
                    pos: thumb,
                    delta: 100000.0,
                    mods: Modifiers::default(),
                },
            ));
            ui.update();
            ui.perform_layout();
            assert!(
                (ui.screen_bounds(list).y - view.y).abs() < 1.0,
                "wheel must reach first row"
            );
            ui.send(UiMessage::new(
                combo,
                MessageDirection::ToWidget,
                ComboBoxMessage::SetSelected(199),
            ));
            ui.update();
            ui.perform_layout();
            let last_y = ui.screen_bounds(list).y + 199.0 * theme::active().density.row_dense;
            let point = glam::Vec2::new(view.x + 20.0, last_y + 5.0);
            assert_eq!(
                ui.hit_test(point),
                list,
                "selected row must be revealed on reopening"
            );
            ui.send(UiMessage::new(
                list,
                MessageDirection::ToWidget,
                WidgetMessage::MouseDown {
                    pos: point,
                    button: MouseButton::Left,
                    mods: Modifiers::default(),
                },
            ));
            let events = ui.update();
            assert!(events.iter().any(|m| matches!(
                m.data::<ComboBoxMessage>(),
                Some(ComboBoxMessage::SelectionChanged(199))
            )));
            assert!(!ui.visibility(popup));
            // Root-parented popups must disappear when their owner is rebuilt.
            ui.remove_node(combo);
            assert!(ui.nodes.try_borrow(popup.transmute()).is_err());
        }
    }
}
