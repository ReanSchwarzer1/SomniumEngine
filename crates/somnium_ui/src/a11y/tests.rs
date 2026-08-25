//! MORROWIND-I tests.
//!
//! Every one of these is written against a *built widget tree* rather than
//! against a hand-made `A11yNode`, because the interesting failures are all in
//! the walk: the collapse rule, the hidden-subtree rule, and where a name comes
//! from when the control that has the role does not own it.

use super::*;
use crate::types::{HorizontalAlignment, VerticalAlignment};
use crate::ui::UserInterface;
use crate::widget::WidgetBuilder;
use crate::widgets::{
    border::BorderBuilder, button::ButtonBuilder, check_box::CheckBoxBuilder,
    slider::SliderBuilder, stack_panel::StackPanelBuilder, text::TextBuilder,
};

fn ui() -> UserInterface {
    UserInterface::new(800.0, 600.0)
}

#[test]
fn an_empty_tree_still_has_a_root() {
    let tree = A11yTree::from_ui(&ui());
    assert!(
        tree.get(tree.root).is_some(),
        "no root to attach a cursor to"
    );
    assert_eq!(tree.get(tree.root).unwrap().role, Role::Window);
    assert_eq!(tree.focus, tree.root);
}

#[test]
fn static_text_is_a_label_and_is_its_own_name() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(
        TextBuilder::new(WidgetBuilder::new())
            .with_text("Seyda Neen")
            .build(),
        root,
    );
    ui.perform_layout();

    let tree = A11yTree::from_ui(&ui);
    let label = tree
        .nodes
        .iter()
        .find(|n| n.role == Role::Label)
        .expect("no label in the tree");
    assert_eq!(label.name, "Seyda Neen");
    assert_eq!(label.id, handle.index() as u64 + 1);
}

/// The collapse rule, which is what makes the tree navigable rather than merely
/// correct. A button is a border wrapping a panel wrapping a text node.
///
/// Built parent-first, because `add_node` parents into the handle it is given:
/// building children first and wrapping them afterwards leaves every node a
/// child of the root *as well*, which is a tree that would pass this test by
/// accident. That mistake is in this file's history and the comment is here so
/// it is not repeated.
#[test]
fn scaffolding_between_a_button_and_its_label_is_collapsed_away() {
    let mut ui = ui();
    let root = ui.root();

    let button = ui.add_node(ButtonBuilder::new(WidgetBuilder::new()).build(), root);
    let border = ui.add_node(BorderBuilder::new(WidgetBuilder::new()).build(), button);
    let panel = ui.add_node(StackPanelBuilder::new(WidgetBuilder::new()).build(), border);
    let _label = ui.add_node(
        TextBuilder::new(WidgetBuilder::new())
            .with_text("Save")
            .build(),
        panel,
    );
    ui.perform_layout();

    let tree = A11yTree::from_ui(&ui);
    let roles: Vec<Role> = tree.nodes.iter().map(|n| n.role).collect();
    assert!(
        roles.contains(&Role::Button),
        "the button vanished: {roles:?}"
    );
    assert!(
        !roles.contains(&Role::Group),
        "presentational scaffolding survived into the tree: {roles:?}"
    );
    assert_eq!(
        tree.nodes.len(),
        3,
        "expected window + button + label, got {roles:?}"
    );

    let button_node = tree
        .nodes
        .iter()
        .find(|n| n.role == Role::Button)
        .expect("button");
    assert_eq!(button_node.id, button.index() as u64 + 1);
    // The label is still reachable *under* the button — collapsing removes the
    // scaffolding, not the content.
    assert_eq!(button_node.children.len(), 1);
    let child = tree.get(button_node.children[0]).expect("the label");
    assert_eq!(child.name, "Save");
}

#[test]
fn a_hidden_subtree_does_not_exist_to_a_reader() {
    let mut ui = ui();
    let root = ui.root();
    let menu = ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_visibility(false)).build(),
        root,
    );
    ui.add_node(
        TextBuilder::new(WidgetBuilder::new())
            .with_text("Closed menu item")
            .build(),
        menu,
    );
    // A visible sibling, so the assertion below is testing the *hidden* branch
    // rather than an empty tree.
    ui.add_node(
        TextBuilder::new(WidgetBuilder::new())
            .with_text("Open item")
            .build(),
        root,
    );
    ui.perform_layout();

    let tree = A11yTree::from_ui(&ui);
    assert!(
        tree.nodes.iter().any(|n| n.name == "Open item"),
        "the visible sibling is missing, so this test proves nothing"
    );
    assert!(
        !tree.nodes.iter().any(|n| n.name.contains("Closed")),
        "a hidden widget was announced; a reader would read out a closed menu"
    );
}

#[test]
fn a_check_box_reports_all_three_states() {
    for (checked, mixed, want) in [
        (false, false, Toggled::False),
        (true, false, Toggled::True),
        (true, true, Toggled::Mixed),
        (false, true, Toggled::Mixed),
    ] {
        let mut ui = ui();
        let root = ui.root();
        let node = CheckBoxBuilder::new(WidgetBuilder::new())
            .with_label("Cast shadows")
            .with_checked(checked)
            .with_mixed(mixed)
            .build();
        ui.add_node(node, root);
        ui.perform_layout();

        let tree = A11yTree::from_ui(&ui);
        let box_node = tree
            .nodes
            .iter()
            .find(|n| n.role == Role::CheckBox)
            .expect("check box");
        assert_eq!(
            box_node.toggled,
            Some(want),
            "checked={checked} mixed={mixed}"
        );
        assert_eq!(box_node.name, "Cast shadows");
    }
}

#[test]
fn a_slider_speaks_a_readable_number_rather_than_an_f32() {
    let mut ui = ui();
    let root = ui.root();
    ui.add_node(
        SliderBuilder::new(WidgetBuilder::new())
            .with_value(0.4399999)
            .build(),
        root,
    );
    ui.perform_layout();

    let tree = A11yTree::from_ui(&ui);
    let slider = tree
        .nodes
        .iter()
        .find(|n| n.role == Role::Slider)
        .expect("slider");
    assert_eq!(slider.value.as_deref(), Some("0.44"));
}

/// The tooltip is the accessible name of an icon-only control. The shell
/// authors these already, for the same reason and without knowing it.
#[test]
fn an_icon_only_button_borrows_its_tooltip_as_a_name() {
    let mut ui = ui();
    let root = ui.root();
    ui.add_node(
        ButtonBuilder::new(WidgetBuilder::new().with_tooltip("Frame selection")).build(),
        root,
    );
    ui.perform_layout();

    let tree = A11yTree::from_ui(&ui);
    let button = tree
        .nodes
        .iter()
        .find(|n| n.role == Role::Button)
        .expect("button");
    assert_eq!(button.name, "Frame selection");
    assert!(tree.unnamed().is_empty());
}

/// The most useful diagnostic in the module, asserted as a diagnostic.
#[test]
fn an_unnamed_focusable_control_is_reported_rather_than_ignored() {
    let mut ui = ui();
    let root = ui.root();
    ui.add_node(ButtonBuilder::new(WidgetBuilder::new()).build(), root);
    ui.perform_layout();

    let tree = A11yTree::from_ui(&ui);
    let unnamed = tree.unnamed();
    assert_eq!(unnamed.len(), 1, "an unnamed button was not reported");
    assert_eq!(unnamed[0].role, Role::Button);
    // ...and a label, which is not focusable, is not reported even unnamed.
    assert!(unnamed.iter().all(|n| n.role != Role::Label));
}

#[test]
fn focus_follows_the_widget_tree() {
    let mut ui = ui();
    let root = ui.root();
    let first = ui.add_node(
        ButtonBuilder::new(WidgetBuilder::new().with_tooltip("First")).build(),
        root,
    );
    let second = ui.add_node(
        ButtonBuilder::new(WidgetBuilder::new().with_tooltip("Second")).build(),
        root,
    );
    ui.perform_layout();

    ui.set_focus(second);
    let tree = A11yTree::from_ui(&ui);
    assert_eq!(tree.focus, second.index() as u64 + 1);
    assert!(tree.get(tree.focus).unwrap().focused);
    assert!(!tree.get(first.index() as u64 + 1).unwrap().focused);
}

// ── announcements ───────────────────────────────────────────────────────────

#[test]
fn a_focus_announcement_reads_name_then_role_then_state() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(
        CheckBoxBuilder::new(WidgetBuilder::new())
            .with_label("Cast shadows")
            .with_checked(true)
            .build(),
        root,
    );
    ui.perform_layout();
    ui.set_focus(handle);

    let tree = A11yTree::from_ui(&ui);
    let said = tree.announce_focus(tree.focus).expect("something to say");
    assert_eq!(said, "Cast shadows, check box, checked");
}

#[test]
fn a_disabled_control_says_so_last() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(
        ButtonBuilder::new(
            WidgetBuilder::new()
                .with_tooltip("Save")
                .with_enabled(false),
        )
        .build(),
        root,
    );
    ui.perform_layout();
    ui.set_focus(handle);

    let tree = A11yTree::from_ui(&ui);
    assert_eq!(
        tree.announce_focus(tree.focus).as_deref(),
        Some("Save, button, dimmed")
    );
}

#[test]
fn an_unnamed_control_still_announces_its_role() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(ButtonBuilder::new(WidgetBuilder::new()).build(), root);
    ui.perform_layout();
    ui.set_focus(handle);

    let tree = A11yTree::from_ui(&ui);
    // Bad, but not silent: "button" alone is a usability failure and silence is
    // a correctness one.
    assert_eq!(tree.announce_focus(tree.focus).as_deref(), Some("button"));
}

#[test]
fn announcing_a_node_that_is_not_there_is_none_rather_than_a_panic() {
    let tree = A11yTree::from_ui(&ui());
    assert!(tree.announce_focus(9_999).is_none());
}

// ── roles ───────────────────────────────────────────────────────────────────

#[test]
fn only_group_is_meaningless_and_only_interactive_roles_are_focusable() {
    assert!(!Role::Group.is_meaningful());
    assert!(Role::Button.is_meaningful());
    assert!(Role::Label.is_meaningful());

    assert!(Role::Button.is_focusable());
    assert!(Role::Slider.is_focusable());
    assert!(Role::TextInput.is_focusable());
    // A label is meaningful and not focusable, which is the pair that matters:
    // `unnamed()` must not report every unnamed decoration.
    assert!(!Role::Label.is_focusable());
    assert!(!Role::Window.is_focusable());
    assert!(!Role::Alert.is_focusable());
}

#[test]
fn every_role_has_a_distinct_spoken_name() {
    let roles = [
        Role::Window,
        Role::Group,
        Role::Label,
        Role::Button,
        Role::CheckBox,
        Role::Slider,
        Role::TextInput,
        Role::ComboBox,
        Role::List,
        Role::ListItem,
        Role::TabList,
        Role::Tab,
        Role::ScrollView,
        Role::Menu,
        Role::MenuItem,
        Role::Image,
        Role::Dialog,
        Role::Alert,
    ];
    let mut names: Vec<&str> = roles.iter().map(|r| r.as_str()).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "two roles speak the same name");
}

// ── settings ────────────────────────────────────────────────────────────────

#[test]
fn reduced_motion_reaches_the_animator() {
    let mut ui = ui();
    let key = crate::motion::MotionKey::new(1, crate::motion::MotionProperty::HoverWash);
    ui.set_a11y_settings(A11ySettings {
        reduced_motion: true,
        high_contrast: false,
    });
    ui.draw_ctx
        .motion
        .start(key, 0.0, 1.0, 120.0, crate::motion::Easing::Standard);
    assert!(
        ui.draw_ctx.motion.is_idle(),
        "reduced motion did not reach the animator"
    );
    assert_eq!(ui.draw_ctx.motion.value_or(key, -1.0), 1.0);
}

#[test]
fn turning_reduced_motion_off_again_lets_motion_run() {
    let mut ui = ui();
    ui.set_a11y_settings(A11ySettings {
        reduced_motion: true,
        ..A11ySettings::default()
    });
    ui.set_a11y_settings(A11ySettings::default());
    let key = crate::motion::MotionKey::new(1, crate::motion::MotionProperty::HoverWash);
    ui.draw_ctx
        .motion
        .start(key, 0.0, 1.0, 120.0, crate::motion::Easing::Standard);
    assert!(!ui.draw_ctx.motion.is_idle());
}

/// The invariant that keeps the two modes one product: neither setting may move
/// anything. A high-contrast build that relaid out would be a second interface
/// nobody tests.
#[test]
fn neither_setting_changes_layout() {
    fn bounds_with(settings: A11ySettings) -> Vec<crate::types::Rect> {
        let mut ui = ui();
        ui.set_a11y_settings(settings);
        let root = ui.root();
        let mut handles = Vec::new();
        for label in ["Raise", "Lower", "Smooth"] {
            handles.push(
                ui.add_node(
                    ButtonBuilder::new(
                        WidgetBuilder::new()
                            .with_width(120.0)
                            .with_height(28.0)
                            .with_horizontal_alignment(HorizontalAlignment::Left)
                            .with_vertical_alignment(VerticalAlignment::Top)
                            .with_tooltip(label),
                    )
                    .build(),
                    root,
                ),
            );
        }
        ui.perform_layout();
        handles.iter().map(|h| ui.screen_bounds(*h)).collect()
    }

    let plain = bounds_with(A11ySettings::default());
    for settings in [
        A11ySettings {
            reduced_motion: true,
            high_contrast: false,
        },
        A11ySettings {
            reduced_motion: false,
            high_contrast: true,
        },
        A11ySettings {
            reduced_motion: true,
            high_contrast: true,
        },
    ] {
        assert_eq!(
            bounds_with(settings),
            plain,
            "{settings:?} moved something; the two modes must be one interface"
        );
    }
}

#[test]
fn the_platform_query_is_honest_about_returning_defaults() {
    // Documented as returning defaults until a platform crate is added. The
    // test exists so that when the real query lands, this fails and somebody
    // updates the doc comment with it.
    assert_eq!(A11ySettings::from_platform(), A11ySettings::default());
}

// ── high contrast, against Zeta's certified pairs ───────────────────────────

#[test]
fn high_contrast_only_ever_raises_the_ratio() {
    use crate::theme::{Srgb8, contrast_ratio};

    let pairs = [
        (crate::theme::TEXT_PRIMARY, crate::theme::BG_PANEL),
        (crate::theme::TEXT_SECONDARY, crate::theme::BG_PANEL),
        (crate::theme::TEXT_PRIMARY, crate::theme::BG_RAISED),
    ];
    for (fg, bg) in pairs {
        let plain = contrast_ratio(Srgb8(fg), Srgb8(bg));
        let raised = contrast_ratio(Srgb8(super::high_contrast(fg, bg)), Srgb8(bg));
        assert!(
            raised >= plain - 0.01,
            "high contrast lowered a ratio: {plain} -> {raised}"
        );
    }
}

#[test]
fn high_contrast_reaches_the_wcag_aa_bar_for_body_text() {
    use crate::theme::{Srgb8, contrast_ratio};

    let bg = crate::theme::BG_PANEL;
    // Zeta certifies its pairs at the normal bar. The point of a high-contrast
    // mode is the users for whom that bar is not enough, so the assertion is
    // the *enhanced* one — 7:1, WCAG AAA for body text.
    for fg in [crate::theme::TEXT_PRIMARY, crate::theme::TEXT_SECONDARY] {
        let raised = contrast_ratio(Srgb8(super::high_contrast(fg, bg)), Srgb8(bg));
        assert!(raised >= 7.0, "high contrast gave only {raised}:1");
    }
}

#[test]
fn high_contrast_preserves_hue_direction() {
    // Text lighter than its background must stay lighter. Flipping one pair and
    // not another is how a high-contrast mode ends up unreadable in a way the
    // ratio does not catch.
    let bg = crate::theme::BG_PANEL;
    let fg = crate::theme::TEXT_PRIMARY;
    let raised = super::high_contrast(fg, bg);
    let lum = |c: [u8; 4]| 0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32;
    assert_eq!(
        lum(raised) > lum(bg),
        lum(fg) > lum(bg),
        "high contrast inverted a pair"
    );
}

#[test]
fn high_contrast_keeps_alpha() {
    let translucent = [0x7A, 0x86, 0xFF, 0x29];
    assert_eq!(
        super::high_contrast(translucent, crate::theme::BG_PANEL)[3],
        0x29,
        "a wash lost its transparency and became a block of colour"
    );
}

// ── AccessKit conversion ────────────────────────────────────────────────────

#[test]
fn a_tree_converts_to_an_accesskit_update_with_a_root_and_a_focus() {
    let mut ui = ui();
    let root = ui.root();
    let button = ui.add_node(
        ButtonBuilder::new(WidgetBuilder::new().with_tooltip("Save")).build(),
        root,
    );
    ui.perform_layout();
    ui.set_focus(button);

    let tree = A11yTree::from_ui(&ui);
    let update = tree.to_accesskit();

    assert_eq!(update.nodes.len(), tree.nodes.len());
    assert_eq!(update.focus.0, tree.focus);
    assert_eq!(
        update.tree.as_ref().map(|t| t.root.0),
        Some(tree.root),
        "an update with no root is one a platform adapter rejects"
    );
    let (_, ak_button) = update
        .nodes
        .iter()
        .find(|(id, _)| id.0 == button.index() as u64 + 1)
        .expect("the button is missing from the update");
    assert_eq!(ak_button.role(), accesskit::Role::Button);
    assert_eq!(ak_button.label().as_deref(), Some("Save"));
}

/// AccessKit's own rule, and the one that is easy to get backwards: a `Label`
/// node carries its text in `value`, not `label`.
#[test]
fn static_text_converts_into_value_and_not_into_label() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(
        TextBuilder::new(WidgetBuilder::new())
            .with_text("Seyda Neen")
            .build(),
        root,
    );
    ui.perform_layout();

    let update = A11yTree::from_ui(&ui).to_accesskit();
    let (_, node) = update
        .nodes
        .iter()
        .find(|(id, _)| id.0 == handle.index() as u64 + 1)
        .expect("the label is missing");
    assert_eq!(node.role(), accesskit::Role::Label);
    assert_eq!(node.value().as_deref(), Some("Seyda Neen"));
    assert_eq!(node.label(), None);
}

#[test]
fn a_disabled_control_converts_to_a_disabled_node() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(
        ButtonBuilder::new(
            WidgetBuilder::new()
                .with_tooltip("Save")
                .with_enabled(false),
        )
        .build(),
        root,
    );
    ui.perform_layout();

    let update = A11yTree::from_ui(&ui).to_accesskit();
    let (_, node) = update
        .nodes
        .iter()
        .find(|(id, _)| id.0 == handle.index() as u64 + 1)
        .expect("the button is missing");
    assert!(node.is_disabled());
}

#[test]
fn a_mixed_check_box_survives_the_conversion() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(
        CheckBoxBuilder::new(WidgetBuilder::new())
            .with_label("Cast shadows")
            .with_mixed(true)
            .build(),
        root,
    );
    ui.perform_layout();

    let update = A11yTree::from_ui(&ui).to_accesskit();
    let (_, node) = update
        .nodes
        .iter()
        .find(|(id, _)| id.0 == handle.index() as u64 + 1)
        .expect("the check box is missing");
    assert_eq!(node.toggled(), Some(accesskit::Toggled::Mixed));
}

#[test]
fn bounds_survive_the_conversion_as_a_rectangle_and_not_as_a_size() {
    let mut ui = ui();
    let root = ui.root();
    let handle = ui.add_node(
        ButtonBuilder::new(
            WidgetBuilder::new()
                .with_tooltip("Save")
                .with_width(120.0)
                .with_height(28.0)
                .with_horizontal_alignment(HorizontalAlignment::Left)
                .with_vertical_alignment(VerticalAlignment::Top),
        )
        .build(),
        root,
    );
    ui.perform_layout();

    let tree = A11yTree::from_ui(&ui);
    let update = tree.to_accesskit();
    let (_, node) = update
        .nodes
        .iter()
        .find(|(id, _)| id.0 == handle.index() as u64 + 1)
        .expect("the button is missing");
    let rect = node
        .bounds()
        .expect("no bounds; a reader cannot point at it");
    let want = tree.get(handle.index() as u64 + 1).unwrap().bounds;
    assert!((rect.x0 - want.x as f64).abs() < 0.5);
    assert!((rect.x1 - (want.x + want.w) as f64).abs() < 0.5);
    assert!((rect.y1 - rect.y0 - 28.0).abs() < 0.5, "{rect:?}");
}

/// Every id referenced as a child must be in the update, or a platform adapter
/// rejects the whole tree.
#[test]
fn every_child_id_in_the_update_exists_in_the_update() {
    let mut ui = ui();
    let root = ui.root();
    let button = ui.add_node(ButtonBuilder::new(WidgetBuilder::new()).build(), root);
    let border = ui.add_node(BorderBuilder::new(WidgetBuilder::new()).build(), button);
    ui.add_node(
        TextBuilder::new(WidgetBuilder::new())
            .with_text("Save")
            .build(),
        border,
    );
    ui.add_node(
        SliderBuilder::new(WidgetBuilder::new())
            .with_value(0.5)
            .build(),
        root,
    );
    ui.perform_layout();

    let update = A11yTree::from_ui(&ui).to_accesskit();
    let present: std::collections::HashSet<u64> = update.nodes.iter().map(|(id, _)| id.0).collect();
    for (id, node) in &update.nodes {
        for child in node.children() {
            assert!(
                present.contains(&child.0),
                "node {} names child {} which is not in the update",
                id.0,
                child.0
            );
        }
    }
    assert!(
        present.contains(&update.focus.0),
        "focus is not in the update"
    );
}
