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
    Run(usize),
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
    pub label: String,
    pub hint: String,
    pub category: PaletteCategory,
}

impl PaletteItem {
    /// A command row. Kept as the terse constructor because the shell declares
    /// a few dozen of these inline.
    pub fn command(label: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
            category: PaletteCategory::Command,
        }
    }

    pub fn new(
        label: impl Into<String>,
        hint: impl Into<String>,
        category: PaletteCategory,
    ) -> Self {
        Self {
            label: label.into(),
            hint: hint.into(),
            category,
        }
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

    fn filtered(&self) -> Vec<(usize, &PaletteItem)> {
        let (scope, terms) = Self::parse_query(&self.query);
        let q = terms.to_ascii_lowercase();
        let mut out: Vec<(usize, &PaletteItem)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| scope.is_none_or(|k| it.category == k))
            .filter(|(_, it)| q.is_empty() || it.label.to_ascii_lowercase().contains(&q))
            .collect();

        // A prefix match is what the user meant; a contained match is a
        // fallback. Without this, typing "sa" surfaces "Toggle Grid Snap"
        // above "Save Scene". Ties keep declaration order, so the command list
        // stays in the sequence the shell declared it.
        if !q.is_empty() {
            out.sort_by_key(|(i, it)| {
                let starts = !it.label.to_ascii_lowercase().starts_with(&q);
                (starts, *i)
            });
        }
        out
    }

    /// Rows currently matching, for tests and for the "no matches" state.
    pub fn match_count(&self) -> usize {
        self.filtered().len()
    }
}

impl Control for CommandPalette {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        Vec2::new(420.0, 280.0)
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
        ctx.push_paint(search, &crate::style::input(crate::style::VisualState::rest()));
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
        for (row, (_orig, item)) in filtered.iter().take(10).enumerate() {
            let y = b.y + 36.0 + row as f32 * 22.0;
            let row_r = Rect::new(b.x + 8.0, y, b.w - 16.0, 22.0);
            if row == sel {
                let selected = crate::style::tree_row(crate::style::VisualState::with(
                    crate::style::Interaction::Selected,
                ));
                ctx.push_paint(row_r, &selected);
            }
            ctx.push_text(
                &item.label,
                Vec2::new(row_r.x + 8.0, y + 4.0),
                self.font_id,
                12.0,
                t.semantic.text.primary.bytes(),
            );
            // The category sits between the label and the shortcut, so a
            // result reads "what it is" before "how to reach it".
            ctx.push_text(
                item.category.label(),
                Vec2::new(row_r.x + row_r.w - 150.0, y + 5.0),
                self.font_id,
                11.0,
                t.semantic.text.muted.bytes(),
            );
            if !item.hint.is_empty() {
                ctx.push_text(
                    &item.hint,
                    Vec2::new(row_r.x + row_r.w - 80.0, y + 4.0),
                    self.font_id,
                    11.0,
                    t.semantic.text.secondary.bytes(),
                );
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(CommandPaletteMessage::SetItems(items)) =
            msg.data::<CommandPaletteMessage>()
        {
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
            WidgetMessage::KeyDown(KeyCode::Backspace) => {
                self.query.pop();
                self.selected = 0;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::ArrowDown) => {
                let n = self.filtered().len().max(1);
                self.selected = (self.selected + 1) % n;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::ArrowUp) => {
                let n = self.filtered().len().max(1);
                self.selected = (self.selected + n - 1) % n;
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::Enter | KeyCode::NumpadEnter) => {
                let filtered = self.filtered();
                if let Some((orig, _)) = filtered.get(self.selected) {
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        CommandPaletteMessage::Run(*orig),
                    ));
                }
                msg.handled = true;
            }
            WidgetMessage::MouseDown { pos, .. } => {
                let b = widget.screen_bounds();
                if pos.y > b.y + 36.0 {
                    let row = ((pos.y - b.y - 36.0) / 22.0).floor() as usize;
                    let filtered = self.filtered();
                    if let Some((orig, _)) = filtered.get(row) {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            CommandPaletteMessage::Run(*orig),
                        ));
                    }
                    msg.handled = true;
                }
            }
            _ => {}
        }
    }
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
            PaletteItem::command("Save Scene", "Ctrl+S"),
            PaletteItem::command("Save All", "Ctrl+Shift+S"),
            PaletteItem::new("Camera", "Select", PaletteCategory::Entity),
            PaletteItem::new("scene.somnium", "Open", PaletteCategory::Asset),
            PaletteItem::new("Scripting", "F1", PaletteCategory::Help),
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
        assert_eq!(palette("s").match_count(), 4, "Save x2, scene.somnium, Scripting");
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
        let first = p.filtered().first().map(|(_, it)| it.label.clone());
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
}
