//! The journal: quests, people, notes and the Voice's log, drawn from `Story::journal_view`, read with the mouse or
//! the keyboard.
//!
//! The game opens it ([`JournalMessage::Open`]), keeps it fed ([`JournalMessage::SetView`] after any change) and
//! answers its [`JournalOutput`]s: `Read` (call `Story::mark_read` for each sequence and send the view again),
//! `Track` (`Story::track`), `Closed`. Words the player reads are the game's ([`JournalLabels`]).
//!
//! Restrained: a dim over the game, one panel, small widely tracked tabs, a list and a page. Journal lines told by
//! less certain voices (a lower truth tier) read fainter, so an unreliable account looks like one. A frosted note
//! shows as frosted. Portraits are the game's pictures ([`JournalMessage::SetPortrait`]); until one is painted a
//! named card stands in.
//!
//! Keys: `Q`/`E` or `Tab`/`Shift+Tab` change pages, arrows or `W`/`S` move, `T` or `Enter` tracks the quest,
//! `PageUp`/`PageDown` scroll the page, `Esc` or `J` closes. The mouse: click a tab, a row or Track; the wheel
//! scrolls what it is over.

use crate::{
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, MouseButton, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    primitive::Primitive,
    text::rich::{self, RichStyle},
    theme,
    types::Rect,
    vector_glyph::{OBJECTIVE_MARK_SVG, VectorGlyph},
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;
use somnium_story::quest_log::{ObjectiveStatus, QuestStatus};
use somnium_story::story::{JournalView, NoteView};
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};

/// The journal's pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JournalTab {
    #[default]
    Quests,
    People,
    Notes,
    /// The Voice's log.
    Voice,
}

impl JournalTab {
    const ALL: [Self; 4] = [Self::Quests, Self::People, Self::Notes, Self::Voice];

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn step(self, by: isize) -> Self {
        Self::ALL[(self.index() as isize + by).rem_euclid(4) as usize]
    }
}

/// The words the journal shows, in the game's language.
#[derive(Clone, Debug, PartialEq)]
pub struct JournalLabels {
    pub tabs: [String; 4],
    pub active: String,
    pub completed: String,
    pub failed: String,
    pub optional: String,
    pub track: String,
    pub tracked: String,
    pub frosted: String,
    pub empty: String,
}

impl Default for JournalLabels {
    fn default() -> Self {
        let s = str::to_owned;
        Self {
            tabs: [s("Quests"), s("People"), s("Notes"), s("The Voice")],
            active: s("Under way"),
            completed: s("Done"),
            failed: s("Failed"),
            optional: s("optional"),
            track: s("Track"),
            tracked: s("Tracked"),
            frosted: s("Frosted over"),
            empty: s("Nothing here yet."),
        }
    }
}

/// What the player did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalOutput {
    /// These entries are now read (the page showing them is open).
    Read(Vec<u64>),
    /// Track this quest (`None`: track nothing).
    Track(Option<String>),
    Closed,
}

/// Messages to and from the journal.
#[derive(Clone, Debug)]
pub enum JournalMessage {
    Open(bool),
    SetView(JournalView),
    SetLabels(JournalLabels),
    /// A portrait picture: `slot` from `UiCanvas::add_image`, `size` in pixels.
    SetPortrait {
        portrait: String,
        expression: String,
        slot: u32,
        size: Vec2,
    },
    /// Input from the game's own bindings: the same as the keys.
    Key(KeyCode, bool),
    /// From the widget.
    Output(JournalOutput),
}

/// Where everything goes, for a canvas of `bounds`.
#[derive(Clone, Debug, PartialEq)]
pub struct JournalLayout {
    pub unit: f32,
    pub panel: Rect,
    pub tabs: [Rect; 4],
    pub list: Rect,
    pub page: Rect,
    pub row_h: f32,
    /// The Track button, on a quest's page.
    pub track: Rect,
}

impl JournalLayout {
    #[must_use]
    pub fn new(bounds: Rect) -> Self {
        let unit = (bounds.h / 1080.0).max(0.25);
        let w = (1360.0 * unit).min(bounds.w - 48.0 * unit);
        let h = (820.0 * unit).min(bounds.h - 48.0 * unit);
        let panel = Rect::new(
            bounds.x + (bounds.w - w) * 0.5,
            bounds.y + (bounds.h - h) * 0.5,
            w,
            h,
        );
        let pad = 40.0 * unit;
        let tab_w = (w - pad * 2.0) / 4.0;
        let tabs = std::array::from_fn(|i| {
            Rect::new(
                panel.x + pad + tab_w * i as f32,
                panel.y + 26.0 * unit,
                tab_w,
                40.0 * unit,
            )
        });
        let top = panel.y + 96.0 * unit;
        let list_w = (380.0 * unit).min(w * 0.36);
        let list = Rect::new(panel.x + pad, top, list_w, panel.y + h - pad - top);
        let page_x = list.x + list.w + 40.0 * unit;
        let page = Rect::new(page_x, top, panel.x + w - pad - page_x, list.h);
        Self {
            unit,
            panel,
            tabs,
            list,
            page,
            row_h: 64.0 * unit,
            track: Rect::new(
                page.x + page.w - 120.0 * unit,
                page.y,
                120.0 * unit,
                32.0 * unit,
            ),
        }
    }

    fn row(&self, index: usize, scroll: f32) -> Rect {
        Rect::new(
            self.list.x,
            self.list.y + index as f32 * self.row_h - scroll,
            self.list.w,
            self.row_h - 6.0 * self.unit,
        )
    }
}

/// One row of a page's list.
struct Row {
    title: String,
    sub: String,
    unread: bool,
    tracked: bool,
}

/// The widget.
pub struct JournalScreen {
    font_id: u8,
    open: bool,
    view: JournalView,
    labels: JournalLabels,
    tab: JournalTab,
    selected: [usize; 4],
    list_scroll: [f32; 4],
    page_scroll: f32,
    page_height: Cell<f32>,
    portraits: HashMap<(String, String), (u32, Vec2)>,
    reported: BTreeSet<u64>,
    mark: VectorGlyph,
}

fn empty_view() -> JournalView {
    JournalView {
        quests: Vec::new(),
        notes: Vec::new(),
        unread: 0,
        people: Vec::new(),
        voice: Vec::new(),
    }
}

impl JournalScreen {
    fn layout(widget: &Widget) -> JournalLayout {
        JournalLayout::new(widget.screen_bounds())
    }

    fn note(&self, sequence: u64) -> Option<&NoteView> {
        self.view.notes.iter().find(|n| n.sequence == sequence)
    }

    fn rows(&self, tab: JournalTab) -> Vec<Row> {
        let labels = &self.labels;
        match tab {
            JournalTab::Quests => self
                .view
                .quests
                .iter()
                .map(|q| Row {
                    title: q.title.clone(),
                    sub: match q.status {
                        QuestStatus::Completed => labels.completed.clone(),
                        QuestStatus::Failed => labels.failed.clone(),
                        _ => labels.active.clone(),
                    },
                    unread: q.lines.iter().any(|l| !l.read),
                    tracked: q.tracked,
                })
                .collect(),
            JournalTab::People => self
                .view
                .people
                .iter()
                .map(|p| Row {
                    title: p.name.clone(),
                    sub: String::new(),
                    unread: p
                        .notes
                        .iter()
                        .any(|s| self.note(*s).is_some_and(|n| !n.read)),
                    tracked: false,
                })
                .collect(),
            JournalTab::Notes => self
                .view
                .notes
                .iter()
                .map(|n| Row {
                    title: if n.title.is_empty() {
                        n.category.clone()
                    } else {
                        n.title.clone()
                    },
                    sub: n.category.clone(),
                    unread: !n.read,
                    tracked: false,
                })
                .collect(),
            JournalTab::Voice => self
                .view
                .voice
                .iter()
                .rev()
                .map(|v| Row {
                    title: if v.heading.is_empty() {
                        v.category.clone()
                    } else {
                        v.heading.clone()
                    },
                    sub: v.text.clone(),
                    unread: !v.read,
                    tracked: false,
                })
                .collect(),
        }
    }

    /// Unread entries on the selected page.
    fn unread_here(&self) -> Vec<u64> {
        let at = self.selected[self.tab.index()];
        match self.tab {
            JournalTab::Quests => self.view.quests.get(at).map_or_else(Vec::new, |q| {
                q.lines
                    .iter()
                    .filter(|l| !l.read)
                    .map(|l| l.sequence)
                    .collect()
            }),
            JournalTab::People => self.view.people.get(at).map_or_else(Vec::new, |p| {
                p.notes
                    .iter()
                    .copied()
                    .filter(|s| self.note(*s).is_some_and(|n| !n.read))
                    .collect()
            }),
            JournalTab::Notes => self
                .view
                .notes
                .get(at)
                .filter(|n| !n.read)
                .map(|n| vec![n.sequence])
                .unwrap_or_default(),
            JournalTab::Voice => self
                .view
                .voice
                .iter()
                .rev()
                .nth(at)
                .filter(|v| !v.read)
                .map(|v| vec![v.sequence])
                .unwrap_or_default(),
        }
    }

    fn report_read(&mut self, outputs: &mut Vec<JournalOutput>) {
        if !self.open {
            return;
        }
        let fresh: Vec<u64> = self
            .unread_here()
            .into_iter()
            .filter(|s| self.reported.insert(*s))
            .collect();
        if !fresh.is_empty() {
            outputs.push(JournalOutput::Read(fresh));
        }
    }

    fn clamp_selection(&mut self) {
        for tab in JournalTab::ALL {
            let count = self.rows(tab).len();
            let at = &mut self.selected[tab.index()];
            *at = (*at).min(count.saturating_sub(1));
        }
    }

    fn select(&mut self, at: usize, layout: &JournalLayout, outputs: &mut Vec<JournalOutput>) {
        let count = self.rows(self.tab).len();
        if count == 0 {
            return;
        }
        let i = self.tab.index();
        self.selected[i] = at.min(count - 1);
        self.page_scroll = 0.0;
        // Keep the selection in view.
        let top = self.selected[i] as f32 * layout.row_h;
        let scroll = &mut self.list_scroll[i];
        if top < *scroll {
            *scroll = top;
        } else if top + layout.row_h > *scroll + layout.list.h {
            *scroll = top + layout.row_h - layout.list.h;
        }
        self.report_read(outputs);
    }

    fn switch(&mut self, tab: JournalTab, outputs: &mut Vec<JournalOutput>) {
        self.tab = tab;
        self.page_scroll = 0.0;
        self.report_read(outputs);
    }

    fn track_selected(&self, outputs: &mut Vec<JournalOutput>) {
        if self.tab != JournalTab::Quests {
            return;
        }
        if let Some(quest) = self.view.quests.get(self.selected[0]) {
            if quest.status == QuestStatus::Active {
                outputs.push(JournalOutput::Track(if quest.tracked {
                    None
                } else {
                    Some(quest.quest.clone())
                }));
            }
        }
    }

    fn key(
        &mut self,
        key: KeyCode,
        shift: bool,
        layout: &JournalLayout,
        outputs: &mut Vec<JournalOutput>,
    ) -> bool {
        let at = self.selected[self.tab.index()];
        match key {
            KeyCode::Escape | KeyCode::KeyJ => {
                self.open = false;
                outputs.push(JournalOutput::Closed);
            }
            KeyCode::Tab => self.switch(self.tab.step(if shift { -1 } else { 1 }), outputs),
            KeyCode::KeyE => self.switch(self.tab.step(1), outputs),
            KeyCode::KeyQ => self.switch(self.tab.step(-1), outputs),
            KeyCode::ArrowDown | KeyCode::KeyS => self.select(at + 1, layout, outputs),
            KeyCode::ArrowUp | KeyCode::KeyW => self.select(at.saturating_sub(1), layout, outputs),
            KeyCode::KeyT | KeyCode::Enter | KeyCode::NumpadEnter => self.track_selected(outputs),
            KeyCode::PageDown => self.scroll_page(layout.page.h * 0.8, layout),
            KeyCode::PageUp => self.scroll_page(-layout.page.h * 0.8, layout),
            _ => return false,
        }
        true
    }

    fn scroll_page(&mut self, by: f32, layout: &JournalLayout) {
        let most = (self.page_height.get() - layout.page.h).max(0.0);
        self.page_scroll = (self.page_scroll + by).clamp(0.0, most);
    }

    fn emit(widget: &Widget, outputs: Vec<JournalOutput>, emit: &mut Vec<UiMessage>) {
        emit.extend(outputs.into_iter().map(|output| {
            UiMessage::new(
                widget.handle,
                MessageDirection::FromWidget,
                JournalMessage::Output(output),
            )
        }));
    }

    // --- drawing ---------------------------------------------------------------------------------------------

    fn body(&self, px: f32, color: [u8; 4]) -> RichStyle {
        RichStyle {
            font: self.font_id,
            px,
            color,
            bold: None,
            italic: None,
            faces: Vec::new(),
            leading: 1.5,
        }
    }

    /// Wrapped text at `y` within the page; returns the y after it.
    fn paragraph(
        &self,
        ctx: &mut DrawingContext,
        text: &str,
        at: Vec2,
        width: f32,
        px: f32,
        color: [u8; 4],
    ) -> f32 {
        let (plain, runs) =
            crate::text::markup::parse(text).unwrap_or_else(|_| (text.to_owned(), Vec::new()));
        let atlas = &ctx.font_atlas;
        let laid = rich::layout(
            &plain,
            &runs,
            width,
            &self.body(px, color),
            |s, px, font| atlas.measure_text(s, px, font).x,
        );
        rich::draw(&laid, ctx, at, usize::MAX, 0.0, color[3]);
        at.y + laid.height()
    }

    fn caps(&self, ctx: &mut DrawingContext, text: &str, at: Vec2, px: f32, color: [u8; 4]) {
        ctx.push_text_tracked(&text.to_uppercase(), at, self.font_id, px, color, px * 0.18);
    }

    fn draw_page(&self, ctx: &mut DrawingContext, layout: &JournalLayout) {
        let theme = theme::active();
        let unit = layout.unit;
        let primary = theme.semantic.text.primary.bytes();
        let muted = theme.semantic.text.secondary.bytes();
        let accent = theme.semantic.accent.default.bytes();
        let page = layout.page;
        let x = page.x;
        let width = page.w;
        let at = self.selected[self.tab.index()];
        ctx.push_clip_rect(page);
        let top = page.y - self.page_scroll;
        let mut y = top;
        let empty = || self.rows(self.tab).is_empty();
        if empty() {
            ctx.push_text(
                &self.labels.empty,
                Vec2::new(x, y),
                self.font_id,
                18.0 * unit,
                muted,
            );
            ctx.pop_clip_rect();
            self.page_height.set(0.0);
            return;
        }
        match self.tab {
            JournalTab::Quests => {
                let quest = &self.view.quests[at];
                if quest.status == QuestStatus::Active {
                    let button = Rect::new(
                        layout.track.x,
                        layout.track.y - self.page_scroll,
                        layout.track.w,
                        layout.track.h,
                    );
                    let label = if quest.tracked {
                        &self.labels.tracked
                    } else {
                        &self.labels.track
                    };
                    ctx.push_primitive(
                        Primitive::fill(
                            button,
                            theme::with_alpha(
                                if quest.tracked {
                                    accent
                                } else {
                                    theme.semantic.surface.raised.bytes()
                                },
                                190,
                            ),
                        )
                        .with_radius(6.0 * unit),
                        None,
                    );
                    let px = 14.0 * unit;
                    let w = ctx.font_atlas.measure_text(label, px, self.font_id).x;
                    let ink = if quest.tracked {
                        theme.semantic.text.inverse.bytes()
                    } else {
                        primary
                    };
                    ctx.push_text(
                        label,
                        Vec2::new(button.x + (button.w - w) * 0.5, button.y + 8.0 * unit),
                        self.font_id,
                        px,
                        ink,
                    );
                }
                ctx.push_text(
                    &quest.title,
                    Vec2::new(x, y),
                    self.font_id,
                    30.0 * unit,
                    primary,
                );
                y += 44.0 * unit;
                let giver = self
                    .view
                    .people
                    .iter()
                    .find(|p| p.id == quest.giver)
                    .map_or(quest.giver.as_str(), |p| p.name.as_str());
                let status = match quest.status {
                    QuestStatus::Completed => &self.labels.completed,
                    QuestStatus::Failed => &self.labels.failed,
                    _ => &self.labels.active,
                };
                let meta = if giver.is_empty() {
                    status.clone()
                } else {
                    format!("{giver}  ·  {status}")
                };
                self.caps(ctx, &meta, Vec2::new(x, y), 13.0 * unit, muted);
                y += 34.0 * unit;
                if !quest.summary.is_empty() {
                    y = self.paragraph(
                        ctx,
                        &quest.summary,
                        Vec2::new(x, y),
                        width,
                        19.0 * unit,
                        primary,
                    ) + 22.0 * unit;
                }
                for objective in &quest.objectives {
                    let done = objective.status == ObjectiveStatus::Done;
                    let gone = matches!(
                        objective.status,
                        ObjectiveStatus::Failed | ObjectiveStatus::Dropped
                    );
                    let ink = if gone {
                        theme::with_alpha(muted, 150)
                    } else if done {
                        muted
                    } else {
                        primary
                    };
                    // A small lozenge: hollow while open, filled when done.
                    let c = Vec2::new(x + 8.0 * unit, y + 11.0 * unit);
                    let r = 5.0 * unit;
                    let diamond = crate::path::Path::polyline(&[
                        c + Vec2::new(0.0, -r),
                        c + Vec2::new(r, 0.0),
                        c + Vec2::new(0.0, r),
                        c + Vec2::new(-r, 0.0),
                        c + Vec2::new(0.0, -r),
                    ]);
                    let style = crate::shaped::ShapedInstance::identity(ink);
                    if done {
                        ctx.push_path(&diamond, style);
                    } else {
                        ctx.push_stroke(&diamond, &crate::path::Stroke::new(1.4 * unit), style);
                    }
                    let mut text = objective.text.clone();
                    if let Some((now, target)) = objective.progress {
                        text.push_str(&format!("  {now}/{target}"));
                    }
                    if objective.optional {
                        text.push_str(&format!("  ({})", self.labels.optional));
                    }
                    let end = self.paragraph(
                        ctx,
                        &text,
                        Vec2::new(x + 28.0 * unit, y),
                        width - 28.0 * unit,
                        18.0 * unit,
                        ink,
                    );
                    if gone {
                        let w = ctx
                            .font_atlas
                            .measure_text(&text, 18.0 * unit, self.font_id)
                            .x
                            .min(width - 28.0 * unit);
                        ctx.push_rect_filled(
                            Rect::new(x + 28.0 * unit, y + 13.0 * unit, w, 1.2 * unit),
                            ink,
                        );
                    }
                    y = end + 6.0 * unit;
                }
                y += 24.0 * unit;
                for line in &quest.lines {
                    // Less certain tellings read fainter.
                    let rank = line.tier_rank.unwrap_or(0) as u8;
                    let ink = theme::with_alpha(
                        primary,
                        245u8.saturating_sub(rank.saturating_mul(22)).max(140),
                    );
                    y = self.paragraph(ctx, &line.text, Vec2::new(x, y), width, 18.0 * unit, ink)
                        + 14.0 * unit;
                }
            }
            JournalTab::People => {
                let person = &self.view.people[at];
                let card = Rect::new(x, y, 200.0 * unit, 270.0 * unit);
                match self
                    .portraits
                    .get(&(person.portrait.clone(), person.expression.clone()))
                    .or_else(|| {
                        self.portraits
                            .get(&(person.portrait.clone(), String::new()))
                    }) {
                    Some((slot, size)) => {
                        let aspect = if size.y > 0.0 { size.x / size.y } else { 0.74 };
                        let rect = Rect::new(card.x, card.y, card.h * aspect, card.h);
                        let mut picture = Primitive::textured(rect, [0.0, 0.0, 1.0, 1.0], [255; 4]);
                        picture.flags |= crate::primitive::FLAG_SRGB_TEXELS;
                        ctx.push_primitive(picture, Some(*slot));
                    }
                    None => {
                        let colour = person.color.unwrap_or(accent);
                        ctx.push_primitive(
                            Primitive::fill(card, theme.semantic.surface.raised.bytes())
                                .with_radius(14.0 * unit)
                                .with_border(1.5 * unit, theme::with_alpha(colour, 200)),
                            None,
                        );
                        let initial: String = person
                            .name
                            .chars()
                            .take(1)
                            .flat_map(char::to_uppercase)
                            .collect();
                        let px = 84.0 * unit;
                        let w = ctx.font_atlas.measure_text(&initial, px, self.font_id).x;
                        ctx.push_text(
                            &initial,
                            Vec2::new(card.x + (card.w - w) * 0.5, card.y + 80.0 * unit),
                            self.font_id,
                            px,
                            colour,
                        );
                    }
                }
                let text_x = card.x + card.w + 32.0 * unit;
                let text_w = width - card.w - 32.0 * unit;
                ctx.push_text(
                    &person.name,
                    Vec2::new(text_x, y),
                    self.font_id,
                    30.0 * unit,
                    primary,
                );
                let mut ty = y + 56.0 * unit;
                for sequence in &person.notes {
                    if let Some(note) = self.note(*sequence) {
                        ty = self.note_text(ctx, note, text_x, ty, text_w, layout) + 18.0 * unit;
                    }
                }
                y = ty.max(card.y + card.h);
            }
            JournalTab::Notes => {
                let note = &self.view.notes[at];
                if !note.title.is_empty() {
                    ctx.push_text(
                        &note.title,
                        Vec2::new(x, y),
                        self.font_id,
                        28.0 * unit,
                        primary,
                    );
                    y += 46.0 * unit;
                }
                y = self.note_text(ctx, note, x, y, width, layout);
            }
            JournalTab::Voice => {
                let entry = self
                    .view
                    .voice
                    .iter()
                    .rev()
                    .nth(at)
                    .expect("selection is clamped");
                self.caps(ctx, &entry.heading, Vec2::new(x, y), 14.0 * unit, accent);
                y += 36.0 * unit;
                y = self.paragraph(
                    ctx,
                    &entry.text,
                    Vec2::new(x, y),
                    width,
                    24.0 * unit,
                    primary,
                );
            }
        }
        ctx.pop_clip_rect();
        self.page_height.set(y - top);
    }

    fn note_text(
        &self,
        ctx: &mut DrawingContext,
        note: &NoteView,
        x: f32,
        y: f32,
        width: f32,
        layout: &JournalLayout,
    ) -> f32 {
        let theme = theme::active();
        let unit = layout.unit;
        if note.obscured {
            self.caps(
                ctx,
                &self.labels.frosted,
                Vec2::new(x, y),
                12.0 * unit,
                theme.semantic.text.secondary.bytes(),
            );
            let ink = theme::with_alpha(theme.semantic.text.secondary.bytes(), 70);
            return self.paragraph(
                ctx,
                &note.text,
                Vec2::new(x, y + 24.0 * unit),
                width,
                18.0 * unit,
                ink,
            );
        }
        let rank = note.tier_rank.unwrap_or(0) as u8;
        let ink = theme::with_alpha(
            theme.semantic.text.primary.bytes(),
            245u8.saturating_sub(rank.saturating_mul(22)).max(140),
        );
        self.paragraph(ctx, &note.text, Vec2::new(x, y), width, 18.0 * unit, ink)
    }
}

impl Control for JournalScreen {
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Dialog
    }

    fn a11y_name(&self) -> Option<String> {
        self.open
            .then(|| self.labels.tabs[self.tab.index()].clone())
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }

    fn is_keyboard_focusable(&self) -> bool {
        self.open
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        let layout = Self::layout(widget);
        if self.open && (layout.tabs.iter().any(|t| t.contains(pos)) || layout.list.contains(pos)) {
            CursorKind::Pointer
        } else {
            CursorKind::Default
        }
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        if !self.open {
            return;
        }
        let theme = theme::active();
        let layout = Self::layout(widget);
        let unit = layout.unit;
        ctx.push_rect_filled(widget.screen_bounds(), [6, 7, 10, 215]);
        ctx.push_primitive(
            Primitive::fill(
                layout.panel,
                theme::with_alpha(theme.semantic.surface.canvas.bytes(), 245),
            )
            .with_radius(14.0 * unit)
            .with_border(
                1.0 * unit,
                theme::with_alpha(theme.semantic.border.strong.bytes(), 160),
            ),
            None,
        );
        let primary = theme.semantic.text.primary.bytes();
        let muted = theme.semantic.text.secondary.bytes();
        for (index, rect) in layout.tabs.iter().enumerate() {
            let on = index == self.tab.index();
            let px = 14.0 * unit;
            let label = self.labels.tabs[index].to_uppercase();
            let w = ctx.font_atlas.measure_text(&label, px, self.font_id).x
                + label.chars().count() as f32 * px * 0.18;
            let at = Vec2::new(rect.x + (rect.w - w) * 0.5, rect.y + 10.0 * unit);
            self.caps(ctx, &label, at, px, if on { primary } else { muted });
            if on {
                ctx.push_rect_filled(
                    Rect::new(at.x, rect.y + rect.h - 4.0 * unit, w, 1.5 * unit),
                    theme::with_alpha(primary, 200),
                );
            }
            let unread = match JournalTab::ALL[index] {
                JournalTab::Quests => self
                    .view
                    .quests
                    .iter()
                    .any(|q| q.lines.iter().any(|l| !l.read)),
                JournalTab::People => false,
                JournalTab::Notes => self.view.notes.iter().any(|n| !n.read),
                JournalTab::Voice => self.view.voice.iter().any(|v| !v.read),
            };
            if unread {
                let dot = 5.0 * unit;
                ctx.push_primitive(
                    Primitive::fill(
                        Rect::new(at.x + w + 6.0 * unit, at.y + 2.0 * unit, dot, dot),
                        theme.semantic.accent.default.bytes(),
                    )
                    .with_radius(dot * 0.5),
                    None,
                );
            }
        }
        let rows = self.rows(self.tab);
        let i = self.tab.index();
        ctx.push_clip_rect(layout.list);
        for (index, row) in rows.iter().enumerate() {
            let rect = layout.row(index, self.list_scroll[i]);
            if rect.y + rect.h < layout.list.y || rect.y > layout.list.y + layout.list.h {
                continue;
            }
            if index == self.selected[i] {
                ctx.push_primitive(
                    Primitive::fill(
                        rect,
                        theme::with_alpha(theme.semantic.surface.raised.bytes(), 200),
                    )
                    .with_radius(8.0 * unit),
                    None,
                );
            }
            let left = rect.x + 16.0 * unit;
            if row.tracked {
                let tip = Vec2::new(self.mark.size().x * 0.5, self.mark.size().y * 0.5);
                self.mark.draw(
                    ctx,
                    tip,
                    Vec2::new(rect.x + rect.w - 22.0 * unit, rect.y + rect.h * 0.5),
                    22.0 * unit,
                    0.0,
                    theme::with_alpha(primary, 220),
                );
            }
            if row.unread {
                let dot = 6.0 * unit;
                ctx.push_primitive(
                    Primitive::fill(
                        Rect::new(rect.x + 4.0 * unit, rect.y + 16.0 * unit, dot, dot),
                        theme.semantic.accent.default.bytes(),
                    )
                    .with_radius(dot * 0.5),
                    None,
                );
            }
            ctx.push_clip_rect(Rect::new(left, rect.y, rect.w - 56.0 * unit, rect.h));
            ctx.push_text(
                &row.title,
                Vec2::new(left, rect.y + 10.0 * unit),
                self.font_id,
                19.0 * unit,
                primary,
            );
            if !row.sub.is_empty() {
                ctx.push_text(
                    &row.sub,
                    Vec2::new(left, rect.y + 36.0 * unit),
                    self.font_id,
                    13.0 * unit,
                    muted,
                );
            }
            ctx.pop_clip_rect();
        }
        ctx.pop_clip_rect();
        self.draw_page(ctx, &layout);
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        let layout = Self::layout(widget);
        let mut outputs = Vec::new();
        if msg.direction == MessageDirection::ToWidget
            && let Some(message) = msg.data::<JournalMessage>().cloned()
        {
            match message {
                JournalMessage::Open(open) => {
                    self.open = open;
                    self.reported.clear();
                    self.report_read(&mut outputs);
                }
                JournalMessage::SetView(view) => {
                    self.view = view;
                    self.clamp_selection();
                    self.report_read(&mut outputs);
                }
                JournalMessage::SetLabels(labels) => self.labels = labels,
                JournalMessage::SetPortrait {
                    portrait,
                    expression,
                    slot,
                    size,
                } => {
                    self.portraits.insert((portrait, expression), (slot, size));
                }
                JournalMessage::Key(key, shift) => {
                    if self.open {
                        self.key(key, shift, &layout, &mut outputs);
                    }
                }
                JournalMessage::Output(_) => return,
            }
            widget.hit_self = self.open;
            msg.handled = true;
            Self::emit(widget, outputs, emit);
            return;
        }
        if !self.open {
            return;
        }
        let Some(message) = msg.data::<WidgetMessage>().cloned() else {
            return;
        };
        match message {
            WidgetMessage::KeyDown(key, mods) => {
                if !self.key(key, mods.shift, &layout, &mut outputs) {
                    return;
                }
            }
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(tab) = layout.tabs.iter().position(|t| t.contains(pos)) {
                    self.switch(JournalTab::ALL[tab], &mut outputs);
                } else if layout.list.contains(pos) {
                    let row = ((pos.y - layout.list.y + self.list_scroll[self.tab.index()])
                        / layout.row_h)
                        .floor();
                    if row >= 0.0 {
                        self.select(row as usize, &layout, &mut outputs);
                    }
                } else if self.tab == JournalTab::Quests
                    && Rect::new(
                        layout.track.x,
                        layout.track.y - self.page_scroll,
                        layout.track.w,
                        layout.track.h,
                    )
                    .contains(pos)
                {
                    self.track_selected(&mut outputs);
                }
            }
            WidgetMessage::MouseWheel { pos, delta, .. } => {
                let step = -delta.signum() * 60.0 * layout.unit;
                if layout.list.contains(pos) {
                    let i = self.tab.index();
                    let most =
                        (self.rows(self.tab).len() as f32 * layout.row_h - layout.list.h).max(0.0);
                    self.list_scroll[i] = (self.list_scroll[i] + step).clamp(0.0, most);
                } else {
                    self.scroll_page(step, &layout);
                }
            }
            _ => return,
        }
        widget.hit_self = self.open;
        msg.handled = true;
        Self::emit(widget, outputs, emit);
    }
}

/// Builds a [`JournalScreen`], normally stretched over a whole screen canvas.
pub struct JournalScreenBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl JournalScreenBuilder {
    #[must_use]
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, font_id: 0 }
    }

    #[must_use]
    pub fn with_font(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    #[must_use]
    pub fn build(self) -> UiNode {
        let mut widget = self.widget.with_background(theme::TRANSPARENT).build();
        widget.hit_self = false;
        UiNode::new(widget, Box::new(Self::screen(self.font_id)))
    }

    fn screen(font_id: u8) -> JournalScreen {
        JournalScreen {
            font_id,
            open: false,
            view: empty_view(),
            labels: JournalLabels::default(),
            tab: JournalTab::Quests,
            selected: [0; 4],
            list_scroll: [0.0; 4],
            page_scroll: 0.0,
            page_height: Cell::new(0.0),
            portraits: HashMap::new(),
            reported: BTreeSet::new(),
            mark: VectorGlyph::parse(OBJECTIVE_MARK_SVG).expect("the shipped mark parses"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_story::story::{LineView, ObjectiveView, PersonView, QuestPage, VoiceView};

    fn view() -> JournalView {
        let line = |sequence, read| LineView {
            sequence,
            stage: "s".into(),
            text: format!("Line {sequence}."),
            tier: "witnessed".into(),
            tier_rank: Some(2),
            at_ms: 0,
            read,
        };
        let quest = |id: &str, tracked, status, lines| QuestPage {
            quest: id.into(),
            title: id.to_uppercase(),
            summary: "Three lamps are dark.".into(),
            category: "side".into(),
            giver: "oren".into(),
            status,
            tracked,
            objectives: vec![ObjectiveView {
                id: "light".into(),
                text: "Relight the lamps".into(),
                status: ObjectiveStatus::Active,
                optional: false,
                progress: Some((1, 3)),
                remaining_ms: None,
            }],
            lines,
        };
        JournalView {
            quests: vec![
                quest(
                    "lamps",
                    true,
                    QuestStatus::Active,
                    vec![line(0, false), line(1, true)],
                ),
                quest("oil", false, QuestStatus::Active, vec![line(2, false)]),
            ],
            notes: vec![NoteView {
                sequence: 3,
                note: "promise".into(),
                category: "lore".into(),
                subject: "oren".into(),
                title: "A lit window".into(),
                text: "A promise.".into(),
                tier: "retold".into(),
                tier_rank: Some(4),
                obscured: false,
                read: false,
                at_ms: 0,
            }],
            unread: 4,
            people: vec![PersonView {
                id: "oren".into(),
                name: "Oren".into(),
                portrait: "portraits/oren".into(),
                expression: "neutral".into(),
                color: None,
                lines_heard: 3,
                notes: vec![3],
            }],
            voice: vec![VoiceView {
                sequence: 4,
                line: "lamps_burning".into(),
                category: "quest".into(),
                heading: "STREET LAMPS".into(),
                text: "All 3 lamps burn again.".into(),
                voice: String::new(),
                at_ms: 0,
                read: false,
            }],
        }
    }

    struct Rig {
        screen: JournalScreen,
        widget: Widget,
    }

    impl Rig {
        fn new() -> Self {
            let mut widget = WidgetBuilder::new().build();
            widget.actual_local_size = Vec2::new(1920.0, 1080.0);
            widget.hit_self = false;
            let mut rig = Self {
                screen: JournalScreenBuilder::screen(0),
                widget,
            };
            rig.send(JournalMessage::SetView(view()));
            rig
        }

        fn send(&mut self, message: impl std::any::Any + Send + 'static) -> Vec<JournalOutput> {
            let mut msg = UiMessage::new(self.widget.handle, MessageDirection::ToWidget, message);
            let mut emit = Vec::new();
            self.screen
                .handle_routed_message(&mut self.widget, &mut msg, &mut emit);
            emit.iter()
                .filter_map(|m| match m.data::<JournalMessage>() {
                    Some(JournalMessage::Output(out)) => Some(out.clone()),
                    _ => None,
                })
                .collect()
        }

        fn key(&mut self, key: KeyCode) -> Vec<JournalOutput> {
            self.send(WidgetMessage::KeyDown(key, Default::default()))
        }
    }

    #[test]
    fn opening_reads_the_page_in_view_once_and_keys_move_and_track() {
        let mut rig = Rig::new();
        assert!(
            !rig.widget.hit_self,
            "closed, it lets the game have the pointer"
        );
        assert_eq!(
            rig.send(JournalMessage::Open(true)),
            [JournalOutput::Read(vec![0])]
        );
        assert!(rig.widget.hit_self);
        assert!(
            rig.send(JournalMessage::SetView(view())).is_empty(),
            "read once, until the game marks it"
        );
        assert_eq!(rig.key(KeyCode::ArrowDown), [JournalOutput::Read(vec![2])]);
        assert_eq!(
            rig.key(KeyCode::KeyT),
            [JournalOutput::Track(Some("oil".into()))]
        );
        rig.key(KeyCode::KeyW);
        assert_eq!(
            rig.key(KeyCode::Enter),
            [JournalOutput::Track(None)],
            "the tracked one untracks"
        );
        assert_eq!(
            rig.key(KeyCode::KeyE),
            [JournalOutput::Read(vec![3])],
            "People: Oren's note"
        );
        assert_eq!(rig.screen.tab, JournalTab::People);
        rig.key(KeyCode::KeyE);
        assert_eq!(
            rig.key(KeyCode::KeyE),
            [JournalOutput::Read(vec![4])],
            "the Voice's log"
        );
        assert_eq!(
            rig.key(KeyCode::Tab),
            Vec::<JournalOutput>::new(),
            "round to Quests, nothing new"
        );
        assert_eq!(rig.screen.tab, JournalTab::Quests);
        assert_eq!(rig.key(KeyCode::Escape), [JournalOutput::Closed]);
        assert!(!rig.widget.hit_self);
    }

    #[test]
    fn the_mouse_picks_tabs_rows_and_track() {
        let mut rig = Rig::new();
        rig.send(JournalMessage::Open(true));
        let layout = JournalLayout::new(Rect::new(0.0, 0.0, 1920.0, 1080.0));
        let click = |pos: Vec2| WidgetMessage::MouseDown {
            pos,
            button: MouseButton::Left,
            mods: Default::default(),
        };
        let middle = |r: Rect| r.pos() + r.size() * 0.5;
        rig.send(click(middle(layout.row(1, 0.0))));
        assert_eq!(rig.screen.selected[0], 1);
        assert_eq!(
            rig.send(click(middle(layout.track))),
            [JournalOutput::Track(Some("oil".into()))]
        );
        rig.send(click(middle(layout.tabs[2])));
        assert_eq!(rig.screen.tab, JournalTab::Notes);
        assert!(
            layout.list.x + layout.list.w < layout.page.x
                && layout.panel.contains(layout.page.pos())
        );
        let small = JournalLayout::new(Rect::new(0.0, 0.0, 1280.0, 720.0));
        assert!(
            (layout.row_h / small.row_h - 1.5).abs() < 0.01,
            "it scales with the canvas"
        );
    }
}
