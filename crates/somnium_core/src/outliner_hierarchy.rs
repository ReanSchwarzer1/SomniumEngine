//! Retain the outliner hierarchy, validating its inputs every frame.
//!
//! This deliberately does not rely on editor commands or an ECS change epoch:
//! game code, undo, and inspector writes must all appear on the next frame.

use std::collections::HashMap;

use somnium_ui::OutlinerRow;

#[derive(Clone, Copy)]
pub(crate) struct Source<'a> {
    pub id: u32,
    pub generation: u32,
    pub name: Option<&'a str>,
    pub parent: Option<u32>,
}

struct CachedSource {
    id: u32,
    generation: u32,
    name: Option<String>,
    parent: Option<u32>,
}

impl CachedSource {
    fn matches(&self, source: Source<'_>) -> bool {
        self.id == source.id
            && self.generation == source.generation
            && self.name.as_deref() == source.name
            && self.parent == source.parent
    }

    fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("Entity {}", self.id))
    }
}

#[derive(Default)]
pub(crate) struct OutlinerHierarchy {
    sources: Vec<CachedSource>,
    base_names: Vec<String>,
    rows: Vec<OutlinerRow>,
}

impl OutlinerHierarchy {
    /// Check live inputs without allocating. Rebuild only on a structural or
    /// naming change; row facts are reset for the caller's live component scan.
    pub fn refresh<'a>(&mut self, sources: impl Iterator<Item = Source<'a>> + Clone) -> bool {
        let mut cached = self.sources.iter();
        let unchanged = sources
            .clone()
            .all(|source| cached.next().is_some_and(|old| old.matches(source)))
            && cached.next().is_none();
        if !unchanged {
            self.sources = sources
                .map(|source| CachedSource {
                    id: source.id,
                    generation: source.generation,
                    name: source.name.map(str::to_owned),
                    parent: source.parent,
                })
                .collect();
            self.rebuild();
        }
        for (row, name) in self.rows.iter_mut().zip(&self.base_names) {
            row.name.clone_from(name);
            row.hidden = false;
            row.locked = false;
            row.script_error = false;
            row.tags.clear();
        }
        !unchanged
    }

    pub fn rows_mut(&mut self) -> &mut [OutlinerRow] {
        &mut self.rows
    }

    fn rebuild(&mut self) {
        let mut order: Vec<_> = (0..self.sources.len()).collect();
        // Match the previous stable, ASCII-insensitive ordering, but compute
        // each lowercase key once rather than twice per comparison.
        order.sort_by_cached_key(|&index| self.sources[index].display_name().to_ascii_lowercase());
        let by_id: HashMap<_, _> = self
            .sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source.id, index))
            .collect();
        let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut roots = Vec::new();
        for &index in &order {
            let source = &self.sources[index];
            match source.parent {
                Some(parent) if by_id.contains_key(&parent) => {
                    children.entry(parent).or_default().push(index);
                }
                _ => roots.push(index),
            }
        }
        self.rows.clear();
        self.base_names.clear();
        let mut pending: Vec<_> = roots.into_iter().rev().map(|index| (index, 0u8)).collect();
        while let Some((index, depth)) = pending.pop() {
            let source = &self.sources[index];
            let kids = children.get(&source.id);
            let name = source.display_name();
            self.base_names.push(name.clone());
            self.rows.push(OutlinerRow {
                id: source.id,
                name,
                depth,
                has_children: kids.is_some_and(|kids| !kids.is_empty()),
                hidden: false,
                locked: false,
                script_error: false,
                tags: Vec::new(),
            });
            if let Some(kids) = kids {
                pending.extend(kids.iter().rev().map(|&kid| (kid, depth.saturating_add(1))));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: u32, name: Option<&str>, parent: Option<u32>) -> Source<'_> {
        Source {
            id,
            generation: 0,
            name,
            parent,
        }
    }

    #[test]
    fn live_edits_rebuild_and_unchanged_frames_keep_row_allocations() {
        let mut hierarchy = OutlinerHierarchy::default();
        let mut sources = vec![
            source(1, Some("zebra"), None),
            source(2, Some("apple"), Some(1)),
            source(3, Some("Apple"), None),
            source(4, None, Some(99)),
        ];
        assert!(hierarchy.refresh(sources.iter().copied()));
        assert_eq!(
            hierarchy
                .rows
                .iter()
                .map(|r| (r.id, r.depth))
                .collect::<Vec<_>>(),
            [(3, 0), (4, 0), (1, 0), (2, 1)]
        );
        let row_pointer = hierarchy.rows.as_ptr();
        hierarchy.rows[0].name.push_str(" [Prefab]");
        hierarchy.rows[0].hidden = true;
        hierarchy.rows[0].locked = true;
        hierarchy.rows[0].tags.push("mesh");
        assert!(!hierarchy.refresh(sources.iter().copied()));
        assert_eq!(row_pointer, hierarchy.rows.as_ptr());
        assert_eq!(hierarchy.rows[0].name, "Apple");
        assert!(!hierarchy.rows[0].hidden && !hierarchy.rows[0].locked);
        assert!(hierarchy.rows[0].tags.is_empty());

        sources[0].name = Some("aardvark");
        assert!(hierarchy.refresh(sources.iter().copied()));
        assert_eq!(hierarchy.rows[0].id, 1);
        sources[1].parent = Some(3);
        assert!(hierarchy.refresh(sources.iter().copied()));
        assert_eq!(
            hierarchy.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [1, 3, 2, 4]
        );
        sources.remove(2);
        assert!(hierarchy.refresh(sources.iter().copied()));
        assert_eq!(hierarchy.rows[1].depth, 0); // Deleted parent becomes a root.
        sources[0].generation += 1;
        assert!(hierarchy.refresh(sources.iter().copied()));
        sources.clear();
        assert!(hierarchy.refresh(sources.iter().copied()));
        assert!(hierarchy.rows.is_empty());
    }

    /// Run explicitly with --ignored --nocapture. Measures the actual retained
    /// hierarchy against the previous row construction, without a window/GPU.
    #[test]
    #[ignore]
    fn benchmark_unchanged_outliner_hierarchy() {
        use std::{hint::black_box, time::Instant};
        for count in [1_340usize, 4_294, 5_611] {
            let names: Vec<_> = (0..count)
                .map(|i| format!("Forest patch {:05} material", (i * 7919) % count))
                .collect();
            let sources: Vec<_> = names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    source(
                        i as u32,
                        Some(name),
                        (i % 7 != 0).then_some((i / 7 * 7) as u32),
                    )
                })
                .collect();
            let mut hierarchy = OutlinerHierarchy::default();
            hierarchy.refresh(sources.iter().copied());
            assert_eq!(hierarchy.rows, legacy_rows(&sources));
            let iterations = 100;
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(legacy_rows(black_box(&sources)));
            }
            let previous = start.elapsed();
            let start = Instant::now();
            for _ in 0..iterations {
                black_box(hierarchy.refresh(black_box(&sources).iter().copied()));
                black_box(&hierarchy.rows);
            }
            let retained = start.elapsed();
            println!(
                "{count} rows: previous {:.3} ms/frame; retained {:.3} ms/frame; {:.2}x hierarchy speedup",
                previous.as_secs_f64() * 1000.0 / f64::from(iterations),
                retained.as_secs_f64() * 1000.0 / f64::from(iterations),
                previous.as_secs_f64() / retained.as_secs_f64()
            );
        }
    }

    // Previous app.rs algorithm retained only as the benchmark/equality oracle.
    fn legacy_rows(sources: &[Source<'_>]) -> Vec<OutlinerRow> {
        let mut names: Vec<_> = sources
            .iter()
            .map(|s| {
                (
                    s.id,
                    s.name
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Entity {}", s.id)),
                    s.parent,
                )
            })
            .collect();
        names.sort_by(|a, b| a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase()));
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut name_of = HashMap::new();
        for (id, name, parent) in &names {
            name_of.insert(*id, name.clone());
            if let Some(parent) = parent {
                children.entry(*parent).or_default().push(*id);
            }
        }
        fn walk(
            id: u32,
            depth: u8,
            names: &HashMap<u32, String>,
            children: &HashMap<u32, Vec<u32>>,
            rows: &mut Vec<OutlinerRow>,
        ) {
            rows.push(OutlinerRow {
                id,
                name: names[&id].clone(),
                depth,
                has_children: children.get(&id).is_some_and(|c| !c.is_empty()),
                hidden: false,
                locked: false,
                script_error: false,
                tags: Vec::new(),
            });
            if let Some(kids) = children.get(&id) {
                for &kid in kids {
                    walk(kid, depth.saturating_add(1), names, children, rows);
                }
            }
        }
        let mut rows = Vec::new();
        for (id, _, parent) in &names {
            if parent.is_none_or(|parent| !name_of.contains_key(&parent)) {
                walk(*id, 0, &name_of, &children, &mut rows);
            }
        }
        rows
    }
}
