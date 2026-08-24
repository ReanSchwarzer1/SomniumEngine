//! The variant cache and the budget report.
//!
//! Seam 3's second half: *"A variant is requested by key and compiled once,
//! cached by hash — the thing `hlms.rs:14` describes and does not do."*
//!
//! Everything in this file is GPU-free on purpose. The cache decides *which*
//! variant is wanted and whether it already exists; `wgpu` enters only at the
//! edge, in [`crate::ShaderSystem`]. That split is what lets the interesting
//! half — keying, invalidation, and the budget — be tested on a machine with no
//! adapter, which is most CI machines.

use std::collections::HashMap;

use crate::compose::{Defines, ModuleId};

/// A shader variant: one module plus the defines it is compiled with.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ShaderKey {
    /// The root module.
    pub module: ModuleId,
    /// The define set.
    pub defines: Defines,
}

impl ShaderKey {
    /// A variant with no defines.
    #[must_use]
    pub const fn new(module: ModuleId) -> Self {
        Self {
            module,
            defines: Defines::NONE,
        }
    }

    /// Add defines.
    #[must_use]
    pub const fn with(mut self, defines: Defines) -> Self {
        self.defines = self.defines.with(defines);
        self
    }
}

/// What one cached variant knows about itself, apart from the pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantRecord {
    /// Every module this variant's resolved source drew from, itself included.
    ///
    /// Hot reload reads this in reverse: a module whose file changed
    /// invalidates every variant listing it. Storing the dependency set per
    /// *variant* rather than per module matters because `//!if` means two
    /// variants of one module can have different dependencies.
    pub dependencies: Vec<ModuleId>,
    /// Bytes of resolved WGSL. Reported by the budget so a module that has
    /// quietly become 6,000 lines through includes is visible.
    pub source_bytes: usize,
    /// How many times [`VariantCache::lookup`] found this entry.
    ///
    /// A variant compiled and never used again is a startup stall nobody asked
    /// for, and the AOT cooker's list should not contain it.
    pub hits: u32,
}

/// The compiled-variant bookkeeping, without the pipelines.
#[derive(Default)]
pub struct VariantCache {
    records: HashMap<ShaderKey, VariantRecord>,
}

impl VariantCache {
    /// Note a hit, if the key is present.
    pub fn lookup(&mut self, key: ShaderKey) -> bool {
        match self.records.get_mut(&key) {
            Some(record) => {
                record.hits = record.hits.saturating_add(1);
                true
            }
            None => false,
        }
    }

    /// Record a freshly compiled variant.
    pub fn insert(&mut self, key: ShaderKey, dependencies: Vec<ModuleId>, source_bytes: usize) {
        self.records.insert(
            key,
            VariantRecord {
                dependencies,
                source_bytes,
                hits: 0,
            },
        );
    }

    /// Look a record up.
    #[must_use]
    pub fn record(&self, key: ShaderKey) -> Option<&VariantRecord> {
        self.records.get(&key)
    }

    /// How many variants are cached.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether nothing is cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Every cached key, sorted, so reports are reproducible.
    #[must_use]
    pub fn keys(&self) -> Vec<ShaderKey> {
        let mut keys: Vec<_> = self.records.keys().copied().collect();
        keys.sort_unstable();
        keys
    }

    /// Drop every variant that transitively depends on `module`, returning them.
    ///
    /// The returned keys are what the caller recompiles. Recompiling *exactly*
    /// these — rather than clearing the cache — is what makes a hot reload of
    /// `brdf.wgsl` take a fraction of a second instead of rebuilding every
    /// pipeline in the engine.
    pub fn invalidate(&mut self, module: ModuleId) -> Vec<ShaderKey> {
        let affected: Vec<_> = self
            .records
            .iter()
            .filter(|(_, record)| record.dependencies.contains(&module))
            .map(|(key, _)| *key)
            .collect();
        for key in &affected {
            self.records.remove(key);
        }
        let mut affected = affected;
        affected.sort_unstable();
        affected
    }

    /// Clear everything.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

/// One module's row in the variant budget report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetRow {
    /// The module's registered name.
    pub module: &'static str,
    /// How many distinct define bits any variant of it sets.
    pub defines_used: u32,
    /// `2^defines_used` — the size of the space, not the cache.
    pub possible: u64,
    /// How many variants were actually compiled.
    pub compiled: usize,
    /// Compiled variants nothing has looked up since.
    pub unused: usize,
    /// Largest resolved source, in bytes.
    pub largest_bytes: usize,
}

/// Build the variant budget report.
///
/// From `phase_MORROWIND.md` §8 item 4 and Appendix A.3.2:
///
/// ```text
/// module              defines  variants  compiled
/// shading.wgsl              6        64        11
/// terrain_material.wgsl     3         8         8
/// ```
///
/// > *A module with six independent defines has 64 possible variants and
/// > probably compiles eleven. If `compiled` approaches `variants`, the key is
/// > too coarse and the fix is splitting the module, not a bigger cache.*
///
/// The `unused` column is this implementation's addition. A variant compiled
/// once and never looked up again is a startup stall nobody asked for, and it
/// is the row `tools/shadercook` should leave out of the shipped set.
#[must_use]
pub fn budget(cache: &VariantCache, name_of: impl Fn(ModuleId) -> &'static str) -> Vec<BudgetRow> {
    let mut by_module: HashMap<ModuleId, (Defines, usize, usize, usize)> = HashMap::new();
    for (key, record) in &cache.records {
        let entry = by_module.entry(key.module).or_insert((Defines::NONE, 0, 0, 0));
        entry.0 = entry.0.with(key.defines);
        entry.1 += 1;
        entry.2 += usize::from(record.hits == 0);
        entry.3 = entry.3.max(record.source_bytes);
    }

    let mut rows: Vec<_> = by_module
        .into_iter()
        .map(|(module, (defines, compiled, unused, largest_bytes))| {
            let defines_used = defines.count();
            BudgetRow {
                module: name_of(module),
                defines_used,
                // A module never compiled with a define still has one variant.
                possible: 1u64 << defines_used.min(63),
                compiled,
                unused,
                largest_bytes,
            }
        })
        .collect();
    // Worst first: the module closest to filling its space is the one whose key
    // is too coarse, and that is the reason to read this report.
    rows.sort_by(|a, b| {
        b.compiled
            .cmp(&a.compiled)
            .then_with(|| a.module.cmp(b.module))
    });
    rows
}

/// Render [`budget`] as the plain-text table the plan sketches.
#[must_use]
pub fn budget_table(rows: &[BudgetRow]) -> String {
    let width = rows.iter().map(|r| r.module.len()).max().unwrap_or(6).max(6);
    let mut out = format!(
        "{:<width$}  defines  possible  compiled  unused  largest\n",
        "module",
        width = width
    );
    for row in rows {
        out.push_str(&format!(
            "{:<width$}  {:>7}  {:>8}  {:>8}  {:>6}  {:>6}K\n",
            row.module,
            row.defines_used,
            row.possible,
            row.compiled,
            row.unused,
            row.largest_bytes / 1024,
            width = width
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ModuleId = ModuleId(0);
    const B: ModuleId = ModuleId(1);
    const SHARED: ModuleId = ModuleId(2);

    fn name_of(id: ModuleId) -> &'static str {
        match id.0 {
            0 => "a.wgsl",
            1 => "b.wgsl",
            _ => "shared.wgsl",
        }
    }

    #[test]
    fn a_lookup_counts_a_hit_and_a_miss_does_not_invent_one() {
        let mut cache = VariantCache::default();
        assert!(!cache.lookup(ShaderKey::new(A)));
        cache.insert(ShaderKey::new(A), vec![A], 100);
        assert!(cache.lookup(ShaderKey::new(A)));
        assert!(cache.lookup(ShaderKey::new(A)));
        assert_eq!(cache.record(ShaderKey::new(A)).unwrap().hits, 2);
    }

    #[test]
    fn defines_make_distinct_variants() {
        let mut cache = VariantCache::default();
        cache.insert(ShaderKey::new(A), vec![A], 100);
        cache.insert(ShaderKey::new(A).with(Defines::bit(0)), vec![A], 140);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.record(ShaderKey::new(A)).unwrap().source_bytes, 100);
    }

    /// A changed module invalidates exactly the variants that used it.
    ///
    /// This is the whole value of hot reload: clearing the cache would work and
    /// would rebuild every pipeline in the engine, which turns "edit a file and
    /// see it" into "edit a file and wait".
    #[test]
    fn invalidation_hits_dependents_and_nothing_else() {
        let mut cache = VariantCache::default();
        cache.insert(ShaderKey::new(A), vec![A, SHARED], 100);
        cache.insert(ShaderKey::new(B), vec![B], 100);
        cache.insert(ShaderKey::new(B).with(Defines::bit(0)), vec![B, SHARED], 100);

        let dirty = cache.invalidate(SHARED);
        assert_eq!(dirty, vec![ShaderKey::new(A), ShaderKey::new(B).with(Defines::bit(0))]);
        assert_eq!(cache.len(), 1);
        assert!(cache.record(ShaderKey::new(B)).is_some(), "b.wgsl never used shared.wgsl");
    }

    /// Two variants of one module can have different dependencies.
    ///
    /// `//!if SKINNED` means the skinned variant pulls in `skinning.wgsl` and
    /// the unskinned one does not. Tracking dependencies per module rather than
    /// per variant would invalidate both, or neither, and both are wrong.
    #[test]
    fn dependencies_are_tracked_per_variant_not_per_module() {
        let mut cache = VariantCache::default();
        cache.insert(ShaderKey::new(A), vec![A], 100);
        cache.insert(ShaderKey::new(A).with(Defines::bit(0)), vec![A, SHARED], 160);
        let dirty = cache.invalidate(SHARED);
        assert_eq!(dirty, vec![ShaderKey::new(A).with(Defines::bit(0))]);
        assert!(cache.record(ShaderKey::new(A)).is_some());
    }

    #[test]
    fn the_budget_shows_the_space_against_what_was_compiled() {
        let mut cache = VariantCache::default();
        // Three independent defines seen across four compiled variants.
        for bits in [0b000u64, 0b001, 0b010, 0b101] {
            cache.insert(ShaderKey { module: A, defines: Defines(bits) }, vec![A], 2048);
        }
        cache.insert(ShaderKey::new(B), vec![B], 512);
        cache.lookup(ShaderKey::new(B));

        let rows = budget(&cache, name_of);
        assert_eq!(rows[0].module, "a.wgsl");
        assert_eq!(rows[0].defines_used, 3);
        assert_eq!(rows[0].possible, 8);
        assert_eq!(rows[0].compiled, 4);
        assert_eq!(
            rows[0].unused, 4,
            "nothing looked any of them up, and a variant compiled but never \
             used is a startup stall nobody asked for"
        );
        assert_eq!(rows[1].module, "b.wgsl");
        assert_eq!(rows[1].possible, 1, "no defines still means one variant");
        assert_eq!(rows[1].unused, 0);
    }

    #[test]
    fn the_budget_table_is_reproducible() {
        let mut cache = VariantCache::default();
        cache.insert(ShaderKey::new(A), vec![A], 3072);
        let table = budget_table(&budget(&cache, name_of));
        assert!(table.starts_with("module  defines  possible  compiled  unused  largest\n"));
        assert!(table.contains("a.wgsl"), "{table}");
        assert!(table.contains("3K"), "{table}");
    }

    #[test]
    fn clearing_empties_the_cache() {
        let mut cache = VariantCache::default();
        cache.insert(ShaderKey::new(A), vec![A], 1);
        cache.clear();
        assert!(cache.is_empty());
        assert!(budget(&cache, name_of).is_empty());
    }
}
