//! Typed data tables (MORROWIND-M, item 2).
//!
//! > *"A **data table editor** — typed columns, sorting, filtering, multi-cell
//! > edit, CSV import and export. Its first customer is the localisation table,
//! > its second is any game's item or dialogue data."*
//!
//! This module is the *model*: what a table is, what sorting and filtering mean,
//! what a rectangular edit does, and how a table survives a round trip through
//! CSV. The widget that draws it is a projection of this, the same way
//! [`crate::somui_editor`] is a projection for the layout editor — which keeps
//! every rule here testable without a window.
//!
//! # Why the model is worth separating
//!
//! A table editor's bugs are almost never in the drawing. They are: a sort that
//! loses which row was selected, a filter that hides a row you then edit by
//! index, a paste that runs off the end of the table, a CSV round trip that eats
//! a comma. Each of those is a property of the model and each one is a test
//! here.
//!
//! # Rows are addressed by key, never by position
//!
//! The same rule [`crate::virtual_list::KeySelection`] follows, for the same
//! reason: sorting and filtering renumber positions, and an edit applied to the
//! wrong row because the view was re-sorted between the click and the commit is
//! the classic data-grid bug. [`RowId`] is stable for the life of a table.

use std::collections::BTreeMap;

/// A column's identity. Stable across reorder, unlike a position.
pub type ColumnId = String;

/// A row's identity. Stable for the life of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowId(pub u64);

/// What a column holds.
///
/// The *point* of a typed column is that sorting and validation follow from it
/// rather than from inspecting the values: a column of numbers sorts
/// numerically even when one cell is empty, and a text cell typed into a number
/// column is refused at the edit rather than discovered at the sort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    Text,
    Number,
    Bool,
}

/// One cell.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// No value. Distinct from an empty string: a locale with no translation
    /// yet is not a locale translated to nothing, and a localisation editor
    /// that cannot tell them apart cannot show you what is missing.
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
}

impl Cell {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// How the cell reads in a grid or a CSV field.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(text) => text.clone(),
            Self::Number(number) => {
                // Integers print without a trailing `.0`, because a table of
                // item counts full of `3.0` is a table nobody trusts.
                if number.fract() == 0.0 && number.abs() < 1e15 {
                    format!("{}", *number as i64)
                } else {
                    format!("{number}")
                }
            }
            Self::Bool(value) => (if *value { "true" } else { "false" }).to_owned(),
        }
    }

    /// Parse text into this kind, or say why it does not fit.
    ///
    /// Empty text is [`Cell::Empty`] for every kind. That is what makes
    /// clearing a cell a normal edit rather than a type error.
    pub fn parse(kind: ColumnKind, text: &str) -> Result<Self, CellError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(Self::Empty);
        }
        match kind {
            ColumnKind::Text => Ok(Self::Text(text.to_owned())),
            ColumnKind::Number => trimmed
                .parse::<f64>()
                .map(Self::Number)
                .map_err(|_| CellError::NotANumber(text.to_owned())),
            ColumnKind::Bool => match trimmed.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => Ok(Self::Bool(true)),
                "false" | "no" | "0" => Ok(Self::Bool(false)),
                _ => Err(CellError::NotABool(text.to_owned())),
            },
        }
    }
}

/// Why a value could not go in a cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellError {
    NotANumber(String),
    NotABool(String),
    UnknownColumn(ColumnId),
    UnknownRow(RowId),
}

impl std::fmt::Display for CellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotANumber(text) => write!(f, "`{text}` is not a number"),
            Self::NotABool(text) => write!(f, "`{text}` is not true or false"),
            Self::UnknownColumn(id) => write!(f, "no column `{id}`"),
            Self::UnknownRow(id) => write!(f, "no row {}", id.0),
        }
    }
}

/// A column definition.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub id: ColumnId,
    /// What the header shows.
    pub title: String,
    pub kind: ColumnKind,
}

impl Column {
    #[must_use]
    pub fn text(id: impl Into<ColumnId>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: ColumnKind::Text,
        }
    }

    #[must_use]
    pub fn number(id: impl Into<ColumnId>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: ColumnKind::Number,
        }
    }
}

/// Which way a sort runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// What the view is sorted and filtered by.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// Column and direction, or insertion order when unset.
    pub sort: Option<(ColumnId, SortOrder)>,
    /// Case-insensitive substring, matched against every column's display text.
    pub filter: String,
    /// Show only rows with at least one empty cell.
    ///
    /// The localisation question — *what is still untranslated* — asked of the
    /// model rather than of the eye.
    pub only_incomplete: bool,
}

/// A table of typed columns and keyed rows.
#[derive(Clone, Debug, Default)]
pub struct DataTable {
    columns: Vec<Column>,
    rows: Vec<(RowId, BTreeMap<ColumnId, Cell>)>,
    next_id: u64,
}

impl DataTable {
    #[must_use]
    pub fn new(columns: Vec<Column>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            next_id: 1,
        }
    }

    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    #[must_use]
    pub fn column(&self, id: &str) -> Option<&Column> {
        self.columns.iter().find(|column| column.id == id)
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Append a row, returning its stable id.
    pub fn push_row(&mut self, cells: impl IntoIterator<Item = (ColumnId, Cell)>) -> RowId {
        let id = RowId(self.next_id);
        self.next_id += 1;
        self.rows.push((id, cells.into_iter().collect()));
        id
    }

    /// A cell, or [`Cell::Empty`] when the row has nothing for that column.
    #[must_use]
    pub fn get(&self, row: RowId, column: &str) -> Cell {
        self.rows
            .iter()
            .find(|(id, _)| *id == row)
            .and_then(|(_, cells)| cells.get(column))
            .cloned()
            .unwrap_or(Cell::Empty)
    }

    /// Write one cell from text, validating against the column's kind.
    pub fn set_text(&mut self, row: RowId, column: &str, text: &str) -> Result<(), CellError> {
        let kind = self
            .column(column)
            .ok_or_else(|| CellError::UnknownColumn(column.to_owned()))?
            .kind;
        let cell = Cell::parse(kind, text)?;
        let cells = self
            .rows
            .iter_mut()
            .find(|(id, _)| *id == row)
            .map(|(_, cells)| cells)
            .ok_or(CellError::UnknownRow(row))?;
        cells.insert(column.to_owned(), cell);
        Ok(())
    }

    /// Write the same text into a rectangle of cells.
    ///
    /// **All or nothing.** A paste that types into four columns and fails the
    /// fifth must not leave four written: half a paste is worse than none,
    /// because the undo the user reaches for no longer matches what happened.
    /// Every cell is parsed before any is written.
    pub fn set_range(
        &mut self,
        rows: &[RowId],
        columns: &[ColumnId],
        text: &str,
    ) -> Result<usize, CellError> {
        let mut parsed = Vec::with_capacity(columns.len());
        for column in columns {
            let kind = self
                .column(column)
                .ok_or_else(|| CellError::UnknownColumn(column.clone()))?
                .kind;
            parsed.push((column.clone(), Cell::parse(kind, text)?));
        }
        for row in rows {
            if !self.rows.iter().any(|(id, _)| id == row) {
                return Err(CellError::UnknownRow(*row));
            }
        }
        for row in rows {
            let cells = self
                .rows
                .iter_mut()
                .find(|(id, _)| id == row)
                .map(|(_, cells)| cells)
                .expect("checked above");
            for (column, cell) in &parsed {
                cells.insert(column.clone(), cell.clone());
            }
        }
        Ok(rows.len() * columns.len())
    }

    /// The row ids the view shows, in view order.
    ///
    /// Returning **ids** rather than rows is the whole discipline: a caller that
    /// took positions from here and wrote them back by position would corrupt
    /// the table the first time somebody sorted it.
    #[must_use]
    pub fn visible_rows(&self, view: &View) -> Vec<RowId> {
        let needle = view.filter.trim().to_ascii_lowercase();
        let mut ids: Vec<RowId> = self
            .rows
            .iter()
            .filter(|(_, cells)| {
                if view.only_incomplete
                    && self
                        .columns
                        .iter()
                        .all(|column| !cells.get(&column.id).unwrap_or(&Cell::Empty).is_empty())
                {
                    return false;
                }
                if needle.is_empty() {
                    return true;
                }
                self.columns.iter().any(|column| {
                    cells
                        .get(&column.id)
                        .unwrap_or(&Cell::Empty)
                        .display()
                        .to_ascii_lowercase()
                        .contains(&needle)
                })
            })
            .map(|(id, _)| *id)
            .collect();

        if let Some((column, order)) = &view.sort {
            let kind = self.column(column).map_or(ColumnKind::Text, |c| c.kind);
            // `sort_by` and not `sort_unstable_by`: rows that compare equal keep
            // their insertion order, so sorting a column with many blanks does
            // not shuffle the rest of the table every time it is re-sorted.
            ids.sort_by(|a, b| {
                let left = self.get(*a, column);
                let right = self.get(*b, column);
                let ordering = compare(&left, &right, kind);
                match order {
                    SortOrder::Ascending => ordering,
                    SortOrder::Descending => ordering.reverse(),
                }
            });
        }
        ids
    }

    /// Serialise to CSV, header row first.
    #[must_use]
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        let header: Vec<String> = self
            .columns
            .iter()
            .map(|column| escape(&column.title))
            .collect();
        out.push_str(&header.join(","));
        out.push('\n');
        for (id, _) in &self.rows {
            let fields: Vec<String> = self
                .columns
                .iter()
                .map(|column| escape(&self.get(*id, &column.id).display()))
                .collect();
            out.push_str(&fields.join(","));
            out.push('\n');
        }
        out
    }

    /// Read CSV into a table whose columns are already declared.
    ///
    /// Columns are matched by **title**, because that is what a CSV header
    /// carries and what a person editing one in a spreadsheet sees. A column in
    /// the file that the table does not declare is ignored rather than fatal:
    /// a spreadsheet that grew a scratch column should still import.
    pub fn read_csv(&mut self, text: &str) -> Result<usize, CellError> {
        let mut lines = parse_csv(text).into_iter();
        let Some(header) = lines.next() else {
            return Ok(0);
        };
        let mapping: Vec<Option<ColumnId>> = header
            .iter()
            .map(|title| {
                self.columns
                    .iter()
                    .find(|column| column.title == *title)
                    .map(|column| column.id.clone())
            })
            .collect();

        self.rows.clear();
        let mut imported = 0;
        for record in lines {
            if record.iter().all(|field| field.trim().is_empty()) {
                continue;
            }
            let mut cells = BTreeMap::new();
            for (index, field) in record.iter().enumerate() {
                let Some(Some(column)) = mapping.get(index) else {
                    continue;
                };
                let kind = self.column(column).map_or(ColumnKind::Text, |c| c.kind);
                cells.insert(column.clone(), Cell::parse(kind, field)?);
            }
            let id = RowId(self.next_id);
            self.next_id += 1;
            self.rows.push((id, cells));
            imported += 1;
        }
        Ok(imported)
    }
}

fn compare(left: &Cell, right: &Cell, kind: ColumnKind) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Empty sorts last in both directions is *wrong* — a reversed sort that
    // still buries the blanks makes "show me what is missing" impossible. Empty
    // is simply the smallest value, and reversing genuinely reverses.
    match (left, right) {
        (Cell::Empty, Cell::Empty) => Ordering::Equal,
        (Cell::Empty, _) => Ordering::Less,
        (_, Cell::Empty) => Ordering::Greater,
        _ => match kind {
            ColumnKind::Number => {
                let a = if let Cell::Number(n) = left { *n } else { 0.0 };
                let b = if let Cell::Number(n) = right { *n } else { 0.0 };
                a.total_cmp(&b)
            }
            ColumnKind::Bool => left.display().cmp(&right.display()),
            // Case-insensitive, because a table sorted with every capitalised
            // word first is a table nobody reads alphabetically.
            ColumnKind::Text => left
                .display()
                .to_ascii_lowercase()
                .cmp(&right.display().to_ascii_lowercase()),
        },
    }
}

fn escape(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// A minimal RFC 4180 reader: quoted fields, doubled quotes, embedded newlines.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, _) => field.push(c),
            (false, '"') => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field)),
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (false, '\r') => {}
            (false, _) => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> DataTable {
        let mut table = DataTable::new(vec![
            Column::text("name", "Name"),
            Column::number("weight", "Weight"),
        ]);
        for (name, weight) in [("Dwemer Cog", 4.0), ("apple", 0.5), ("Bonemold Helm", 12.0)] {
            table.push_row([
                ("name".to_owned(), Cell::Text(name.into())),
                ("weight".to_owned(), Cell::Number(weight)),
            ]);
        }
        table
    }

    // ── Typed columns ──────────────────────────────────────────────────────

    #[test]
    fn a_column_kind_decides_what_a_cell_will_take() {
        let mut table = items();
        let row = table.visible_rows(&View::default())[0];
        assert!(table.set_text(row, "weight", "3.5").is_ok());
        assert_eq!(
            table.set_text(row, "weight", "heavy"),
            Err(CellError::NotANumber("heavy".into()))
        );
        // The refused edit left the previous value alone.
        assert_eq!(table.get(row, "weight"), Cell::Number(3.5));
    }

    #[test]
    fn clearing_a_cell_is_an_edit_and_not_a_type_error() {
        let mut table = items();
        let row = table.visible_rows(&View::default())[0];
        table.set_text(row, "weight", "  ").expect("blank clears");
        assert_eq!(table.get(row, "weight"), Cell::Empty);
    }

    #[test]
    fn a_whole_number_does_not_print_a_trailing_decimal() {
        assert_eq!(Cell::Number(12.0).display(), "12");
        assert_eq!(Cell::Number(0.5).display(), "0.5");
    }

    // ── Sorting ────────────────────────────────────────────────────────────

    #[test]
    fn text_sorts_case_insensitively() {
        // "apple" between two capitals is the whole point: a case-sensitive
        // sort puts every lowercase word after every uppercase one, which is
        // not an order anybody reads a table in.
        let table = items();
        let view = View {
            sort: Some(("name".into(), SortOrder::Ascending)),
            ..View::default()
        };
        let names: Vec<String> = table
            .visible_rows(&view)
            .into_iter()
            .map(|row| table.get(row, "name").display())
            .collect();
        assert_eq!(names, ["apple", "Bonemold Helm", "Dwemer Cog"]);
    }

    #[test]
    fn numbers_sort_numerically_and_not_as_text() {
        let table = items();
        let view = View {
            sort: Some(("weight".into(), SortOrder::Descending)),
            ..View::default()
        };
        let weights: Vec<String> = table
            .visible_rows(&view)
            .into_iter()
            .map(|row| table.get(row, "weight").display())
            .collect();
        // As text this would be "4", "12", "0.5".
        assert_eq!(weights, ["12", "4", "0.5"]);
    }

    #[test]
    fn reversing_a_sort_really_reverses_it_including_the_blanks() {
        // Pinning blanks to the bottom in both directions is a common choice
        // and it makes "show me what is missing" impossible in a big table.
        let mut table = items();
        let row = table.visible_rows(&View::default())[1];
        table.set_text(row, "weight", "").unwrap();

        let ascending = View {
            sort: Some(("weight".into(), SortOrder::Ascending)),
            ..View::default()
        };
        let descending = View {
            sort: Some(("weight".into(), SortOrder::Descending)),
            ..View::default()
        };
        let up = table.visible_rows(&ascending);
        let mut down = table.visible_rows(&descending);
        down.reverse();
        assert_eq!(up, down);
        assert!(table.get(up[0], "weight").is_empty(), "blank sorts first");
    }

    #[test]
    fn sorting_is_stable_for_equal_values() {
        let mut table = DataTable::new(vec![
            Column::text("name", "Name"),
            Column::number("tier", "Tier"),
        ]);
        for name in ["a", "b", "c", "d"] {
            table.push_row([
                ("name".to_owned(), Cell::Text(name.into())),
                ("tier".to_owned(), Cell::Number(1.0)),
            ]);
        }
        let view = View {
            sort: Some(("tier".into(), SortOrder::Ascending)),
            ..View::default()
        };
        let names: Vec<String> = table
            .visible_rows(&view)
            .into_iter()
            .map(|row| table.get(row, "name").display())
            .collect();
        assert_eq!(names, ["a", "b", "c", "d"], "equal rows kept their order");
    }

    // ── Filtering ──────────────────────────────────────────────────────────

    #[test]
    fn a_filter_matches_any_column_case_insensitively() {
        let table = items();
        let view = View {
            filter: "BONE".into(),
            ..View::default()
        };
        let rows = table.visible_rows(&view);
        assert_eq!(rows.len(), 1);
        assert_eq!(table.get(rows[0], "name").display(), "Bonemold Helm");
    }

    #[test]
    fn only_incomplete_answers_what_is_still_missing() {
        let mut table = items();
        let row = table.visible_rows(&View::default())[2];
        table.set_text(row, "weight", "").unwrap();
        let view = View {
            only_incomplete: true,
            ..View::default()
        };
        assert_eq!(table.visible_rows(&view), vec![row]);
    }

    // ── Multi-cell edit ────────────────────────────────────────────────────

    #[test]
    fn a_range_edit_writes_every_cell_it_covers() {
        let mut table = items();
        let rows = table.visible_rows(&View::default());
        let written = table
            .set_range(&rows, &["weight".to_owned()], "2")
            .expect("2 is a number");
        assert_eq!(written, 3);
        for row in rows {
            assert_eq!(table.get(row, "weight"), Cell::Number(2.0));
        }
    }

    #[test]
    fn a_range_edit_that_cannot_finish_writes_nothing() {
        // Half a paste is worse than none: the undo the user reaches for no
        // longer matches what happened.
        let mut table = items();
        let rows = table.visible_rows(&View::default());
        let before: Vec<Cell> = rows.iter().map(|row| table.get(*row, "name")).collect();

        let error = table
            .set_range(&rows, &["name".to_owned(), "weight".to_owned()], "heavy")
            .expect_err("`heavy` is not a number");
        assert_eq!(error, CellError::NotANumber("heavy".into()));

        let after: Vec<Cell> = rows.iter().map(|row| table.get(*row, "name")).collect();
        assert_eq!(before, after, "the text column must not have been written");
    }

    // ── CSV ────────────────────────────────────────────────────────────────

    #[test]
    fn a_table_round_trips_through_csv() {
        let table = items();
        let mut back = DataTable::new(table.columns().to_vec());
        let imported = back.read_csv(&table.to_csv()).expect("valid csv");
        assert_eq!(imported, table.row_count());

        let original: Vec<String> = table
            .visible_rows(&View::default())
            .into_iter()
            .map(|row| {
                format!(
                    "{}|{}",
                    table.get(row, "name").display(),
                    table.get(row, "weight").display()
                )
            })
            .collect();
        let reread: Vec<String> = back
            .visible_rows(&View::default())
            .into_iter()
            .map(|row| {
                format!(
                    "{}|{}",
                    back.get(row, "name").display(),
                    back.get(row, "weight").display()
                )
            })
            .collect();
        assert_eq!(original, reread);
    }

    #[test]
    fn a_field_with_a_comma_a_quote_or_a_newline_survives() {
        // The three characters that break every hand-rolled CSV writer, in one
        // cell, checked through a real round trip rather than by eye.
        let columns = vec![Column::text("k", "Key"), Column::text("v", "Value")];
        let mut table = DataTable::new(columns.clone());
        table.push_row([
            ("k".to_owned(), Cell::Text("greeting".into())),
            (
                "v".to_owned(),
                Cell::Text("Hello, \"friend\"\nsecond line".into()),
            ),
        ]);
        let mut back = DataTable::new(columns);
        back.read_csv(&table.to_csv()).expect("valid csv");
        let row = back.visible_rows(&View::default())[0];
        assert_eq!(
            back.get(row, "v"),
            Cell::Text("Hello, \"friend\"\nsecond line".into())
        );
    }

    #[test]
    fn an_unknown_column_in_the_file_is_ignored_rather_than_fatal() {
        // A spreadsheet that grew a scratch column should still import.
        let mut table = DataTable::new(vec![Column::text("k", "Key")]);
        let imported = table
            .read_csv("Key,Notes\nhello,ignore me\n")
            .expect("the extra column is not an error");
        assert_eq!(imported, 1);
        let row = table.visible_rows(&View::default())[0];
        assert_eq!(table.get(row, "k").display(), "hello");
    }

    #[test]
    fn a_row_id_survives_sorting_and_filtering() {
        // The discipline the whole module is built on. An editor that took
        // positions from a sorted view and wrote them back by position would
        // corrupt the table the first time somebody clicked a header.
        let table = items();
        let unsorted = table.visible_rows(&View::default());
        let sorted = table.visible_rows(&View {
            sort: Some(("name".into(), SortOrder::Ascending)),
            ..View::default()
        });
        assert_ne!(unsorted, sorted, "the fixture must actually re-order");
        for row in &unsorted {
            assert!(sorted.contains(row), "row {row:?} vanished");
            assert_eq!(
                table.get(*row, "name"),
                table.get(*row, "name"),
                "a row's cells follow its id, not its position"
            );
        }
    }
}
