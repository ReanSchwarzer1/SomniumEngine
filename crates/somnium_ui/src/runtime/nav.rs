//! Directional navigation and the input-source model (MORROWIND-F).
//!
//! # What already existed, and what this adds
//!
//! Phase CONTROL-A1 shipped focus state, a modal focus trap, focus-into-view
//! and **linear** traversal — up/down/Home/End over a flat list of focus stops.
//! That is the right model for a Details panel and the wrong one for a menu: a
//! grid of buttons has no meaningful linear order, and Tab through a 4x3
//! inventory visits the cells in an order nobody predicted.
//!
//! This module adds **two-dimensional** navigation, and does not replace the
//! linear traversal it sits beside.
//!
//! # Godot's model and Unity's, together, because each fails alone
//!
//! §8 item 3 is explicit about the design and about why:
//!
//! > *"explicit neighbour links where authored, geometric search where not —
//! > Godot's model and Unity's together, because each fails alone (explicit
//! > links are unmaintainable at scale; geometric search picks the wrong widget
//! > in dense layouts)."*
//!
//! Both halves of that are real:
//!
//! - **Explicit links alone** mean every button in a settings screen names four
//!   neighbours, and inserting a row means editing eight of them. Nobody keeps
//!   that correct, and the failure is silent: navigation still *works*, it just
//!   goes somewhere surprising.
//! - **Geometry alone** is right almost always and wrong exactly where it
//!   matters — a dense toolbar where the nearest thing to the right is
//!   diagonally up, or a list beside a preview pane where "right" should leave
//!   the list rather than move down it.
//!
//! So: an authored link wins if there is one; otherwise geometry decides, with
//! a scoring function that prefers alignment over proximity. That preference is
//! the whole difference between "picks the nearest widget" and "picks the
//! widget a person meant".

use crate::{message::NodeHandle, types::Rect};
use glam::Vec2;
use std::collections::HashMap;

/// A navigation direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// The unit vector, in the UI's y-down space.
    #[must_use]
    pub fn vector(self) -> Vec2 {
        match self {
            Self::Left => Vec2::new(-1.0, 0.0),
            Self::Right => Vec2::new(1.0, 0.0),
            Self::Up => Vec2::new(0.0, -1.0),
            Self::Down => Vec2::new(0.0, 1.0),
        }
    }

    /// Whether this direction runs along x.
    #[must_use]
    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// The opposite direction.
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Up => Self::Down,
            Self::Down => Self::Up,
        }
    }
}

/// Where input came from.
///
/// # Hover has no meaning on a gamepad, and the API says so
///
/// §8 item 4: *"pointer, touch and gamepad as one event stream; hover has no
/// meaning on a pad and the API must say so rather than pretending."*
///
/// The pretence is the common bug: a UI that treats the focused widget as
/// hovered shows a hover highlight *and* a focus ring on the same control, so a
/// pad user sees two cues meaning one thing while a mouse user sees them mean
/// two different things. [`InputSource::has_hover`] is the one call that stops
/// it, and Zeta's four-cue state grammar is why it matters: hover and focus are
/// two of the four, and collapsing them leaves the grammar with three.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputSource {
    /// A mouse or trackpad. Has a position between clicks, and therefore hover.
    #[default]
    Pointer,
    /// A touchscreen. Has a position only while touching, so a "hover" state
    /// would be a state the user cannot see themselves enter.
    Touch,
    /// A gamepad or keyboard. Has no position at all; focus is the only cue.
    Gamepad,
}

impl InputSource {
    /// Whether a hover state is meaningful for this source.
    #[must_use]
    pub fn has_hover(self) -> bool {
        matches!(self, Self::Pointer)
    }

    /// Whether this source drives focus by navigation rather than by pointing.
    #[must_use]
    pub fn navigates(self) -> bool {
        matches!(self, Self::Gamepad)
    }
}

/// A navigation verb, independent of what pressed it.
///
/// # The seam MORROWIND-AE plugs into
///
/// §8 item 5 makes this sub-phase's last item a **forward dependency**:
///
/// > *"Consumes MORROWIND-AE's action map for navigation verbs, so a player's
/// > rebound 'confirm' works in menus. This is a forward dependency and Track 8
/// > must land AE before F closes."*
///
/// So F defines the verbs and the routing; AE supplies the bindings. Until AE
/// lands, [`NavAction::from_key`] is a hard-coded keyboard default — which is
/// deliberately a *free function taking a key*, not a match buried in the
/// widget tree, so AE replaces one call site rather than hunting for keycodes.
/// Seam 5's whole point is that keycodes appear in exactly one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction {
    /// Move focus.
    Move(Direction),
    /// Activate the focused widget.
    Confirm,
    /// Dismiss, or step out of the current focus scope.
    Cancel,
    /// Next / previous focus stop in the linear order CONTROL-A1 already
    /// traverses. Kept distinct from `Move`, because Tab is not Down: Tab
    /// visits every stop in order and Down visits the thing below.
    Next,
    Previous,
}

impl NavAction {
    /// The default keyboard binding.
    ///
    /// **Temporary by design.** MORROWIND-AE replaces this with a lookup
    /// through the player's action map; the signature stays, so the widget tree
    /// never learns what a keycode is.
    #[must_use]
    pub fn from_key(key: winit::keyboard::KeyCode, shift: bool) -> Option<Self> {
        use winit::keyboard::KeyCode;
        Some(match key {
            KeyCode::ArrowLeft => Self::Move(Direction::Left),
            KeyCode::ArrowRight => Self::Move(Direction::Right),
            KeyCode::ArrowUp => Self::Move(Direction::Up),
            KeyCode::ArrowDown => Self::Move(Direction::Down),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => Self::Confirm,
            KeyCode::Escape => Self::Cancel,
            KeyCode::Tab if shift => Self::Previous,
            KeyCode::Tab => Self::Next,
            _ => return None,
        })
    }
}

/// Explicit neighbour links, authored where geometry gets it wrong.
#[derive(Clone, Debug, Default)]
pub struct NavLinks {
    links: HashMap<(NodeHandle, Direction), NodeHandle>,
}

impl NavLinks {
    /// Author `from -> to` in `direction`.
    ///
    /// One-way on purpose. The symmetric case is common enough to deserve
    /// [`Self::link_both`], but a one-way link is exactly what a "back to the
    /// list" edge needs, and inferring the reverse would quietly overwrite an
    /// author's other choice.
    pub fn link(&mut self, from: NodeHandle, direction: Direction, to: NodeHandle) {
        self.links.insert((from, direction), to);
    }

    /// Author `from <-> to`, in `direction` and back.
    pub fn link_both(&mut self, from: NodeHandle, direction: Direction, to: NodeHandle) {
        self.link(from, direction, to);
        self.link(to, direction.opposite(), from);
    }

    /// The authored neighbour, if any.
    #[must_use]
    pub fn get(&self, from: NodeHandle, direction: Direction) -> Option<NodeHandle> {
        self.links.get(&(from, direction)).copied()
    }

    /// Forget every link from `handle`, in both roles.
    ///
    /// Called when a widget is removed. Without it a link points at a freed
    /// handle and navigation lands on whatever was allocated in its place —
    /// which is the generational-pool version of a use-after-free, and shows up
    /// as focus jumping somewhere impossible.
    pub fn forget(&mut self, handle: NodeHandle) {
        self.links
            .retain(|(from, _), to| *from != handle && *to != handle);
    }

    /// How many links are authored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.links.len()
    }

    /// Whether nothing is authored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }
}

/// One navigable widget: its handle and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavCandidate {
    pub handle: NodeHandle,
    pub rect: Rect,
}

/// How far off-axis a candidate may sit before it stops counting.
///
/// A candidate whose perpendicular extent does not overlap the source's at all
/// is still reachable, but it loses to anything that does. Beyond this many
/// times the source's own extent it is not considered a neighbour in that
/// direction at all — which is what stops "right" from finding a widget at the
/// far corner of the screen when there is genuinely nothing to the right.
const MAX_OFF_AXIS_RATIO: f32 = 8.0;

/// Pick the neighbour of `from` in `direction`.
///
/// Authored links win. Otherwise the candidates are scored and the best wins,
/// or `None` when nothing lies that way.
#[must_use]
pub fn navigate(
    from: NavCandidate,
    candidates: &[NavCandidate],
    direction: Direction,
    links: &NavLinks,
) -> Option<NodeHandle> {
    if let Some(explicit) = links.get(from.handle, direction) {
        // An authored link is honoured even if the target is not in the
        // candidate list — the author knows something the geometry does not,
        // and second-guessing it makes the feature useless where it is needed.
        return Some(explicit);
    }

    candidates
        .iter()
        .filter(|c| c.handle != from.handle)
        .filter_map(|c| score(from.rect, c.rect, direction).map(|s| (s, c.handle)))
        // `total_cmp` rather than `partial_cmp().unwrap()`: a NaN score from a
        // degenerate rect would panic, and a widget with a zero size is a real
        // thing that happens during a layout transition.
        .min_by(|(a, _), (b, _)| a.total_cmp(b))
        .map(|(_, handle)| handle)
}

/// Score a candidate. Lower is better; `None` means "not in that direction".
///
/// The shape of this function is the whole design:
///
/// 1. **Reject anything not actually in the direction.** Measured from the
///    *near edges*, not the centres, so a tall list item beside a short button
///    does not count as being above it.
/// 2. **Weight off-axis distance far more heavily than on-axis.** This is what
///    makes navigation feel deliberate: pressing Right in a toolbar goes to the
///    next button along, not to the slightly-nearer thing diagonally above.
///    Nearest-centre scoring gets that case wrong, which is §8's "geometric
///    search picks the wrong widget in dense layouts".
/// 3. **Reward overlap.** Two widgets that share a row are neighbours in a way
///    two that merely happen to be close are not.
fn score(from: Rect, to: Rect, direction: Direction) -> Option<f32> {
    let (near, off_axis_overlap, off_axis_gap) = match direction {
        Direction::Right => (
            to.x - (from.x + from.w),
            overlap(from.y, from.y + from.h, to.y, to.y + to.h),
            gap(from.y, from.y + from.h, to.y, to.y + to.h),
        ),
        Direction::Left => (
            from.x - (to.x + to.w),
            overlap(from.y, from.y + from.h, to.y, to.y + to.h),
            gap(from.y, from.y + from.h, to.y, to.y + to.h),
        ),
        Direction::Down => (
            to.y - (from.y + from.h),
            overlap(from.x, from.x + from.w, to.x, to.x + to.w),
            gap(from.x, from.x + from.w, to.x, to.x + to.w),
        ),
        Direction::Up => (
            from.y - (to.y + to.h),
            overlap(from.x, from.x + from.w, to.x, to.x + to.w),
            gap(from.x, from.x + from.w, to.x, to.x + to.w),
        ),
    };

    // Strictly past the source's near edge. A small negative tolerance lets
    // adjacent widgets that share an edge — a toolbar with no gaps — still be
    // neighbours, without letting an overlapping widget qualify.
    if near < -1.0 {
        return None;
    }
    let near = near.max(0.0);

    let source_extent = if direction.is_horizontal() {
        from.h.max(1.0)
    } else {
        from.w.max(1.0)
    };
    if off_axis_gap > source_extent * MAX_OFF_AXIS_RATIO {
        return None;
    }

    // Off-axis distance dominates. The factor is large enough that a perfectly
    // aligned candidate ten times further away still wins over a misaligned
    // one, which is the behaviour a person expects from a d-pad.
    const OFF_AXIS_WEIGHT: f32 = 10.0;
    // Overlap is a bonus rather than a gate: two widgets in the same row should
    // beat two that merely nearly are, but a grid with gutters has no overlap
    // at all in one axis and must still navigate.
    let overlap_bonus = if off_axis_overlap > 0.0 { 0.0 } else { 1.0 };

    Some(near + off_axis_gap * OFF_AXIS_WEIGHT + overlap_bonus * source_extent)
}

/// Length of the overlap between two 1-D spans, or 0.
fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// Distance between two 1-D spans, or 0 when they overlap.
fn gap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    if a1 < b0 {
        b0 - a1
    } else if b1 < a0 {
        a0 - b1
    } else {
        0.0
    }
}

/// The first widget to focus when navigation starts with nothing focused.
///
/// Top-left-most, which matches reading order in a left-to-right layout and is
/// what every menu wants on open. A right-to-left locale wants the mirror; that
/// is MORROWIND-G's bidi work and this function is where it will hook in.
#[must_use]
pub fn first_focus(candidates: &[NavCandidate]) -> Option<NodeHandle> {
    candidates
        .iter()
        .min_by(|a, b| {
            (a.rect.y, a.rect.x)
                .partial_cmp(&(b.rect.y, b.rect.x))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|c| c.handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(index: u32) -> NodeHandle {
        NodeHandle::new(index, 1)
    }

    fn candidate(index: u32, x: f32, y: f32, w: f32, h: f32) -> NavCandidate {
        NavCandidate {
            handle: handle(index),
            rect: Rect::new(x, y, w, h),
        }
    }

    /// A row of buttons navigates along the row.
    #[test]
    fn a_toolbar_navigates_along_itself() {
        let a = candidate(1, 0.0, 0.0, 40.0, 24.0);
        let b = candidate(2, 44.0, 0.0, 40.0, 24.0);
        let c = candidate(3, 88.0, 0.0, 40.0, 24.0);
        let all = [a, b, c];
        let links = NavLinks::default();

        assert_eq!(navigate(a, &all, Direction::Right, &links), Some(b.handle));
        assert_eq!(navigate(b, &all, Direction::Right, &links), Some(c.handle));
        assert_eq!(navigate(c, &all, Direction::Right, &links), None);
        assert_eq!(navigate(c, &all, Direction::Left, &links), Some(b.handle));
    }

    /// **The case nearest-centre scoring gets wrong.**
    ///
    /// Pressing Right in a toolbar must go to the next button along, even when
    /// something diagonally above is closer by straight-line distance. This is
    /// §8's "geometric search picks the wrong widget in dense layouts", and it
    /// is the reason off-axis distance is weighted an order of magnitude more
    /// heavily than on-axis.
    #[test]
    fn alignment_beats_raw_proximity() {
        let from = candidate(1, 0.0, 100.0, 40.0, 24.0);
        // Directly right, but further away.
        let aligned = candidate(2, 200.0, 100.0, 40.0, 24.0);
        // Nearer as the crow flies, but well off the row.
        let diagonal = candidate(3, 50.0, 20.0, 40.0, 24.0);
        let all = [from, aligned, diagonal];

        assert_eq!(
            navigate(from, &all, Direction::Right, &NavLinks::default()),
            Some(aligned.handle),
            "a d-pad Right should follow the row, not the shortest line"
        );
    }

    /// A grid with gutters has no overlap in one axis and must still navigate.
    #[test]
    fn a_grid_navigates_in_both_axes() {
        let mut all = Vec::new();
        for row in 0..3 {
            for col in 0..4 {
                all.push(candidate(
                    (row * 4 + col) as u32 + 1,
                    col as f32 * 60.0,
                    row as f32 * 40.0,
                    50.0,
                    30.0,
                ));
            }
        }
        let links = NavLinks::default();
        let top_left = all[0];
        assert_eq!(navigate(top_left, &all, Direction::Right, &links), Some(all[1].handle));
        assert_eq!(navigate(top_left, &all, Direction::Down, &links), Some(all[4].handle));
        assert_eq!(navigate(top_left, &all, Direction::Left, &links), None);
        assert_eq!(navigate(top_left, &all, Direction::Up, &links), None);

        let middle = all[5];
        assert_eq!(navigate(middle, &all, Direction::Up, &links), Some(all[1].handle));
        assert_eq!(navigate(middle, &all, Direction::Left, &links), Some(all[4].handle));
    }

    /// An authored link wins over geometry.
    #[test]
    fn an_explicit_link_overrides_the_geometry() {
        let list = candidate(1, 0.0, 0.0, 100.0, 300.0);
        let below = candidate(2, 0.0, 310.0, 100.0, 40.0);
        let preview = candidate(3, 120.0, 0.0, 200.0, 300.0);
        let all = [list, below, preview];
        let mut links = NavLinks::default();

        // Geometry already agrees here; the point is that the author can force it.
        assert_eq!(
            navigate(list, &all, Direction::Right, &links),
            Some(preview.handle)
        );
        links.link(list.handle, Direction::Right, below.handle);
        assert_eq!(
            navigate(list, &all, Direction::Right, &links),
            Some(below.handle),
            "the author knows something the geometry does not"
        );
    }

    /// A link to something outside the candidate list is still honoured.
    ///
    /// Second-guessing it makes the feature useless exactly where it is needed:
    /// a link into a panel that is collapsed, or onto a widget the caller did
    /// not bother to enumerate.
    #[test]
    fn an_explicit_link_to_an_unlisted_widget_is_honoured() {
        let from = candidate(1, 0.0, 0.0, 40.0, 24.0);
        let mut links = NavLinks::default();
        links.link(from.handle, Direction::Down, handle(99));
        assert_eq!(
            navigate(from, &[from], Direction::Down, &links),
            Some(handle(99))
        );
    }

    #[test]
    fn linking_both_ways_creates_the_return_edge() {
        let mut links = NavLinks::default();
        links.link_both(handle(1), Direction::Right, handle(2));
        assert_eq!(links.get(handle(1), Direction::Right), Some(handle(2)));
        assert_eq!(links.get(handle(2), Direction::Left), Some(handle(1)));
    }

    /// A one-way link does not invent its reverse.
    #[test]
    fn a_one_way_link_stays_one_way() {
        let mut links = NavLinks::default();
        links.link(handle(1), Direction::Right, handle(2));
        assert_eq!(links.get(handle(2), Direction::Left), None);
    }

    /// Removing a widget forgets its links, in both roles.
    ///
    /// A stale link points at a freed handle, and in a generational pool that
    /// means focus lands on whatever was allocated in its place.
    #[test]
    fn forgetting_a_widget_removes_links_pointing_at_it_too() {
        let mut links = NavLinks::default();
        links.link_both(handle(1), Direction::Right, handle(2));
        links.link_both(handle(2), Direction::Down, handle(3));
        links.forget(handle(2));
        assert_eq!(links.get(handle(1), Direction::Right), None);
        assert_eq!(links.get(handle(3), Direction::Up), None);
        assert!(links.is_empty());
    }

    /// Nothing in that direction returns nothing, rather than the far corner.
    #[test]
    fn a_distant_off_axis_widget_is_not_a_neighbour() {
        let from = candidate(1, 0.0, 0.0, 40.0, 24.0);
        let far = candidate(2, 100.0, 900.0, 40.0, 24.0);
        assert_eq!(
            navigate(from, &[from, far], Direction::Right, &NavLinks::default()),
            None,
            "there is genuinely nothing to the right"
        );
    }

    /// Adjacent widgets sharing an edge are neighbours.
    #[test]
    fn widgets_that_touch_are_neighbours() {
        let a = candidate(1, 0.0, 0.0, 40.0, 24.0);
        let b = candidate(2, 40.0, 0.0, 40.0, 24.0);
        assert_eq!(
            navigate(a, &[a, b], Direction::Right, &NavLinks::default()),
            Some(b.handle)
        );
    }

    /// A zero-sized widget scores without panicking.
    ///
    /// `partial_cmp().unwrap()` on a NaN score would panic here, and a widget
    /// with zero size is a real thing that happens mid-layout.
    #[test]
    fn a_degenerate_rect_does_not_panic() {
        let from = candidate(1, 0.0, 0.0, 0.0, 0.0);
        let other = candidate(2, 10.0, 0.0, 0.0, 0.0);
        let _ = navigate(from, &[from, other], Direction::Right, &NavLinks::default());
    }

    #[test]
    fn first_focus_is_the_top_left_most() {
        let all = [
            candidate(1, 100.0, 50.0, 10.0, 10.0),
            candidate(2, 10.0, 10.0, 10.0, 10.0),
            candidate(3, 60.0, 10.0, 10.0, 10.0),
        ];
        assert_eq!(first_focus(&all), Some(handle(2)));
        assert_eq!(first_focus(&[]), None);
    }

    /// **Hover is a pointer concept, and the API says so.**
    ///
    /// The pretence this prevents: treating the focused widget as hovered shows
    /// a hover highlight *and* a focus ring on one control, so a pad user sees
    /// two cues meaning one thing while a mouse user sees them mean two.
    #[test]
    fn only_a_pointer_hovers() {
        assert!(InputSource::Pointer.has_hover());
        assert!(!InputSource::Touch.has_hover());
        assert!(!InputSource::Gamepad.has_hover());
        assert!(InputSource::Gamepad.navigates());
        assert!(!InputSource::Pointer.navigates());
    }

    #[test]
    fn directions_are_their_own_opposites_twice_over() {
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            assert_eq!(direction.opposite().opposite(), direction);
            assert_eq!(direction.vector(), -direction.opposite().vector());
        }
    }

    /// Tab is not Down, and the distinction is not pedantic.
    ///
    /// Tab visits every focus stop in order; Down visits the thing below. A
    /// settings screen with two columns has one Tab order and two Down chains,
    /// and collapsing them makes one of the two wrong.
    #[test]
    fn tab_and_arrows_are_different_verbs() {
        use winit::keyboard::KeyCode;
        assert_eq!(NavAction::from_key(KeyCode::Tab, false), Some(NavAction::Next));
        assert_eq!(
            NavAction::from_key(KeyCode::Tab, true),
            Some(NavAction::Previous)
        );
        assert_eq!(
            NavAction::from_key(KeyCode::ArrowDown, false),
            Some(NavAction::Move(Direction::Down))
        );
        assert_ne!(
            NavAction::from_key(KeyCode::Tab, false),
            NavAction::from_key(KeyCode::ArrowDown, false)
        );
    }

    #[test]
    fn confirm_and_cancel_have_the_conventional_keys() {
        use winit::keyboard::KeyCode;
        for key in [KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space] {
            assert_eq!(NavAction::from_key(key, false), Some(NavAction::Confirm));
        }
        assert_eq!(
            NavAction::from_key(KeyCode::Escape, false),
            Some(NavAction::Cancel)
        );
        assert_eq!(NavAction::from_key(KeyCode::KeyQ, false), None);
    }
}
