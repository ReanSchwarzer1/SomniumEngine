//! The dock tree (MORROWIND-J, step 1).
//!
//! Tiles, splitters and tabs, as data. This module owns *what the arrangement
//! is* and answers one question — given a rectangle, where does every panel go
//! — while the shell owns turning that answer into widgets.
//!
//! # Why this is a separate module and not more of `shell.rs`
//!
//! Phase 26 chose a fixed five-region shell with named workspace presets,
//! explicitly ruling out arbitrary docking: *"excellent defaults beat unlimited
//! docking"*. That was right for a phase whose job was the information
//! architecture, and it is what `workspace.rs` still encodes. MORROWIND-J's
//! first requirement changes the ceiling rather than the default:
//!
//! > *"A dock tree (tiles, splitters, tabs) replacing the fixed five-region
//! > shell, with the current arrangement as the **default layout** so nothing
//! > looks different on first run."*
//!
//! The tree is therefore built to reproduce the shipped arrangement exactly
//! ([`DockTree::default_layout`]) and to allow arrangements the old model could
//! not express. `shell.rs` is already 1 850 lines and `context.md` names
//! `UiManager` and `Widget` among the largest hubs in the tree; putting the
//! layout algebra there would have deepened the two things least able to take
//! it. Here it is GPU-free, `winit`-free and testable by calling one function.
//!
//! # The interface
//!
//! - [`DockTree::default_layout`] — the shipped arrangement.
//! - [`DockTree::resolve`] — the whole layout algebra: rectangles for every
//!   visible panel and every splitter, in one pass.
//! - [`DockTree::dock`], [`close`](DockTree::close), [`activate`](DockTree::activate),
//!   [`set_ratio`](DockTree::set_ratio) — the four mutations a docking UI needs.
//! - `serde` on the whole thing, so persistence is `serde_json` and not a
//!   bespoke format.
//!
//! Everything else — collapsing a tile that lost its last tab, promoting the
//! surviving sibling of a dead splitter, keeping `active` in range, keeping a
//! panel from appearing twice, clamping a ratio so neither side vanishes — is
//! implementation. A caller says *"dock Details to the right of Viewport"* and
//! never performs tree surgery.
//!
//! # References
//!
//! `FlaxEngine-master/Source/Editor/GUI/Docking/` and `fyrox-ui/src/dock/`
//! (ATTRIBUTION.md §13). Both keep the same shape — a binary split tree whose
//! leaves are tab sets. Neither was copied; the layout arithmetic and the
//! collapse rules here are written against Somnium's `Rect` and its existing
//! splitter minimums.

use crate::types::Rect;
use serde::{Deserialize, Serialize};

/// Which way a splitter divides its tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    /// Children sit side by side; the splitter is vertical.
    Horizontal,
    /// Children sit one above the other; the splitter is horizontal.
    Vertical,
}

/// A dockable panel, named rather than numbered.
///
/// A name survives a reorder, a rename of a Rust type and a released build,
/// which an index does not — and a layout file that has to be thrown away
/// whenever the panel list changes is a layout file nobody keeps.
pub type PanelId = String;

/// Where a panel lands relative to an existing one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockSide {
    Left,
    Right,
    Above,
    Below,
    /// Join the target's tab set instead of splitting it.
    Tab,
}

/// The route from the root to a node: `false` takes the first child, `true` the
/// second.
///
/// Splitter drags need to name a splitter, and a path is the only identity a
/// tree of anonymous nodes has that survives serialisation. Short by
/// construction — an editor layout is a handful of levels deep.
pub type NodePath = Vec<bool>;

/// One tile of the tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DockNode {
    /// Two children divided at `ratio` of the tile along `axis`.
    Split {
        axis: Axis,
        /// The first child's share, in `0.05..=0.95`.
        ratio: f32,
        first: Box<DockNode>,
        second: Box<DockNode>,
    },
    /// A tab set. **Never empty** — a tile that loses its last panel is removed
    /// by the operation that emptied it.
    Tabs { panels: Vec<PanelId>, active: usize },
}

/// A panel's place in a resolved layout.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelRect {
    pub panel: PanelId,
    /// The tile's rectangle, tab strip included.
    pub rect: Rect,
    /// Whether this panel is the visible one in its tab set.
    pub active: bool,
    /// Every panel sharing this tile, in tab order, this one included.
    pub siblings: Vec<PanelId>,
}

/// A splitter's place in a resolved layout, and the handle a drag needs.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitterRect {
    pub path: NodePath,
    pub axis: Axis,
    pub rect: Rect,
    /// The tile the splitter divides, which is what a drag measures against.
    pub tile: Rect,
}

/// What [`DockTree::resolve`] produces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub panels: Vec<PanelRect>,
    pub splitters: Vec<SplitterRect>,
}

impl Layout {
    /// The tile a panel occupies, if it is in the tree.
    #[must_use]
    pub fn rect_of(&self, panel: &str) -> Option<Rect> {
        self.panels
            .iter()
            .find(|p| p.panel == panel)
            .map(|p| p.rect)
    }
}

/// Splitter thickness in pixels, matching the shell's existing splitters.
pub const SPLITTER: f32 = 6.0;

/// The smallest a tile may be resolved to before a ratio is overridden.
///
/// Ratios are fractions, so a window narrow enough turns any fraction into a
/// few pixels. Clamping at *resolve* time rather than at drag time is what lets
/// a layout survive a trip to a small monitor and back: the stored ratio is
/// untouched and comes back when the window does.
pub const MIN_TILE: f32 = 80.0;

/// The lowest and highest share a splitter may store.
const MIN_RATIO: f32 = 0.05;
const MAX_RATIO: f32 = 0.95;

/// The panels that may occupy the shared bottom row.
///
/// Named here rather than in `workspace.rs` because [`DockTree::chrome`] has to
/// recognise one when it reads a tree back, and a list the reader and the
/// writer both consult is one list.
pub const BOTTOM_PANELS: [&str; 3] = ["Content Drawer", "Output Log", "Jobs"];

/// The five-region projection of a tree, in pixels.
///
/// What the shell has always consumed. Kept as a projection rather than as the
/// representation, so an arrangement the projection cannot describe is a
/// `None` rather than a lie.
#[derive(Clone, Debug, PartialEq)]
pub struct Chrome {
    pub tools: f32,
    pub details: f32,
    pub outliner: f32,
    pub drawer_height: f32,
    /// The active bottom panel, or `None` when the tree has no bottom tile.
    pub bottom: Option<PanelId>,
    pub viewport: f32,
}

/// A dock arrangement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DockTree {
    root: DockNode,
}

impl Default for DockTree {
    fn default() -> Self {
        Self::default_layout()
    }
}

impl DockTree {
    /// The arrangement the editor has shipped since Phase 26.
    ///
    /// `tools | viewport | (outliner / details)`, with the bottom panel under
    /// the viewport. The ratios are the pixel defaults from
    /// [`crate::layout_persist`] expressed against a 1920×1080 window, so first
    /// run looks the way it always has.
    #[must_use]
    pub fn default_layout() -> Self {
        // Ratios are shares of the space a splitter leaves, not of the tile,
        // so each one is derived against the span its own parent actually
        // hands it. Writing `168.0 / 1920.0` instead gave a 337.8 px Details
        // column: close enough to look right and wrong enough to fail the test
        // that says first run is unchanged.
        const W: f32 = 1920.0;
        const H: f32 = 1080.0;
        /// 168 px of tools out of the width the root splitter leaves.
        const TOOLS: f32 = 168.0 / (W - SPLITTER);
        /// A 340 px right column out of what is left after the tools strip and
        /// the second splitter.
        const CONTENT: f32 = W - SPLITTER - 168.0 - SPLITTER;
        const VIEWPORT: f32 = (CONTENT - 340.0) / CONTENT;
        /// 300 px of outliner out of the column height.
        const OUTLINER: f32 = 300.0 / (H - SPLITTER);
        /// The bottom panel is a quarter of the viewport column.
        const BOTTOM: f32 = 0.75;

        Self {
            root: DockNode::Split {
                axis: Axis::Horizontal,
                ratio: TOOLS,
                first: Box::new(DockNode::tabs(["Tools"])),
                second: Box::new(DockNode::Split {
                    axis: Axis::Horizontal,
                    ratio: VIEWPORT,
                    first: Box::new(DockNode::Split {
                        axis: Axis::Vertical,
                        ratio: BOTTOM,
                        first: Box::new(DockNode::tabs(["Viewport"])),
                        second: Box::new(DockNode::tabs(["Content Drawer", "Output Log", "Jobs"])),
                    }),
                    second: Box::new(DockNode::Split {
                        axis: Axis::Vertical,
                        ratio: OUTLINER,
                        first: Box::new(DockNode::tabs(["Outliner"])),
                        second: Box::new(DockNode::tabs(["Details"])),
                    }),
                }),
            },
        }
    }

    /// Build the shipped shape with ratios encoding an explicit arrangement.
    ///
    /// This is what makes the tree load-bearing rather than a model nobody
    /// calls: [`crate::workspace::Workspace::preset`] states its intent in
    /// pixels — a 220 px tool rail for Terrain, a 400 px Details for Lighting —
    /// and then goes through here and back, so the arrangements the editor
    /// actually ships are *expressed as dock trees* while the design decisions
    /// stay where they were written.
    ///
    /// `bottom` of `None` produces no bottom tile at all, which is the
    /// arrangement the old model spelled `BottomPanel::None` and could not
    /// otherwise represent.
    #[must_use]
    pub fn from_chrome(
        tools: f32,
        details: f32,
        outliner: f32,
        drawer_height: f32,
        bottom: Option<&str>,
        window_w: f32,
        window_h: f32,
    ) -> Self {
        let width = window_w.max(1.0);
        let height = window_h.max(1.0);
        // Each ratio is a share of what its own parent's splitter leaves. See
        // `default_layout` for why the naive `pixels / span` is wrong.
        let root_usable = (width - SPLITTER).max(1.0);
        let content = (root_usable - tools - SPLITTER).max(1.0);

        let viewport_column = match bottom {
            Some(active) => {
                let column_usable = (height - SPLITTER).max(1.0);
                let viewport_h = (column_usable - drawer_height).max(1.0);
                DockNode::Split {
                    axis: Axis::Vertical,
                    ratio: viewport_h / column_usable,
                    first: Box::new(DockNode::tabs(["Viewport"])),
                    second: Box::new(bottom_tabs(active)),
                }
            }
            None => DockNode::tabs(["Viewport"]),
        };

        let right_usable = (height - SPLITTER).max(1.0);
        Self {
            root: DockNode::Split {
                axis: Axis::Horizontal,
                ratio: tools / root_usable,
                first: Box::new(DockNode::tabs(["Tools"])),
                second: Box::new(DockNode::Split {
                    axis: Axis::Horizontal,
                    ratio: (content - details) / content,
                    first: Box::new(viewport_column),
                    second: Box::new(DockNode::Split {
                        axis: Axis::Vertical,
                        ratio: outliner / right_usable,
                        first: Box::new(DockNode::tabs(["Outliner"])),
                        second: Box::new(DockNode::tabs(["Details"])),
                    }),
                }),
            },
        }
    }

    /// Read an arrangement back out as the pixel widths the shell consumes.
    ///
    /// The inverse of [`from_chrome`](Self::from_chrome), and the reason the
    /// shell did not have to change to gain a dock tree: it still asks for
    /// four numbers and a bottom-panel choice, and now those are *derived* from
    /// a tree instead of being the only representation there is.
    ///
    /// Returns `None` for a tree that is not the shipped five-region shape —
    /// once a user docks something the old projection stops being meaningful,
    /// and saying so is better than returning plausible numbers for an
    /// arrangement they do not describe.
    #[must_use]
    pub fn chrome(&self, window_w: f32, window_h: f32) -> Option<Chrome> {
        let layout = self.resolve(Rect {
            x: 0.0,
            y: 0.0,
            w: window_w,
            h: window_h,
        });
        let tools = layout.rect_of("Tools")?;
        let details = layout.rect_of("Details")?;
        let outliner = layout.rect_of("Outliner")?;
        let viewport = layout.rect_of("Viewport")?;
        let bottom = layout
            .panels
            .iter()
            .find(|p| p.active && BOTTOM_PANELS.contains(&p.panel.as_str()));
        Some(Chrome {
            tools: tools.w,
            details: details.w,
            outliner: outliner.h,
            drawer_height: bottom.map_or(0.0, |p| p.rect.h),
            bottom: bottom.map(|p| p.panel.clone()),
            viewport: viewport.w,
        })
    }

    /// A tree holding one panel, for tests and for a workspace built from
    /// nothing.
    #[must_use]
    pub fn single(panel: impl Into<PanelId>) -> Self {
        Self {
            root: DockNode::tabs([panel.into()]),
        }
    }

    /// Rectangles for every panel and splitter inside `area`.
    ///
    /// The only function the shell needs in order to draw. Tiles that would
    /// fall below [`MIN_TILE`] borrow from their sibling; a tile smaller than
    /// two minimums plus a splitter is divided evenly, because at that size
    /// there is no honest answer and an even split is at least a predictable
    /// one.
    #[must_use]
    pub fn resolve(&self, area: Rect) -> Layout {
        let mut layout = Layout::default();
        resolve_node(&self.root, area, &mut Vec::new(), &mut layout);
        layout
    }

    /// Every panel in the tree, in depth-first order.
    #[must_use]
    pub fn panels(&self) -> Vec<PanelId> {
        let mut out = Vec::new();
        collect(&self.root, &mut out);
        out
    }

    /// Whether the tree holds this panel.
    #[must_use]
    pub fn contains(&self, panel: &str) -> bool {
        self.panels().iter().any(|p| p == panel)
    }

    /// Dock `panel` beside `target`, removing it from wherever it was.
    ///
    /// Returns `false` and changes nothing when `target` is absent, or when
    /// `panel` and `target` are the same panel — a request to dock something
    /// next to itself is a no-op and not an error, because it is what a drag
    /// that ends where it started means.
    ///
    /// Moving a panel out of a tile that then holds nothing collapses the tile
    /// and promotes its sibling, which is why removal happens first: docking
    /// into the tree and then repairing it is how a stale empty tile survives.
    pub fn dock(&mut self, panel: impl Into<PanelId>, target: &str, side: DockSide) -> bool {
        let panel = panel.into();
        if panel == target || !self.contains(target) {
            return false;
        }
        self.close(&panel);
        // `close` cannot have removed the target: it is a different panel, and
        // collapsing a tile never deletes a panel that was not being closed.
        dock_into(&mut self.root, &panel, target, side);
        self.repair();
        true
    }

    /// Remove a panel. Returns whether it was there.
    ///
    /// The last panel of the last tile cannot be closed: an empty dock tree has
    /// no rectangle to drop anything back into, so the arrangement would be
    /// unrecoverable without a reset.
    pub fn close(&mut self, panel: &str) -> bool {
        if !self.contains(panel) {
            return false;
        }
        if self.panels().len() == 1 {
            return false;
        }
        remove(&mut self.root, panel);
        self.repair();
        true
    }

    /// Bring a panel to the front of its tab set. Returns whether it was there.
    pub fn activate(&mut self, panel: &str) -> bool {
        activate_in(&mut self.root, panel)
    }

    /// Move a splitter. `ratio` is the first child's share and is clamped.
    ///
    /// Returns `false` when the path does not name a splitter, which is what a
    /// drag against a layout that changed underneath it looks like.
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) -> bool {
        let Some(node) = node_at_mut(&mut self.root, path) else {
            return false;
        };
        match node {
            DockNode::Split { ratio: r, .. } => {
                *r = ratio.clamp(MIN_RATIO, MAX_RATIO);
                true
            }
            DockNode::Tabs { .. } => false,
        }
    }

    /// Restore the invariants after a mutation, and after a deserialise.
    ///
    /// Public because loading a layout file is exactly the case where the
    /// invariants cannot be assumed: the file may have been written by an older
    /// build, hand-edited, or truncated. A tree that has been repaired is safe
    /// to resolve.
    pub fn repair(&mut self) {
        repair_node(&mut self.root);
        // Every mutation above preserves at least one panel, so an empty root
        // can only come from a deserialised file. A layout with nothing in it
        // is not repairable — there is no rectangle to dock into — so it is
        // replaced wholesale rather than papered over.
        if self.panels().is_empty() {
            *self = Self::default_layout();
        }
        dedupe(&mut self.root, &mut Vec::new());
        repair_node(&mut self.root);
    }
}

impl DockNode {
    fn tabs<I, S>(panels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<PanelId>,
    {
        Self::Tabs {
            panels: panels.into_iter().map(Into::into).collect(),
            active: 0,
        }
    }
}

// ── Resolution ──────────────────────────────────────────────────────────────

fn resolve_node(node: &DockNode, area: Rect, path: &mut NodePath, out: &mut Layout) {
    match node {
        DockNode::Tabs { panels, active } => {
            for (index, panel) in panels.iter().enumerate() {
                out.panels.push(PanelRect {
                    panel: panel.clone(),
                    rect: area,
                    active: index == *active,
                    siblings: panels.clone(),
                });
            }
        }
        DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let span = match axis {
                Axis::Horizontal => area.w,
                Axis::Vertical => area.h,
            };
            let usable = (span - SPLITTER).max(0.0);
            // The collapsed Tools rail is chrome, not an authoring panel. Preserve
            // its 48-unit intent while retaining the 80-unit minimum elsewhere.
            let rail = *axis == Axis::Horizontal
                && matches!(first.as_ref(), DockNode::Tabs { panels, .. } if panels.len() == 1 && panels[0] == "Tools");
            let first_span = if rail && usable >= 48.0 + MIN_TILE {
                (usable * ratio.clamp(0.0, 1.0)).clamp(48.0, usable - MIN_TILE)
            } else {
                split_span(usable, *ratio)
            };
            let second_span = usable - first_span;

            let (first_rect, splitter_rect, second_rect) = match axis {
                Axis::Horizontal => (
                    Rect {
                        w: first_span,
                        ..area
                    },
                    Rect {
                        x: area.x + first_span,
                        w: SPLITTER,
                        ..area
                    },
                    Rect {
                        x: area.x + first_span + SPLITTER,
                        w: second_span,
                        ..area
                    },
                ),
                Axis::Vertical => (
                    Rect {
                        h: first_span,
                        ..area
                    },
                    Rect {
                        y: area.y + first_span,
                        h: SPLITTER,
                        ..area
                    },
                    Rect {
                        y: area.y + first_span + SPLITTER,
                        h: second_span,
                        ..area
                    },
                ),
            };

            out.splitters.push(SplitterRect {
                path: path.clone(),
                axis: *axis,
                rect: splitter_rect,
                tile: area,
            });

            path.push(false);
            resolve_node(first, first_rect, path, out);
            path.pop();
            path.push(true);
            resolve_node(second, second_rect, path, out);
            path.pop();
        }
    }
}

/// The first child's pixels, honouring [`MIN_TILE`] where there is room for it.
fn split_span(usable: f32, ratio: f32) -> f32 {
    if usable < MIN_TILE * 2.0 {
        // No arrangement satisfies both minimums. Halving is at least stable
        // and symmetric, so a window being dragged small and back does not
        // shuffle the panels on the way.
        return usable * 0.5;
    }
    (usable * ratio).clamp(MIN_TILE, usable - MIN_TILE)
}

// ── Tree surgery ────────────────────────────────────────────────────────────

fn collect(node: &DockNode, out: &mut Vec<PanelId>) {
    match node {
        DockNode::Tabs { panels, .. } => out.extend(panels.iter().cloned()),
        DockNode::Split { first, second, .. } => {
            collect(first, out);
            collect(second, out);
        }
    }
}

fn node_at_mut<'a>(node: &'a mut DockNode, path: &[bool]) -> Option<&'a mut DockNode> {
    let Some((step, rest)) = path.split_first() else {
        return Some(node);
    };
    match node {
        DockNode::Tabs { .. } => None,
        DockNode::Split { first, second, .. } => {
            node_at_mut(if *step { second } else { first }, rest)
        }
    }
}

fn remove(node: &mut DockNode, panel: &str) {
    match node {
        DockNode::Tabs { panels, active } => {
            if let Some(index) = panels.iter().position(|p| p == panel) {
                panels.remove(index);
                // Keep the *same* panel in front where possible: closing a tab
                // to the left of the active one must not change what is shown.
                if *active > index || (*active == index && *active == panels.len()) {
                    *active = active.saturating_sub(1);
                }
            }
        }
        DockNode::Split { first, second, .. } => {
            remove(first, panel);
            remove(second, panel);
        }
    }
}

fn dock_into(node: &mut DockNode, panel: &str, target: &str, side: DockSide) -> bool {
    match node {
        DockNode::Tabs { panels, active } => {
            if !panels.iter().any(|p| p == target) {
                return false;
            }
            if side == DockSide::Tab {
                panels.push(panel.to_string());
                *active = panels.len() - 1;
                return true;
            }
            // The target tile becomes a split of itself and a new tile. Every
            // tab that shared the tile travels with the target, which is what
            // dragging one tab out of a group and dropping it beside the group
            // has to mean.
            let axis = match side {
                DockSide::Left | DockSide::Right => Axis::Horizontal,
                _ => Axis::Vertical,
            };
            let existing = DockNode::Tabs {
                panels: std::mem::take(panels),
                active: *active,
            };
            let fresh = DockNode::tabs([panel.to_string()]);
            let (first, second) = match side {
                DockSide::Left | DockSide::Above => (fresh, existing),
                _ => (existing, fresh),
            };
            *node = DockNode::Split {
                axis,
                ratio: 0.5,
                first: Box::new(first),
                second: Box::new(second),
            };
            true
        }
        DockNode::Split { first, second, .. } => {
            dock_into(first, panel, target, side) || dock_into(second, panel, target, side)
        }
    }
}

fn activate_in(node: &mut DockNode, panel: &str) -> bool {
    match node {
        DockNode::Tabs { panels, active } => {
            if let Some(index) = panels.iter().position(|p| p == panel) {
                *active = index;
                true
            } else {
                false
            }
        }
        DockNode::Split { first, second, .. } => {
            activate_in(first, panel) || activate_in(second, panel)
        }
    }
}

/// Collapse empty tiles, promote lone children, and clamp what is left.
fn repair_node(node: &mut DockNode) {
    if let DockNode::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        repair_node(first);
        repair_node(second);
        let rail = matches!(first.as_ref(), DockNode::Tabs { panels, .. } if panels.len() == 1 && panels[0] == "Tools");
        *ratio = ratio.clamp(if rail { 0.001 } else { MIN_RATIO }, MAX_RATIO);

        let first_empty = is_empty(first);
        let second_empty = is_empty(second);
        if first_empty && second_empty {
            *node = DockNode::Tabs {
                panels: Vec::new(),
                active: 0,
            };
        } else if first_empty {
            *node = (**second).clone();
        } else if second_empty {
            *node = (**first).clone();
        }
    }
    if let DockNode::Tabs { panels, active } = node
        && *active >= panels.len()
    {
        *active = panels.len().saturating_sub(1);
    }
}

/// The bottom tab set, with `active` on the requested panel.
fn bottom_tabs(active: &str) -> DockNode {
    let index = BOTTOM_PANELS.iter().position(|p| *p == active).unwrap_or(0);
    DockNode::Tabs {
        panels: BOTTOM_PANELS.iter().map(|p| (*p).to_string()).collect(),
        active: index,
    }
}

fn is_empty(node: &DockNode) -> bool {
    match node {
        DockNode::Tabs { panels, .. } => panels.is_empty(),
        DockNode::Split { first, second, .. } => is_empty(first) && is_empty(second),
    }
}

/// Drop later copies of a panel that already appeared earlier.
///
/// Only reachable from a deserialised file. Two tiles both claiming to hold
/// Details is not a layout with a wrong-looking panel in it, it is a layout
/// whose resolve produces two rectangles for one widget.
fn dedupe(node: &mut DockNode, seen: &mut Vec<PanelId>) {
    match node {
        DockNode::Tabs { panels, .. } => {
            panels.retain(|p| {
                if seen.contains(p) {
                    false
                } else {
                    seen.push(p.clone());
                    true
                }
            });
        }
        DockNode::Split { first, second, .. } => {
            dedupe(first, seen);
            dedupe(second, seen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1080.0,
    };

    fn tree() -> DockTree {
        DockTree::default_layout()
    }

    // ── The default arrangement is the shipped one ─────────────────────────

    #[test]
    fn the_default_layout_holds_every_shipped_panel_once() {
        let panels = tree().panels();
        for expected in [
            "Tools",
            "Viewport",
            "Outliner",
            "Details",
            "Content Drawer",
            "Output Log",
            "Jobs",
        ] {
            assert_eq!(
                panels.iter().filter(|p| *p == expected).count(),
                1,
                "{expected} should appear exactly once in {panels:?}"
            );
        }
    }

    #[test]
    fn the_default_layout_reproduces_the_shipped_proportions() {
        // The numbers this asserts are `layout_persist`'s defaults: a 168 px
        // tools strip, a 340 px right column, a 300 px outliner. Within a pixel,
        // because the ratios are stored as fractions of 1920x1080.
        let layout = tree().resolve(WINDOW);
        let tools = layout.rect_of("Tools").unwrap();
        let details = layout.rect_of("Details").unwrap();
        let outliner = layout.rect_of("Outliner").unwrap();
        let viewport = layout.rect_of("Viewport").unwrap();

        assert!((tools.w - 168.0).abs() < 1.0, "tools {}", tools.w);
        assert!((details.w - 340.0).abs() < 1.0, "details {}", details.w);
        assert!((outliner.w - 340.0).abs() < 1.0, "outliner {}", outliner.w);
        assert!((outliner.h - 300.0).abs() < 1.0, "outliner {}", outliner.h);
        assert!(
            viewport.w > tools.w + details.w,
            "the viewport must still be the largest region: {} vs {} + {}",
            viewport.w,
            tools.w,
            details.w
        );
    }

    #[test]
    fn the_bottom_panel_shares_a_tile_and_only_one_is_active() {
        let layout = tree().resolve(WINDOW);
        let drawer = layout
            .panels
            .iter()
            .find(|p| p.panel == "Content Drawer")
            .unwrap();
        assert_eq!(
            drawer.siblings,
            vec!["Content Drawer", "Output Log", "Jobs"]
        );
        assert!(drawer.active);
        for name in ["Output Log", "Jobs"] {
            let tab = layout.panels.iter().find(|p| p.panel == name).unwrap();
            assert_eq!(tab.rect, drawer.rect, "tabs share one tile");
            assert!(!tab.active, "{name} should not also be active");
        }
    }

    // ── Resolution ─────────────────────────────────────────────────────────

    #[test]
    fn tiles_tile_the_area_without_overlapping() {
        let layout = tree().resolve(WINDOW);
        // Every distinct tile, plus every splitter, must sum to the window with
        // nothing left over and nothing counted twice.
        let mut tiles: Vec<Rect> = Vec::new();
        for panel in &layout.panels {
            if !tiles.contains(&panel.rect) {
                tiles.push(panel.rect);
            }
        }
        let area: f32 = tiles.iter().map(|r| r.w * r.h).sum::<f32>()
            + layout
                .splitters
                .iter()
                .map(|s| s.rect.w * s.rect.h)
                .sum::<f32>();
        let expected = WINDOW.w * WINDOW.h;
        assert!(
            (area - expected).abs() / expected < 0.001,
            "tiles cover {area} of {expected}"
        );

        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                let overlap_w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
                let overlap_h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
                assert!(
                    overlap_w <= 0.01 || overlap_h <= 0.01,
                    "{a:?} overlaps {b:?}"
                );
            }
        }
    }

    #[test]
    fn a_window_too_small_for_the_minimums_still_produces_usable_tiles() {
        // 200 px cannot give four columns 80 px each. The rule is that nothing
        // reaches zero or goes negative, because a zero-width tile is a panel
        // the user cannot get back.
        for (w, h) in [(200.0, 200.0), (64.0, 64.0), (1.0, 1.0)] {
            let layout = tree().resolve(Rect { w, h, ..Rect::ZERO });
            for panel in &layout.panels {
                assert!(
                    panel.rect.w >= 0.0 && panel.rect.h >= 0.0,
                    "{}x{}: {} got {:?}",
                    w,
                    h,
                    panel.panel,
                    panel.rect
                );
            }
        }
    }

    #[test]
    fn a_splitter_names_the_tile_a_drag_measures_against() {
        let layout = tree().resolve(WINDOW);
        let root = layout
            .splitters
            .iter()
            .find(|s| s.path.is_empty())
            .expect("the root split has an empty path");
        assert_eq!(root.axis, Axis::Horizontal);
        assert_eq!(root.tile, WINDOW);
        assert!((root.rect.w - SPLITTER).abs() < f32::EPSILON);
    }

    // ── Mutation ───────────────────────────────────────────────────────────

    #[test]
    fn docking_beside_a_panel_splits_its_tile() {
        let mut t = tree();
        assert!(t.dock("Profiler", "Viewport", DockSide::Below));
        let layout = t.resolve(WINDOW);
        let viewport = layout.rect_of("Viewport").unwrap();
        let profiler = layout.rect_of("Profiler").unwrap();
        assert!(
            profiler.y > viewport.y,
            "Below should put it under: {profiler:?} vs {viewport:?}"
        );
        assert!((profiler.x - viewport.x).abs() < 0.01, "same column");
    }

    #[test]
    fn docking_as_a_tab_joins_the_group_and_takes_focus() {
        let mut t = tree();
        assert!(t.dock("Profiler", "Output Log", DockSide::Tab));
        let layout = t.resolve(WINDOW);
        let profiler = layout
            .panels
            .iter()
            .find(|p| p.panel == "Profiler")
            .unwrap();
        assert!(profiler.active, "a docked tab comes to the front");
        assert!(profiler.siblings.contains(&"Output Log".to_string()));
        assert_eq!(
            layout.rect_of("Profiler"),
            layout.rect_of("Output Log"),
            "tabs share a tile"
        );
    }

    #[test]
    fn moving_a_panel_leaves_no_trace_of_where_it_was() {
        // Details is alone in its tile, so moving it must collapse that tile and
        // promote the Outliner — not leave an empty column behind.
        let mut t = tree();
        let before = t.panels().len();
        assert!(t.dock("Details", "Tools", DockSide::Below));
        assert_eq!(t.panels().len(), before, "no panel gained or lost");

        let layout = t.resolve(WINDOW);
        let outliner = layout.rect_of("Outliner").unwrap();
        let details = layout.rect_of("Details").unwrap();
        assert!(
            details.x < outliner.x,
            "Details moved to the tools column: {details:?}"
        );
        // The old right column was Outliner over Details; with Details gone the
        // Outliner should have taken the whole column height.
        assert!(
            outliner.h > 600.0,
            "the Outliner should have inherited the column: {outliner:?}"
        );
    }

    #[test]
    fn docking_a_panel_onto_itself_changes_nothing() {
        let mut t = tree();
        let before = t.clone();
        assert!(!t.dock("Details", "Details", DockSide::Left));
        assert_eq!(t, before);
    }

    #[test]
    fn docking_against_an_absent_target_changes_nothing() {
        let mut t = tree();
        let before = t.clone();
        assert!(!t.dock("Profiler", "Nonexistent", DockSide::Left));
        assert_eq!(t, before);
    }

    #[test]
    fn closing_a_tab_keeps_the_same_panel_in_front() {
        let mut t = tree();
        assert!(t.activate("Jobs"));
        // Close a tab to the *left* of the active one. "Jobs" must still show.
        assert!(t.close("Output Log"));
        let layout = t.resolve(WINDOW);
        let jobs = layout.panels.iter().find(|p| p.panel == "Jobs").unwrap();
        assert!(jobs.active, "closing another tab must not steal focus");
    }

    #[test]
    fn closing_the_active_last_tab_falls_back_to_its_neighbour() {
        let mut t = tree();
        assert!(t.activate("Jobs"));
        assert!(t.close("Jobs"));
        let layout = t.resolve(WINDOW);
        let shown = layout
            .panels
            .iter()
            .filter(|p| p.active && p.siblings.len() > 1)
            .map(|p| p.panel.clone())
            .collect::<Vec<_>>();
        assert_eq!(shown, vec!["Output Log".to_string()]);
    }

    #[test]
    fn closing_the_last_panel_is_refused() {
        let mut t = DockTree::single("Viewport");
        assert!(
            !t.close("Viewport"),
            "an empty tree has nowhere to dock into"
        );
        assert_eq!(t.panels(), vec!["Viewport".to_string()]);
    }

    #[test]
    fn a_splitter_drag_is_clamped_and_survives_a_resolve() {
        let mut t = tree();
        assert!(t.set_ratio(&[], 0.4));
        let layout = t.resolve(WINDOW);
        let tools = layout.rect_of("Tools").unwrap();
        assert!(
            (tools.w - (1920.0 - SPLITTER) * 0.4).abs() < 1.0,
            "{tools:?}"
        );

        // Out-of-range drags are clamped, not rejected: a drag that leaves the
        // window still has to leave the layout usable.
        assert!(t.set_ratio(&[], 5.0));
        assert!(t.set_ratio(&[], -5.0));
        let layout = t.resolve(WINDOW);
        assert!(layout.rect_of("Tools").unwrap().w >= MIN_TILE);
        assert!(layout.rect_of("Viewport").unwrap().w >= MIN_TILE);
    }

    #[test]
    fn a_ratio_path_that_names_a_tab_set_is_refused() {
        let mut t = DockTree::single("Viewport");
        assert!(!t.set_ratio(&[], 0.5));
    }

    // ── Persistence ────────────────────────────────────────────────────────

    #[test]
    fn a_layout_round_trips_through_json() {
        let mut t = tree();
        t.dock("Profiler", "Viewport", DockSide::Right);
        t.set_ratio(&[], 0.3);
        let json = serde_json::to_string(&t).unwrap();
        let back: DockTree = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn repair_fixes_what_a_hand_edited_file_can_do_to_a_tree() {
        // Three things a file can contain that no operation can produce: an
        // empty tab set, a duplicated panel, and an out-of-range ratio and
        // active index.
        let broken = r#"{
            "root": {
                "Split": {
                    "axis": "Horizontal",
                    "ratio": 40.0,
                    "first": { "Tabs": { "panels": [], "active": 3 } },
                    "second": {
                        "Split": {
                            "axis": "Vertical",
                            "ratio": -1.0,
                            "first": { "Tabs": { "panels": ["Viewport", "Details"], "active": 9 } },
                            "second": { "Tabs": { "panels": ["Details"], "active": 0 } }
                        }
                    }
                }
            }
        }"#;
        let mut t: DockTree = serde_json::from_str(broken).unwrap();
        t.repair();

        assert_eq!(
            t.panels(),
            vec!["Viewport".to_string(), "Details".to_string()]
        );
        let layout = t.resolve(WINDOW);
        assert_eq!(layout.panels.len(), 2, "no panel resolved twice");
        // The empty tile and the tile emptied by dedupe both collapsed, so what
        // is left is a single tab set and no splitters at all.
        assert!(layout.splitters.is_empty(), "{:?}", layout.splitters);
    }

    #[test]
    fn an_empty_file_falls_back_to_the_shipped_layout() {
        let mut t: DockTree =
            serde_json::from_str(r#"{ "root": { "Tabs": { "panels": [], "active": 0 } } }"#)
                .unwrap();
        t.repair();
        assert_eq!(t, DockTree::default_layout());
    }
}
