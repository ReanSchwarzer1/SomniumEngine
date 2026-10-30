//! Command palette (Phase 26-I). Ctrl+P / Ctrl+Shift+P.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum CommandPaletteMessage {
    /// Run the row identified by its stable id, never its array position.
    Run(String),
    Query(String),
    SetQuery(String),
    /// Replace the searchable set. Phase 27-G: the palette is repopulated each
    /// time it opens, because entities and assets change while it is closed.
    SetItems(Vec<PaletteItem>),
}

/// What a palette row refers to.
///
/// Phase 27-G. The category is carried on the item rather than inferred from
/// the label, because two different things can share a name — an entity called
/// "Terrain" and the Terrain help page both exist — and a search surface that
/// cannot tell them apart is worse than one that only searches commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteCategory {
    Command,
    Entity,
    Asset,
    Panel,
    Help,
}

impl PaletteCategory {
    /// The one-character prefix that scopes a query to this category.
    pub const fn prefix(self) -> char {
        match self {
            PaletteCategory::Command => '>',
            PaletteCategory::Entity => '@',
            PaletteCategory::Asset => '#',
            PaletteCategory::Panel => ':',
            PaletteCategory::Help => '?',
        }
    }

    /// Short label shown on the row, so a result says what kind of thing it is
    /// without the user having to recognise the icon.
    pub const fn label(self) -> &'static str {
        match self {
            PaletteCategory::Command => "Command",
            PaletteCategory::Entity => "Entity",
            PaletteCategory::Asset => "Asset",
            PaletteCategory::Panel => "Panel",
            PaletteCategory::Help => "Help",
        }
    }

    pub const fn all() -> [PaletteCategory; 5] {
        [
            PaletteCategory::Command,
            PaletteCategory::Entity,
            PaletteCategory::Asset,
            PaletteCategory::Panel,
            PaletteCategory::Help,
        ]
    }

    fn from_prefix(c: char) -> Option<Self> {
        Self::all().into_iter().find(|k| k.prefix() == c)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaletteItem {
    /// Stable dispatch/history key.
    pub id: String,
    pub label: String,
    pub hint: String,
    pub category: PaletteCategory,
    /// Disabled rows remain discoverable and explain why they cannot run.
    pub enabled: bool,
    /// Persisted last-used tick; larger values rank first after fuzzy score.
    pub recency: u64,
    /// The row's glyph (Phase ALYX-B); none falls back to its category's.
    pub icon: Option<crate::icons::IconId>,
}

impl PaletteItem {
    /// A command row derived from the authoritative registry.
    pub fn command(
        id: impl Into<String>,
        label: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: hint.into(),
            category: PaletteCategory::Command,
            enabled: true,
            recency: 0,
            icon: None,
        }
    }

    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        hint: impl Into<String>,
        category: PaletteCategory,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: hint.into(),
            category,
            enabled: true,
            recency: 0,
            icon: None,
        }
    }

    /// Attach current enablement and its explanation.
    #[must_use]
    pub fn with_enablement(mut self, enabled: bool, reason: Option<&str>) -> Self {
        self.enabled = enabled;
        if let Some(reason) = reason {
            self.hint = reason.to_string();
        }
        self
    }

    /// Attach the row's glyph.
    #[must_use]
    pub const fn with_icon(mut self, icon: crate::icons::IconId) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The glyph drawn on the row: its own, else its category's.
    pub fn glyph(&self) -> crate::icons::IconId {
        use crate::icons::IconId as I;
        self.icon.unwrap_or(match self.category {
            PaletteCategory::Command => I::More,
            PaletteCategory::Entity => I::EmptyEntity,
            PaletteCategory::Asset => I::File,
            PaletteCategory::Panel => I::AppWindow,
            PaletteCategory::Help => I::HelpCircle,
        })
    }

    /// Attach the persisted usage tick.
    #[must_use]
    pub const fn with_recency(mut self, recency: u64) -> Self {
        self.recency = recency;
        self
    }
}

pub struct CommandPalette {
    pub query: String,
    pub selected: usize,
    pub font_id: u8,
    pub items: Vec<PaletteItem>,
}

impl CommandPalette {
    /// Split a query into an optional category scope and the search terms.
    ///
    /// `@cam` searches entities for "cam"; `@` alone lists every entity, which
    /// is what makes the prefix useful for browsing and not just filtering.
    pub fn parse_query(query: &str) -> (Option<PaletteCategory>, &str) {
        let mut chars = query.chars();
        match chars.next().and_then(PaletteCategory::from_prefix) {
            Some(kind) => (Some(kind), chars.as_str().trim_start()),
            None => (None, query),
        }
    }

    fn filtered(&self) -> Vec<&PaletteItem> {
        let (scope, terms) = Self::parse_query(&self.query);
        let mut out: Vec<(&PaletteItem, i64, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| scope.is_none_or(|kind| item.category == kind))
            .filter_map(|(index, item)| {
                palette_score(item, terms).map(|score| (item, score, index))
            })
            .collect();
        out.sort_by(
            |(left, left_score, left_index), (right, right_score, right_index)| {
                right_score
                    .cmp(left_score)
                    .then_with(|| right.recency.cmp(&left.recency))
                    .then_with(|| left_index.cmp(right_index))
            },
        );
        out.into_iter().map(|(item, _, _)| item).collect()
    }

    /// Rows currently matching, for tests and for the "no matches" state.
    pub fn match_count(&self) -> usize {
        self.filtered().len()
    }
}

/// Phase ALYX-B: one line per result (glyph, label, then what it is and its shortcut flush right), twelve shown.
const PALETTE_WIDTH: f32 = 560.0;
const PALETTE_HEIGHT: f32 = 444.0;
const ROWS_TOP: f32 = 36.0;
const ROW_HEIGHT: f32 = 32.0;
const ROWS_SHOWN: usize = 12;

/// The first result drawn while `selected` is the selection.
const fn first_shown(selected: usize) -> usize {
    selected.saturating_sub(ROWS_SHOWN - 1)
}

impl Control for CommandPalette {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        Vec2::new(PALETTE_WIDTH, PALETTE_HEIGHT)
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        let b = widget.screen_bounds();
        if pos.y < b.y + 28.0 {
            CursorKind::Text
        } else {
            CursorKind::Pointer
        }
    }

    fn is_text_input(&self) -> bool {
        true
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // The palette is the highest ordinary surface in the shell: modal rung,
        // modal radius, and the focus border it already had.
        let t = theme::active();
        ctx.push_drop_shadow_rounded(b, [t.geometry.radius_modal; 4], t.elevation.modal);
        ctx.push_primitive(
            crate::primitive::Primitive::fill(b, t.semantic.surface.popup.bytes())
                .with_radius(t.geometry.radius_modal)
                .with_border(t.geometry.stroke_focus, t.semantic.border.focus.bytes()),
            None,
        );
        let search = Rect::new(b.x + 8.0, b.y + 8.0, b.w - 16.0, 22.0);
        ctx.push_paint(
            search,
            &crate::style::input(crate::style::VisualState::rest()),
        );
        let shown = if self.query.is_empty() {
            "Search commands, entities, assets…"
        } else {
            self.query.as_str()
        };
        let color = if self.query.is_empty() {
            t.semantic.text.disabled.bytes()
        } else {
            t.semantic.text.primary.bytes()
        };
        ctx.push_text(
            shown,
            Vec2::new(search.x + 6.0, search.y + 4.0),
            self.font_id,
            12.0,
            color,
        );

        let filtered = self.filtered();
        if filtered.is_empty() {
            ctx.push_text(
                "No matches",
                Vec2::new(b.x + 16.0, b.y + 44.0),
                self.font_id,
                12.0,
                t.semantic.text.muted.bytes(),
            );
            return;
        }
        let sel = self.selected.min(filtered.len().saturating_sub(1));
        // The list scrolls with the selection, so arrowing past the last row shown keeps it in view.
        let first = first_shown(sel);
        for (index, item) in filtered.iter().enumerate().skip(first).take(ROWS_SHOWN) {
            let row = index - first;
            let y = b.y + ROWS_TOP + row as f32 * ROW_HEIGHT;
            let row_r = Rect::new(b.x + 8.0, y, b.w - 16.0, ROW_HEIGHT);
            if index == sel {
                let selected = crate::style::tree_row(crate::style::VisualState::with(
                    crate::style::Interaction::Selected,
                ));
                ctx.push_paint(row_r, &selected);
            }
            let (label_color, glyph_color) = if item.enabled {
                (t.semantic.text.primary.bytes(), t.semantic.text.secondary.bytes())
            } else {
                (t.semantic.text.disabled.bytes(), t.semantic.text.disabled.bytes())
            };
            let glyph = Rect::new(row_r.x + 10.0, y + (ROW_HEIGHT - 16.0) * 0.5, 16.0, 16.0);
            let (uv, texture) = item.glyph().draw_quad(glyph);
            ctx.push_textured_rect(glyph, uv, glyph_color, texture);
            // Right to left: the shortcut (or why it cannot run) flush right, then what kind of thing the row is.
            let mut right = row_r.x + row_r.w - 10.0;
            if !item.hint.is_empty() {
                let hint_color = if item.enabled {
                    t.semantic.text.muted.bytes()
                } else {
                    t.semantic.text.secondary.bytes()
                };
                let (hint, _) = ctx.font_atlas.ellipsise(&item.hint, row_r.w * 0.4, 11.0, self.font_id);
                let width = ctx.font_atlas.measure_text(&hint, 11.0, self.font_id).x;
                right -= width;
                ctx.push_text(&hint, Vec2::new(right, y + 9.0), self.font_id, 11.0, hint_color);
                right -= 14.0;
            }
            if item.category != PaletteCategory::Command {
                let kind = item.category.label();
                let width = ctx.font_atlas.measure_text(kind, 11.0, self.font_id).x;
                right -= width;
                ctx.push_text(kind, Vec2::new(right, y + 9.0), self.font_id, 11.0, t.semantic.text.muted.bytes());
                right -= 14.0;
            }
            let label_x = row_r.x + 36.0;
            let (label, _) =
                ctx.font_atlas.ellipsise(&item.label, (right - label_x).max(40.0), 12.0, self.font_id);
            ctx.push_text(&label, Vec2::new(label_x, y + 8.0), self.font_id, 12.0, label_color);
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(CommandPaletteMessage::SetItems(items)) = msg.data::<CommandPaletteMessage>() {
            self.items = items.clone();
            self.selected = 0;
            return;
        }
        if let Some(CommandPaletteMessage::SetQuery(q)) = msg.data::<CommandPaletteMessage>() {
            self.query = q.clone();
            self.selected = 0;
            msg.handled = true;
            return;
        }
        let Some(wmsg) = msg.data::<WidgetMessage>() else {
            return;
        };
        match wmsg.clone() {
            WidgetMessage::Text(s) => {
                self.query.push_str(&s);
                self.selected = 0;
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    CommandPaletteMessage::Query(self.query.clone()),
                ));
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::Backspace, _) => {
                self.query.pop();
                self.selected = 0;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::ArrowDown, _) => {
                let n = self.filtered().len().max(1);
                self.selected = (self.selected + 1) % n;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::ArrowUp, _) => {
                let n = self.filtered().len().max(1);
                self.selected = (self.selected + n - 1) % n;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::Enter | KeyCode::NumpadEnter, _) => {
                let filtered = self.filtered();
                if let Some(item) = filtered.get(self.selected).filter(|item| item.enabled) {
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        CommandPaletteMessage::Run(item.id.clone()),
                    ));
                }
                msg.handled = true;
            }
            WidgetMessage::MouseDown { pos, .. } => {
                let b = widget.screen_bounds();
                if pos.y > b.y + ROWS_TOP {
                    let row = ((pos.y - b.y - ROWS_TOP) / ROW_HEIGHT).floor() as usize;
                    let filtered = self.filtered();
                    let shown = first_shown(self.selected.min(filtered.len().saturating_sub(1))) + row;
                    if let Some(item) = filtered.get(shown).filter(|item| row < ROWS_SHOWN && item.enabled) {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            CommandPaletteMessage::Run(item.id.clone()),
                        ));
                    }
                    msg.handled = true;
                }
            }
            _ => {}
        }
    }
}

/// Structured fields are exact vocabulary with exact/prefix values. The
/// remaining text is subsequence-fuzzy with prefix and word-boundary bonuses.
fn palette_score(item: &PaletteItem, query: &str) -> Option<i64> {
    let mut free = Vec::new();
    for token in query.split_whitespace() {
        let Some((field, value)) = token.split_once(':') else {
            free.push(token);
            continue;
        };
        let value = value.to_ascii_lowercase();
        let haystack = match field {
            "category" => item.category.label().to_ascii_lowercase(),
            "id" => item.id.to_ascii_lowercase(),
            _ => return None,
        };
        if !haystack.starts_with(&value) {
            return None;
        }
    }
    let text = format!("{} {} {}", item.label, item.hint, item.id);
    crate::commands::fuzzy_score(&text, &free.join(" "))
}

pub struct CommandPaletteBuilder {
    widget: WidgetBuilder,
    font_id: u8,
    items: Vec<PaletteItem>,
}

impl CommandPaletteBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            font_id: 0,
            items: Vec::new(),
        }
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn with_items(mut self, items: Vec<PaletteItem>) -> Self {
        self.items = items;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(CommandPalette {
                query: String::new(),
                selected: 0,
                font_id: self.font_id,
                items: self.items,
            }),
        )
    }
}

#[cfg(test)]
mod search_everywhere_tests {
    use super::*;

    fn items() -> Vec<PaletteItem> {
        vec![
            PaletteItem::command("editor.scene.save", "Save Scene", "Ctrl+S"),
            PaletteItem::command("editor.scene.save_all", "Save All", "Ctrl+Shift+S"),
            PaletteItem::new("entity:camera", "Camera", "Select", PaletteCategory::Entity),
            PaletteItem::new(
                "asset:scene.somnium",
                "scene.somnium",
                "Open",
                PaletteCategory::Asset,
            ),
            PaletteItem::new("help:scripting", "Scripting", "F1", PaletteCategory::Help),
        ]
    }

    fn palette(query: &str) -> CommandPalette {
        CommandPalette {
            query: query.to_string(),
            selected: 0,
            font_id: 0,
            items: items(),
        }
    }

    #[test]
    fn every_category_has_a_distinct_prefix() {
        let mut seen = Vec::new();
        for k in PaletteCategory::all() {
            assert!(!seen.contains(&k.prefix()), "{k:?} reuses a prefix");
            seen.push(k.prefix());
        }
    }

    #[test]
    fn a_bare_query_searches_every_category() {
        assert_eq!(
            palette("s").match_count(),
            5,
            "labels and action hints are searchable"
        );
    }

    #[test]
    fn a_prefix_scopes_the_search_to_one_category() {
        assert_eq!(palette("@").match_count(), 1, "entities only");
        assert_eq!(palette("#").match_count(), 1, "assets only");
        assert_eq!(palette("?").match_count(), 1, "help only");
        assert_eq!(palette(">").match_count(), 2, "commands only");
    }

    #[test]
    fn a_bare_prefix_lists_the_whole_category() {
        // Browsing, not just filtering: `@` with no terms must not match zero.
        assert!(palette("@").match_count() > 0);
    }

    #[test]
    fn a_prefix_combines_with_terms() {
        assert_eq!(palette("@cam").match_count(), 1);
        assert_eq!(palette("@save").match_count(), 0, "Save is not an entity");
    }

    #[test]
    fn prefix_matches_rank_above_contained_matches() {
        // "sa" must surface "Save Scene" before "scene.somnium", which merely
        // contains an "s"... and before anything that only contains "sa".
        let p = palette("sa");
        let first = p.filtered().first().map(|it| it.label.clone());
        assert_eq!(first.as_deref(), Some("Save Scene"));
    }

    #[test]
    fn an_unmatched_query_reports_zero_rather_than_panicking() {
        // The draw path indexes `selected` into the filtered list, so an empty
        // result set has to be handled rather than clamped into a bad index.
        assert_eq!(palette("zzzz").match_count(), 0);
    }

    #[test]
    fn parse_query_splits_scope_from_terms() {
        assert_eq!(
            CommandPalette::parse_query("@ cam"),
            (Some(PaletteCategory::Entity), "cam")
        );
        assert_eq!(CommandPalette::parse_query("save"), (None, "save"));
    }

    #[test]
    fn structured_tokens_do_not_fuzzy_match_their_vocabulary() {
        assert_eq!(palette("category:com svsc").match_count(), 2);
        assert_eq!(palette("categroy:command").match_count(), 0);
    }

    #[test]
    fn recency_breaks_equal_scores() {
        let mut p = palette("save");
        p.items[1].recency = 9;
        assert_eq!(p.filtered()[0].id, "editor.scene.save_all");
    }
}
