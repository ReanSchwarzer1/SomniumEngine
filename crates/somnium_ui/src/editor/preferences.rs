//! The Preferences window — CONTROL-H.
//!
//! Two things are worth saying about the shape of this file, because both are
//! deliberate and both are why it is short.
//!
//! **Settings are properties.** The rows in the Settings tab are built by the
//! same [`build_generated_details`](super::inspector::build_generated_details)
//! that builds the entity inspector, from the same
//! [`GeneratedComponentPanel`](super::inspector_gen::GeneratedComponentPanel)
//! model, because a preference is a property of a non-entity object. Grouping,
//! units, ranges, precision, the modified dot and per-setting revert therefore
//! all work without a line of code here.
//!
//! **The search index is generated, not maintained.** Unity's
//! `GetSearchKeywordsFrom*` exists because a hand-written keyword list is a
//! list that goes stale. The filter runs over the declared rows — label, field
//! name and doc comment — so a setting is searchable the moment it is
//! declared.

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
        grid::{Column, GridBuilder, Row},
        popup::{PopupBuilder, PopupPlacement},
        scroll_viewer::ScrollViewerBuilder,
        search_box::SearchBoxBuilder,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
    },
};

use super::parts::window_chrome_button;

/// Handles the shell keeps for the Preferences window.
pub(crate) struct PreferencesHandles {
    /// The modal popup.
    pub overlay: NodeHandle,
    /// Where generated settings rows are mounted and re-mounted.
    pub settings_body: NodeHandle,
    /// Where keybinding rows are mounted and re-mounted.
    pub bindings_body: NodeHandle,
    /// Filter box, shared by both tabs.
    pub search: NodeHandle,
    /// "Modified only".
    pub modified_only: NodeHandle,
    /// Tab buttons.
    pub tab_settings: NodeHandle,
    /// Tab buttons.
    pub tab_bindings: NodeHandle,
    /// Reset every override.
    pub reset_all: NodeHandle,
    /// Close.
    pub close: NodeHandle,
}

/// Build the window. It is a modal card, like Help, because preferences are a
/// place you go rather than a panel you keep open beside the work.
pub(crate) fn build_preferences_window(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
) -> PreferencesHandles {
    let overlay = PopupBuilder::new(WidgetBuilder::new().with_background([0x0E, 0x10, 0x14, 0xE0]))
        .with_placement(PopupPlacement::Center)
        .build();
    let overlay = ui.add_node(overlay, root);

    let card = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(720.0)
            .with_height(520.0)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_background(theme::active().semantic.surface.panel.bytes())
            .with_foreground(theme::active().semantic.border.subtle.bytes()),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let card = ui.add_node(card, overlay);

    let grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(36.0))
        .add_row(Row::strict(34.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let grid = ui.add_node(grid, card);

    // ── header ──────────────────────────────────────────────────────────────
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
    let header = ui.add_node(header, grid);
    let header_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let header_grid = ui.add_node(header_grid, header);
    let title = TextBuilder::new(WidgetBuilder::new().with_column(0).with_margin(Thickness {
        left: 12.0,
        top: 10.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_role(TextRole::Title)
    .with_text("Preferences")
    .build();
    ui.add_node(title, header_grid);
    let close_col = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_column(1)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let close_col = ui.add_node(close_col, header_grid);
    let close = window_chrome_button(ui, close_col, IconId::Close, "Close Preferences");

    // ── toolbar: tabs, search, filters ──────────────────────────────────────
    let bar = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::active().semantic.surface.header.bytes()),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let bar = ui.add_node(bar, grid);

    let tab_settings = labelled_button(ui, bar, font_id, "Settings");
    let tab_bindings = labelled_button(ui, bar, font_id, "Keyboard");
    let search = SearchBoxBuilder::new(
        WidgetBuilder::new()
            .with_width(240.0)
            .with_margin(Thickness::axes(8.0, 5.0)),
    )
    .with_font_id(font_id)
    .build();
    let search = ui.add_node(search, bar);
    let modified_only = ui.add_node(
        CheckBoxBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 9.0)))
            .with_label("Modified only")
            .with_font_id(font_id)
            .build(),
        bar,
    );
    let reset_all = labelled_button(ui, bar, font_id, "Reset All");

    // ── body ────────────────────────────────────────────────────────────────
    let scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::active().semantic.surface.panel.bytes()),
    )
    .build();
    let scroll = ui.add_node(scroll, grid);
    let body = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::uniform(10.0))
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let body = ui.add_node(body, scroll);

    let settings_body = ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build(),
        body,
    );
    let bindings_body = ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build(),
        body,
    );
    ui.set_visibility(bindings_body, false);

    PreferencesHandles {
        overlay,
        settings_body,
        bindings_body,
        search,
        modified_only,
        tab_settings,
        tab_bindings,
        reset_all,
        close,
    }
}

fn labelled_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
    label: &str,
) -> NodeHandle {
    let button = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_height(24.0)
            .with_margin(Thickness::axes(6.0, 5.0))
            .with_background(theme::active().semantic.surface.raised.bytes()),
    )
    .build();
    let button = ui.add_node(button, parent);
    let text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(10.0, 4.0)))
        .with_text(label)
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_color(theme::active().semantic.text.primary.bytes())
        .build();
    ui.add_node(text, button);
    button
}

/// One keybinding row: the command, its chord, and the two buttons.
///
/// Built here rather than by the generated property path because a chord is
/// not a `ReflectValue` — it is captured from a keystroke, not typed — and
/// pretending otherwise would mean inventing a `FieldType` that only one thing
/// uses.
pub(crate) fn build_binding_row(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
    label: &str,
    chord: &str,
    conflicted: bool,
    customised: bool,
) -> (NodeHandle, NodeHandle) {
    let row = GridBuilder::new(
        WidgetBuilder::new()
            .with_height(26.0)
            .with_background(theme::TRANSPARENT),
    )
    .add_row(Row::stretch())
    .add_column(Column::stretch())
    .add_column(Column::strict(140.0))
    .add_column(Column::strict(64.0))
    .build();
    let row = ui.add_node(row, parent);

    let name = TextBuilder::new(
        WidgetBuilder::new()
            .with_column(0)
            .with_margin(Thickness::axes(8.0, 6.0)),
    )
    .with_text(label)
    .with_font_size(12.0)
    .with_font_id(font_id)
    .with_color(if conflicted {
        theme::active().semantic.status.error.bytes()
    } else {
        theme::active().semantic.text.primary.bytes()
    })
    .build();
    ui.add_node(name, row);

    // The chord itself is the click target: clicking it starts capture, which
    // is the interaction every editor uses and the one that needs no label.
    let capture = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_column(1)
            .with_margin(Thickness::axes(2.0, 3.0))
            .with_background(theme::active().semantic.surface.raised.bytes()),
    )
    .build();
    let capture = ui.add_node(capture, row);
    let chord_text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
        .with_text(chord)
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_color(if customised {
            theme::active().semantic.accent.default.bytes()
        } else {
            theme::active().semantic.text.secondary.bytes()
        })
        .build();
    ui.add_node(chord_text, capture);

    let reset = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_column(2)
            .with_margin(Thickness::axes(2.0, 3.0))
            .with_background(theme::active().semantic.surface.raised.bytes())
            .with_enabled(customised),
    )
    .build();
    let reset = ui.add_node(reset, row);
    let reset_text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
        .with_text("Reset")
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_color(theme::active().semantic.text.secondary.bytes())
        .build();
    ui.add_node(reset_text, reset);

    (capture, reset)
}
