//! MORROWIND-I — the accessibility tree.
//!
//! *"The row nobody plans and everybody eventually needs."* (`phase_MORROWIND.md` §8.)
//!
//! # What this is, and what it is not
//!
//! This module turns Somnium's widget tree into an **accessibility tree**: a
//! parallel structure a screen reader can walk, where every node has a *role*
//! (this is a button), a *name* (it says "Save"), a *value* where one applies
//! (the slider is at 0.4), and a state (focused, disabled, checked).
//!
//! It is **not** a conformance claim. §14.5 says so and this file agrees: an
//! accessibility tree is a necessary condition for a usable screen-reader
//! experience and nowhere near a sufficient one. What is delivered is the tree,
//! the role model, the announcement queue and the [`accesskit`] conversion.
//!
//! # Why a self-rendered UI has to do this at all
//!
//! A UI drawn with the platform's own controls gets an accessibility tree for
//! free, because the controls *are* the tree. Somnium draws every pixel itself,
//! so to a screen reader the editor is one opaque rectangle — and so is every
//! game built on it. **Godot 4.5 shipped AccessKit support** for exactly this
//! reason (`phase_MORROWIND.md` §6.9.2), which is what moved this sub-phase
//! from speculative to precedented: the hard integration questions have public
//! answers now.
//!
//! # The two things that are easy to get wrong
//!
//! 1. **The name is not the debug name.** `Widget::name` is what a developer
//!    called the node. The accessible name is what a *user* would call it, and
//!    for an icon-only toolbar button the best available answer is its
//!    **tooltip** — which the shell already authors, for the same reason and
//!    without knowing it. [`A11yNode::from_widget`] uses it as the fallback.
//! 2. **A tree that mirrors the widget tree exactly is too deep.** A button is
//!    a border containing a stack panel containing a text node, and reading
//!    that to somebody as three nested groups is worse than useless.
//!    [`A11yTree::from_ui`] **collapses** a subtree whose root has a role and
//!    whose descendants are all presentational, which is what makes the output
//!    navigable rather than merely correct.

use crate::message::NodeHandle;
use crate::types::Rect;
use crate::ui::UserInterface;

/// What a node *is*, to a screen reader.
///
/// A deliberately small set, chosen to map cleanly onto both AccessKit and the
/// widgets Somnium actually ships. A role Somnium has no widget for is a role
/// nothing can produce, and an unused variant is a promise nobody keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Role {
    /// The tree's root.
    Window,
    /// Structure with no semantics of its own — a panel, a border, a layout
    /// container. Collapsed away when it has nothing to say.
    #[default]
    Group,
    /// Static text.
    Label,
    Button,
    CheckBox,
    Slider,
    /// A single- or multi-line editable field.
    TextInput,
    ComboBox,
    /// A container whose children are selectable rows.
    List,
    ListItem,
    TabList,
    Tab,
    ScrollView,
    Menu,
    MenuItem,
    Image,
    /// A modal that takes focus.
    Dialog,
    /// Something that appeared and should be read without being focused — a
    /// toast, a validation failure.
    Alert,
}

impl Role {
    /// Whether a node with this role is worth telling a screen reader about.
    ///
    /// `Group` is the "no" — a border inside a button is scaffolding, and a
    /// reader that announces it makes the button harder to use, not easier.
    /// A `Group` survives only when it is a *container of interesting things*,
    /// which [`A11yTree::from_ui`] decides by looking at its children.
    pub fn is_meaningful(self) -> bool {
        !matches!(self, Role::Group)
    }

    /// Whether the role implies the node can take keyboard focus.
    pub fn is_focusable(self) -> bool {
        matches!(
            self,
            Role::Button
                | Role::CheckBox
                | Role::Slider
                | Role::TextInput
                | Role::ComboBox
                | Role::ListItem
                | Role::Tab
                | Role::MenuItem
        )
    }

    /// The durable name, for logs and for the AccessKit mapping.
    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Window => "window",
            Role::Group => "group",
            Role::Label => "label",
            Role::Button => "button",
            Role::CheckBox => "check box",
            Role::Slider => "slider",
            Role::TextInput => "text input",
            Role::ComboBox => "combo box",
            Role::List => "list",
            Role::ListItem => "list item",
            Role::TabList => "tab list",
            Role::Tab => "tab",
            Role::ScrollView => "scroll view",
            Role::Menu => "menu",
            Role::MenuItem => "menu item",
            Role::Image => "image",
            Role::Dialog => "dialog",
            Role::Alert => "alert",
        }
    }
}

/// A checkable node's state. Three-valued, because a tri-state checkbox is a
/// real thing Somnium ships (`CheckBox::mixed`) and "checked: bool" cannot say
/// what a mixed box means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggled {
    False,
    True,
    Mixed,
}

/// One node in the accessibility tree.
#[derive(Clone, Debug, PartialEq)]
pub struct A11yNode {
    /// Stable within a tree build. Derived from the widget handle's index, so
    /// the same widget keeps the same id across frames and a reader's cursor
    /// does not jump when an unrelated node is added.
    pub id: u64,
    pub role: Role,
    /// What a user would call it. Empty is allowed and is a finding, not a
    /// crash — see [`A11yTree::unnamed`].
    pub name: String,
    /// The current value, where one applies: a slider's number, a text box's
    /// contents, a combo box's selection.
    pub value: Option<String>,
    pub bounds: Rect,
    pub focused: bool,
    pub disabled: bool,
    pub toggled: Option<Toggled>,
    pub children: Vec<u64>,
}

/// How urgently an announcement should interrupt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Politeness {
    /// Wait for a gap. Status changes, "3 results".
    #[default]
    Polite,
    /// Interrupt. Errors, and anything that has just made the user's next
    /// action wrong.
    Assertive,
}

/// Something to say that is not a focus change.
#[derive(Clone, Debug, PartialEq)]
pub struct Announcement {
    pub text: String,
    pub politeness: Politeness,
}

/// The accessibility tree for one `UserInterface`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct A11yTree {
    pub nodes: Vec<A11yNode>,
    pub root: u64,
    /// The focused node's id, or the root when nothing is focused.
    pub focus: u64,
}

impl A11yTree {
    /// Build the tree from a live widget tree.
    ///
    /// Walks from the root, assigns a role to every node, and **collapses**
    /// presentational scaffolding: a `Group` with no name whose subtree
    /// contains nothing meaningful contributes nothing, and a `Group` that
    /// merely wraps a single meaningful child is replaced by that child. This
    /// is the difference between a tree that is correct and one that is
    /// navigable.
    pub fn from_ui(ui: &UserInterface) -> Self {
        let root_handle = ui.root();
        let mut tree = Self {
            nodes: Vec::new(),
            root: id_of(root_handle),
            focus: id_of(root_handle),
        };
        let focused = ui.focused();
        tree.visit(ui, root_handle, focused, true);
        if let Some(node) = tree.nodes.iter().find(|n| n.focused) {
            tree.focus = node.id;
        }
        // The root always exists, even for an empty tree: a reader handed a
        // tree with no root has nothing to attach its cursor to.
        if !tree.nodes.iter().any(|n| n.id == tree.root) {
            tree.nodes.push(A11yNode {
                id: tree.root,
                role: Role::Window,
                name: String::new(),
                value: None,
                bounds: ui.screen_bounds(root_handle),
                focused: false,
                disabled: false,
                toggled: None,
                children: Vec::new(),
            });
        }
        tree
    }

    /// Depth-first walk. Returns the ids this subtree contributes to its
    /// parent — which is *not* always one id: a collapsed group contributes its
    /// children directly, and an invisible subtree contributes none.
    fn visit(
        &mut self,
        ui: &UserInterface,
        handle: NodeHandle,
        focused: NodeHandle,
        is_root: bool,
    ) -> Vec<u64> {
        let Some(node) = ui.a11y_probe(handle) else {
            return Vec::new();
        };
        if !node.visible {
            // A hidden widget is not "a widget the user cannot see" — to a
            // screen reader it must not exist at all, or the reader will read
            // out a menu that is closed.
            return Vec::new();
        }

        let mut children = Vec::new();
        for child in node.children.iter().copied() {
            children.extend(self.visit(ui, child, focused, false));
        }

        let role = if is_root { Role::Window } else { node.role };
        let name = node.name;
        let has_own_meaning = role.is_meaningful() || !name.is_empty() || node.value.is_some();

        if !is_root && !has_own_meaning {
            // Pure scaffolding. Its children stand in for it, which is what
            // stops a button reading as three nested groups.
            return children;
        }
        if !is_root && role == Role::Group && children.len() == 1 {
            // A group wrapping exactly one meaningful thing is the same thing.
            return children;
        }

        let id = id_of(handle);
        self.nodes.push(A11yNode {
            id,
            role,
            name,
            value: node.value,
            bounds: node.bounds,
            focused: handle == focused && !is_root,
            disabled: !node.enabled,
            toggled: node.toggled,
            children,
        });
        vec![id]
    }

    /// The node with an id, if any.
    pub fn get(&self, id: u64) -> Option<&A11yNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Focusable nodes with no accessible name.
    ///
    /// **The single most useful diagnostic in this module.** An unnamed button
    /// reads as "button" and nothing else, which is the most common real
    /// accessibility failure in a self-rendered UI and is invisible to everyone
    /// who is not using a reader. Surfaced as a list rather than a warning so a
    /// test can assert on it — see `the_shell_names_everything_focusable`.
    pub fn unnamed(&self) -> Vec<&A11yNode> {
        self.nodes
            .iter()
            .filter(|n| n.role.is_focusable() && n.name.trim().is_empty())
            .collect()
    }

    /// What a reader would say when focus lands on `id`.
    ///
    /// Name, then role, then value, then state — the order every major screen
    /// reader uses, and the reason it is that order is that the name is what
    /// the user is looking for and the rest is qualification.
    pub fn announce_focus(&self, id: u64) -> Option<String> {
        let node = self.get(id)?;
        let mut parts = Vec::new();
        if !node.name.trim().is_empty() {
            parts.push(node.name.trim().to_string());
        }
        parts.push(node.role.as_str().to_string());
        if let Some(value) = &node.value {
            parts.push(value.clone());
        }
        match node.toggled {
            Some(Toggled::True) => parts.push("checked".into()),
            Some(Toggled::False) => parts.push("not checked".into()),
            Some(Toggled::Mixed) => parts.push("partially checked".into()),
            None => {}
        }
        if node.disabled {
            parts.push("dimmed".into());
        }
        Some(parts.join(", "))
    }
}

/// What `UserInterface` reports about one node, for tree building.
///
/// A plain struct rather than a borrow of the node, so `a11y.rs` never needs to
/// know how the pool is shaped and `A11yTree` stays testable without one.
pub struct A11yProbe {
    pub role: Role,
    pub name: String,
    pub value: Option<String>,
    pub bounds: Rect,
    pub visible: bool,
    pub enabled: bool,
    pub toggled: Option<Toggled>,
    pub children: Vec<NodeHandle>,
}

/// Widget handle to accessibility id.
///
/// The handle's index, not its raw bits: a generation bump means the widget was
/// replaced, and a reader should treat a replaced widget as the same *place* in
/// the interface rather than as a brand-new node its cursor has never seen.
fn id_of(handle: NodeHandle) -> u64 {
    // Offset by one so the root is never zero — AccessKit treats a zero id as
    // valid, but a zero that also means "unset" in Somnium's pool would be one
    // ambiguity too many.
    handle.index() as u64 + 1
}

// ── AccessKit ────────────────────────────────────────────────────────────────
//
// The conversion, kept in one place and away from the tree building. `A11yTree`
// is Somnium's model and knows nothing about AccessKit; this is the adapter,
// and it is short because the role set was chosen to map 1:1 (see `Role`'s doc
// comment — a role Somnium has no widget for is a role nothing can produce).

impl Role {
    /// The AccessKit role.
    ///
    /// Total, and deliberately so: a `match` with no wildcard means adding a
    /// Somnium role is a compile error here rather than a silent
    /// `GenericContainer` that a reader announces as nothing.
    pub fn to_accesskit(self) -> accesskit::Role {
        use accesskit::Role as Ak;
        match self {
            Role::Window => Ak::Window,
            Role::Group => Ak::GenericContainer,
            Role::Label => Ak::Label,
            Role::Button => Ak::Button,
            Role::CheckBox => Ak::CheckBox,
            Role::Slider => Ak::Slider,
            Role::TextInput => Ak::TextInput,
            Role::ComboBox => Ak::ComboBox,
            Role::List => Ak::List,
            Role::ListItem => Ak::ListItem,
            Role::TabList => Ak::TabList,
            Role::Tab => Ak::Tab,
            Role::ScrollView => Ak::ScrollView,
            Role::Menu => Ak::Menu,
            Role::MenuItem => Ak::MenuItem,
            Role::Image => Ak::Image,
            Role::Dialog => Ak::Dialog,
            Role::Alert => Ak::Alert,
        }
    }
}

impl From<Toggled> for accesskit::Toggled {
    fn from(value: Toggled) -> Self {
        match value {
            Toggled::False => accesskit::Toggled::False,
            Toggled::True => accesskit::Toggled::True,
            Toggled::Mixed => accesskit::Toggled::Mixed,
        }
    }
}

impl A11yNode {
    /// This node as an AccessKit node.
    pub fn to_accesskit(&self) -> accesskit::Node {
        let mut node = accesskit::Node::new(self.role.to_accesskit());
        if !self.name.trim().is_empty() {
            // A `Label` node carries its text in `value`, not `label` — that is
            // AccessKit's rule and getting it backwards makes static text read
            // as an empty label with a tooltip.
            if self.role == Role::Label {
                node.set_value(self.name.clone());
            } else {
                node.set_label(self.name.clone());
            }
        }
        if let Some(value) = &self.value {
            node.set_value(value.clone());
        }
        if let Some(toggled) = self.toggled {
            node.set_toggled(toggled.into());
        }
        if self.disabled {
            node.set_disabled();
        }
        node.set_bounds(accesskit::Rect {
            x0: self.bounds.x as f64,
            y0: self.bounds.y as f64,
            x1: (self.bounds.x + self.bounds.w) as f64,
            y1: (self.bounds.y + self.bounds.h) as f64,
        });
        node.set_children(
            self.children
                .iter()
                .copied()
                .map(accesskit::NodeId)
                .collect::<Vec<_>>(),
        );
        node
    }
}

impl A11yTree {
    /// The whole tree as an AccessKit update.
    ///
    /// A full update every time rather than a diff. Somnium's trees are in the
    /// hundreds of nodes and the shell rebuilds the widget tree freely; a diff
    /// would be an optimisation whose correctness depends on the *previous*
    /// tree being right, which is exactly the assumption that makes stale
    /// accessibility state so hard to debug. If this ever shows up in a profile,
    /// diff it then, with the full update kept as the reference implementation.
    pub fn to_accesskit(&self) -> accesskit::TreeUpdate {
        accesskit::TreeUpdate {
            nodes: self
                .nodes
                .iter()
                .map(|n| (accesskit::NodeId(n.id), n.to_accesskit()))
                .collect(),
            // The main tree, not a subtree. AccessKit 0.24 added subtree
            // grafting; Somnium has one window and one widget tree per
            // interface, so ROOT is the whole story and a second TreeId would
            // be a distinction with nothing on the other side of it.
            tree_id: accesskit::TreeId::ROOT,
            tree: Some(accesskit::Tree::new(accesskit::NodeId(self.root))),
            focus: accesskit::NodeId(self.focus),
        }
    }
}

// ── Reduced motion and contrast ──────────────────────────────────────────────

/// The accessibility preferences a running UI honours.
///
/// Two settings, each of which changes *rendering* and neither of which changes
/// *layout*. That is the invariant: an interface with reduced motion and high
/// contrast on must be the same interface, in the same places, or the two modes
/// are two products and only one of them gets tested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct A11ySettings {
    /// Complete every motion track instantly. MORROWIND-H's animator already
    /// implements this; the sub-phase's job is to make it reachable from a
    /// setting rather than only from editor code.
    pub reduced_motion: bool,
    /// Raise contrast using Zeta's *certified* pairs.
    pub high_contrast: bool,
}

impl A11ySettings {
    /// Read the platform's own preferences where they are cheap to get.
    ///
    /// **Returns defaults today, and that is deliberate.** Windows exposes this
    /// through `SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)` and macOS
    /// through `NSWorkspace.accessibilityDisplayShouldReduceMotion`, and both
    /// are a platform-crate dependency this sub-phase declined to add for one
    /// boolean. The function exists so the *call site* is already right: the
    /// shell asks the platform first and falls back to the stored setting, and
    /// wiring the real query later changes this body and nothing else.
    pub fn from_platform() -> Self {
        Self::default()
    }
}

/// Raise a foreground colour against its background until it clears the
/// enhanced contrast bar.
///
/// **Reuses Zeta's certified pairs rather than inventing a second palette**,
/// which §8 requires and which is not a stylistic preference: Zeta's tokens are
/// certified at specific ratios (`theme.rs`'s contrast tests), and a
/// high-contrast mode built from a *different* set of colours would be a second
/// palette with no certification at all, which is how a mode meant to help ends
/// up worse than the one it replaces.
///
/// So this does not pick new colours. It walks the *existing* foreground toward
/// whichever pole its background is not — white if the background is dark, black
/// if it is light — until the ratio clears [`ENHANCED_CONTRAST`]. The result:
///
/// - is always at least as contrasty as the input (never worse);
/// - keeps the pair's polarity, so light-on-dark stays light-on-dark and a
///   mode meant to help does not invert one pair and not another;
/// - keeps alpha, so a translucent wash stays a wash rather than becoming a
///   block of colour over the thing it was washing.
///
/// A colour that already clears the bar is returned unchanged, which means the
/// mode is a no-op on the parts of the interface Zeta already got right.
pub fn high_contrast(fg: crate::theme::Color, bg: crate::theme::Color) -> crate::theme::Color {
    if ratio(fg, bg) >= ENHANCED_CONTRAST {
        return fg;
    }

    // Which pole to walk toward: away from the background, so the polarity of
    // the pair is preserved by construction rather than by a check afterwards.
    let bg_luma = luma(bg);
    let target: [u8; 3] = if bg_luma < 0.5 {
        [0xFF, 0xFF, 0xFF]
    } else {
        [0x00, 0x00, 0x00]
    };

    // Binary search on the blend. Sixteen steps resolves finer than one 8-bit
    // channel, so the loop terminates on precision rather than on iterations.
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    let mut best = fg;
    for _ in 0..16 {
        let mid = 0.5 * (low + high);
        let candidate = blend(fg, target, mid);
        if ratio(candidate, bg) >= ENHANCED_CONTRAST {
            best = candidate;
            high = mid;
        } else {
            low = mid;
        }
    }
    // If even the pole does not clear the bar, take the pole: it is the most
    // contrast this background admits, and returning the original would be
    // worse for no reason.
    if ratio(best, bg) < ENHANCED_CONTRAST {
        best = blend(fg, target, 1.0);
    }
    best
}

/// [crate::theme::contrast_ratio] over raw byte colours.
///
/// The theme states contrast in `Srgb8`, which is the right type for a token
/// sheet and a wrapper this module would otherwise construct four times a call.
fn ratio(a: crate::theme::Color, b: crate::theme::Color) -> f32 {
    crate::theme::contrast_ratio(crate::theme::Srgb8(a), crate::theme::Srgb8(b))
}

/// WCAG AAA for body text.
///
/// Zeta certifies its pairs at the *normal* bar, which is the right default.
/// The whole point of a high-contrast mode is the users for whom that bar is
/// not enough, so the mode targets the enhanced one — a mode that merely
/// re-achieved the default would be a switch that does nothing.
pub const ENHANCED_CONTRAST: f32 = 7.0;

/// Relative luminance, 0..=1, ignoring alpha.
fn luma(c: crate::theme::Color) -> f32 {
    (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0
}

/// Blend `from` toward an opaque `target` by `t`, keeping `from`'s alpha.
fn blend(from: crate::theme::Color, target: [u8; 3], t: f32) -> crate::theme::Color {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| {
        (a as f32 + (b as f32 - a as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [
        mix(from[0], target[0]),
        mix(from[1], target[1]),
        mix(from[2], target[2]),
        from[3],
    ]
}

#[cfg(test)]
mod tests;
