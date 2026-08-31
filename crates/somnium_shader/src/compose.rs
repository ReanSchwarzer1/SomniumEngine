//! WGSL module composition.
//!
//! Seam 3's first half: *"WGSL modules compose by named include, resolved by
//! the system, not by `include_str!` at the call site."*
//!
//! Before this, `crates/somnium_renderer/src/pass/shading.rs:540` opened with a
//! `format!` of eight `include_str!` calls, and the *order* of those eight was
//! load-bearing, invisible from the shader, and duplicated — with drift — into
//! `restir_gi.rs` and `lighting_extra.rs`. A shader that needs `brdf.wgsl` now
//! says so at the top of its own source, and the resolver works out the rest.
//!
//! # The directive language, and where it deliberately stops
//!
//! ```wgsl
//! //!include "brdf.wgsl"
//! //!if SKINNED
//! //!include "skinning.wgsl"
//! //!endif
//! ```
//!
//! Four rules keep this from becoming a second language:
//!
//! 1. `//!include` takes one quoted module name and nothing else.
//! 2. `//!if` / `//!else` / `//!endif` occupy whole lines and **do not nest**.
//! 3. A condition is one define name, optionally negated with `!`. No `&&`, no
//!    `||`, no parentheses, no `defined(...)`.
//! 4. Anything a rule above forbids is a [`ComposeError`], not a best guess.
//!
//! The reasoning is the same one that keeps a build system from growing a
//! scripting language: when a conditional wants to be cleverer than this, the
//! honest answer is a second module, and a resolver that permits the clever
//! version removes the pressure to write the honest one.
//!
//! # `enable` hoisting, which is not optional
//!
//! WGSL `enable` directives are **file-scoped and must precede every other
//! declaration**. Concatenating a module that declares `enable f16;` after one
//! that declares a struct produces a naga parse failure pointing at line 1 of a
//! file nobody edited. The resolver therefore lifts every `enable` and
//! `requires` line out of every included module, de-duplicates them, and emits
//! them first. This is the single most confusing failure the composer can
//! prevent and the cheapest to prevent.

use std::collections::{BTreeSet, HashMap, HashSet};

/// Where one line of a composed shader came from.
///
/// DREAMS-A. Composition concatenates 55 modules into one string, and naga
/// reports errors against that string: a mistake on line 48 of `brdf.wgsl` is
/// reported as line 195 of a 4,801-line text with no file name, and the
/// renderer then prefixed it with the name of the *root* module, which is a
/// file the error is not in. That was measured, not assumed, and it is the
/// tax every shader written in DREAMS-B through E would have paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The module the line was written in.
    ///
    /// The name rather than the [`ModuleId`], so a map can be read without the
    /// registry that produced it. The consumer is a diagnostic, and a
    /// diagnostic wants the word a person would type into a file open dialog.
    pub module: &'static str,
    /// Its 1-based line number in that module's own source.
    pub line: usize,
}

/// A lookup from a line of composed WGSL back to the line somebody wrote.
///
/// Runs rather than lines: consecutive composed lines from consecutive source
/// lines of one module collapse into a single span, so the map for
/// `shading.wgsl` is a handful of entries rather than 4,801. Composition
/// already walks the modules in order, so building it costs one comparison per
/// line and no extra pass.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    /// Sorted by `composed`, never empty of order.
    spans: Vec<Span>,
    /// Lines occupied by the hoisted `enable`/`requires` header, which belong
    /// to no single module because they were lifted out of several.
    header: usize,
}

#[derive(Clone, Copy, Debug)]
struct Span {
    /// 1-based first line of the run in the composed text.
    composed: usize,
    module: &'static str,
    /// 1-based source line that `composed` corresponds to.
    source: usize,
    /// How many lines the run covers.
    len: usize,
}

impl SourceMap {
    /// Note that the next composed line came from `module`'s line `source`.
    fn push(&mut self, module: &'static str, source: usize, composed: usize) {
        if let Some(last) = self.spans.last_mut() {
            // Extend the run when both sides advanced by one, which is the
            // common case and the reason this stays small.
            if last.module == module
                && last.source + last.len == source
                && last.composed + last.len == composed
            {
                last.len += 1;
                return;
            }
        }
        self.spans.push(Span {
            composed,
            module,
            source,
            len: 1,
        });
    }

    /// Shift every span down, for the `enable` header prepended after emission.
    fn shift(&mut self, lines: usize) {
        if lines == 0 {
            return;
        }
        self.header = lines;
        for span in &mut self.spans {
            span.composed += lines;
        }
    }

    /// Where a 1-based line of the composed text was written.
    ///
    /// `None` for a line in the hoisted header, and for a line past the end.
    #[must_use]
    pub fn locate(&self, composed_line: usize) -> Option<Origin> {
        if composed_line == 0 || composed_line <= self.header {
            return None;
        }
        // The last span that starts at or before the line. `partition_point`
        // rather than a scan, because a shader reload asks this once per
        // diagnostic and a scan would still be fine; the binary search is here
        // because the spans are already sorted and it costs nothing to say so.
        let index = self
            .spans
            .partition_point(|span| span.composed <= composed_line);
        let span = self.spans.get(index.checked_sub(1)?)?;
        let offset = composed_line - span.composed;
        (offset < span.len).then_some(Origin {
            module: span.module,
            line: span.source + offset,
        })
    }

    /// How many runs the map holds, for a test that cares it stays small.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Whether the map covers nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

/// A registered WGSL module.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

/// The define set a variant is compiled with, as a bitset.
///
/// Compile-time-registered (see [`crate::define`]) so a typo is a build error
/// rather than a silent cache miss on a variant nobody ever compiled — which is
/// the failure mode of every string-keyed permutation system.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Default, Debug, PartialOrd, Ord)]
pub struct Defines(pub u64);

impl Defines {
    /// The empty set.
    pub const NONE: Self = Self(0);

    /// A single-bit set. `const fn`, so define constants are compile-time.
    #[must_use]
    pub const fn bit(index: u32) -> Self {
        Self(1 << index)
    }

    /// Union.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every bit of `other` is present.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// How many defines are set.
    #[must_use]
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }
}

/// Why a source could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeError {
    /// `//!include "x.wgsl"` named a module that was never registered.
    ///
    /// Carries the including module so the message can name both ends: "who
    /// asked for this" is the half a bare "not found" always omits.
    UnknownModule {
        /// The module containing the directive.
        from: &'static str,
        /// The name it asked for.
        wanted: String,
    },
    /// The include graph contains a cycle.
    ///
    /// Reported as the path that closes it. Without cycle detection this
    /// surfaces as a stack overflow inside the resolver, which says nothing
    /// about which two files disagree.
    Cycle(Vec<&'static str>),
    /// A directive did not parse.
    BadDirective {
        /// The module it is in.
        module: &'static str,
        /// 1-based line number.
        line: usize,
        /// What was wrong, in a sentence.
        reason: String,
    },
    /// `//!if` with no matching `//!endif`, or the reverse.
    UnbalancedCondition {
        /// The module it is in.
        module: &'static str,
        /// 1-based line number of the offending directive.
        line: usize,
        /// What was wrong.
        reason: String,
    },
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownModule { from, wanted } => {
                write!(
                    f,
                    "{from}: `//!include \"{wanted}\"` names an unregistered module"
                )
            }
            Self::Cycle(path) => write!(f, "include cycle: {}", path.join(" -> ")),
            Self::BadDirective {
                module,
                line,
                reason,
            } => write!(f, "{module}:{line}: {reason}"),
            Self::UnbalancedCondition {
                module,
                line,
                reason,
            } => write!(f, "{module}:{line}: {reason}"),
        }
    }
}

impl std::error::Error for ComposeError {}

/// One registered module's source.
pub(crate) struct Module {
    pub name: &'static str,
    pub source: String,
}

/// The name-to-id registry and the resolver over it.
#[derive(Default)]
pub struct Registry {
    modules: Vec<Module>,
    by_name: HashMap<&'static str, ModuleId>,
    /// Names of the defines that exist, indexed by bit, for the budget report
    /// and for error messages. A bit with no name is a define somebody forgot
    /// to register.
    define_names: Vec<Option<&'static str>>,
}

impl Registry {
    /// Register a module under the name `//!include` will use.
    ///
    /// Re-registering an existing name **replaces its source and keeps its id**,
    /// which is exactly what hot reload needs: every cached key that referenced
    /// the module still refers to it, and only the resolved text changes.
    pub fn register(&mut self, name: &'static str, source: impl Into<String>) -> ModuleId {
        let source = source.into();
        if let Some(&id) = self.by_name.get(name) {
            self.modules[id.0 as usize].source = source;
            return id;
        }
        let id = ModuleId(self.modules.len() as u32);
        self.modules.push(Module { name, source });
        self.by_name.insert(name, id);
        id
    }

    /// Register generated source that has no durable include name.
    ///
    /// Runtime-authored material graphs need independent module ids, but their
    /// asset-derived names are not `'static`. Keeping them anonymous avoids
    /// leaking one string per reload and prevents one graph from replacing
    /// another under a shared name.
    pub fn register_generated(&mut self, source: impl Into<String>) -> ModuleId {
        let id = ModuleId(self.modules.len() as u32);
        self.modules.push(Module {
            name: "generated-material-graph",
            source: source.into(),
        });
        id
    }

    /// Give a define bit a name, for reports and diagnostics.
    pub fn register_define(&mut self, bit: u32, name: &'static str) {
        let index = bit as usize;
        if self.define_names.len() <= index {
            self.define_names.resize(index + 1, None);
        }
        self.define_names[index] = Some(name);
    }

    /// Look a module up by the name used in `//!include`.
    #[must_use]
    pub fn id(&self, name: &str) -> Option<ModuleId> {
        self.by_name.get(name).copied()
    }

    /// The registered name of a module.
    #[must_use]
    pub fn name(&self, id: ModuleId) -> Option<&'static str> {
        self.modules.get(id.0 as usize).map(|m| m.name)
    }

    /// A module's own text, before any composition.
    ///
    /// Hot reload keeps a copy so a source that fails to resolve can be put
    /// back exactly, which is what makes a failed reload non-destructive.
    #[must_use]
    pub fn raw_source(&self, id: ModuleId) -> Option<String> {
        self.modules.get(id.0 as usize).map(|m| m.source.clone())
    }

    /// How many modules are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Whether no module is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Every registered module id.
    pub fn ids(&self) -> impl Iterator<Item = ModuleId> + '_ {
        (0..self.modules.len() as u32).map(ModuleId)
    }

    /// The name of a define bit, if one was registered.
    #[must_use]
    pub fn define_name(&self, bit: u32) -> Option<&'static str> {
        self.define_names.get(bit as usize).copied().flatten()
    }

    /// Resolve `root` under `defines` into one WGSL source string.
    ///
    /// Includes are depth-first at the point of the directive and each module
    /// is emitted **at most once**, so declaration order matches the order a
    /// hand-written concatenation would have produced — which is what makes the
    /// migration from `include_str!` a move rather than a rewrite.
    pub fn resolve(&self, root: ModuleId, defines: Defines) -> Result<String, ComposeError> {
        self.resolve_mapped(root, defines).map(|(text, _)| text)
    }

    /// Resolve, and say where every line of the result came from.
    ///
    /// The map is what turns a naga diagnostic from a line number in a
    /// 209 KB blob into a file and a line somebody can open. Building it is
    /// free: composition already visits each line in order.
    pub fn resolve_mapped(
        &self,
        root: ModuleId,
        defines: Defines,
    ) -> Result<(String, SourceMap), ComposeError> {
        let mut out = String::new();
        let mut emitted = HashSet::new();
        // BTreeSet: `enable` order must be deterministic, or two runs of the
        // same key produce different source and the cache stops meaning
        // anything.
        let mut enables = BTreeSet::new();
        let mut stack = Vec::new();
        let mut map = SourceMap::default();
        let mut emitted_lines = 0usize;
        self.emit(
            root,
            defines,
            &mut out,
            &mut emitted,
            &mut enables,
            &mut stack,
            &mut map,
            &mut emitted_lines,
        )?;

        if enables.is_empty() {
            return Ok((out, map));
        }
        let mut header = String::new();
        for line in &enables {
            header.push_str(line);
            header.push('\n');
        }
        header.push_str(&out);
        map.shift(enables.len());
        Ok((header, map))
    }

    /// Every module `root` transitively includes under `defines`, itself first.
    ///
    /// Hot reload uses this in reverse: a changed module invalidates every key
    /// whose dependency set contains it.
    pub fn dependencies(
        &self,
        root: ModuleId,
        defines: Defines,
    ) -> Result<Vec<ModuleId>, ComposeError> {
        let mut out = String::new();
        let mut emitted = HashSet::new();
        let mut enables = BTreeSet::new();
        let mut stack = Vec::new();
        let mut map = SourceMap::default();
        let mut lines = 0usize;
        self.emit(
            root,
            defines,
            &mut out,
            &mut emitted,
            &mut enables,
            &mut stack,
            &mut map,
            &mut lines,
        )?;
        let mut ids: Vec<_> = emitted.into_iter().collect();
        ids.sort_unstable();
        Ok(ids)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        id: ModuleId,
        defines: Defines,
        out: &mut String,
        emitted: &mut HashSet<ModuleId>,
        enables: &mut BTreeSet<String>,
        stack: &mut Vec<&'static str>,
        map: &mut SourceMap,
        emitted_lines: &mut usize,
    ) -> Result<(), ComposeError> {
        let module = &self.modules[id.0 as usize];

        if stack.contains(&module.name) {
            let mut path = stack.clone();
            path.push(module.name);
            return Err(ComposeError::Cycle(path));
        }
        // A module included twice is emitted once. This is not an optimisation:
        // WGSL has no include guards, and a duplicated struct is a redefinition
        // error. `shading.wgsl` and `restir_gi.wgsl` both want `brdf.wgsl`.
        if !emitted.insert(id) {
            return Ok(());
        }
        stack.push(module.name);

        // `true` while the enclosing `//!if` matched (or there is none).
        let mut condition: Option<bool> = None;

        for (index, line) in module.source.lines().enumerate() {
            let number = index + 1;
            let trimmed = line.trim_start();

            if let Some(rest) = trimmed.strip_prefix("//!") {
                let rest = rest.trim();
                let (keyword, argument) = match rest.split_once(char::is_whitespace) {
                    Some((k, a)) => (k, a.trim()),
                    None => (rest, ""),
                };
                match keyword {
                    "include" => {
                        if condition == Some(false) {
                            continue;
                        }
                        let wanted =
                            parse_quoted(argument).ok_or_else(|| ComposeError::BadDirective {
                                module: module.name,
                                line: number,
                                reason: format!(
                                    "expected `//!include \"name.wgsl\"`, found `{rest}`"
                                ),
                            })?;
                        let child = self.by_name.get(wanted.as_str()).copied().ok_or(
                            ComposeError::UnknownModule {
                                from: module.name,
                                wanted: wanted.clone(),
                            },
                        )?;
                        self.emit(
                            child,
                            defines,
                            out,
                            emitted,
                            enables,
                            stack,
                            map,
                            emitted_lines,
                        )?;
                    }
                    "if" => {
                        if condition.is_some() {
                            return Err(ComposeError::UnbalancedCondition {
                                module: module.name,
                                line: number,
                                reason: "`//!if` inside `//!if` — nesting is not supported; \
                                         write a second module instead"
                                    .into(),
                            });
                        }
                        let (name, negated) = match argument.strip_prefix('!') {
                            Some(rest) => (rest.trim(), true),
                            None => (argument, false),
                        };
                        if name.is_empty() || name.split_whitespace().count() != 1 {
                            return Err(ComposeError::BadDirective {
                                module: module.name,
                                line: number,
                                reason: format!(
                                    "`//!if` takes one define name, optionally negated; \
                                     found `{argument}`"
                                ),
                            });
                        }
                        let Some(bit) = self.define_bit(name) else {
                            return Err(ComposeError::BadDirective {
                                module: module.name,
                                line: number,
                                reason: format!(
                                    "`{name}` is not a registered define — a typo here would \
                                     otherwise silently disable the block"
                                ),
                            });
                        };
                        let present = defines.contains(Defines::bit(bit));
                        condition = Some(present != negated);
                    }
                    "else" => {
                        let Some(current) = condition else {
                            return Err(ComposeError::UnbalancedCondition {
                                module: module.name,
                                line: number,
                                reason: "`//!else` without `//!if`".into(),
                            });
                        };
                        condition = Some(!current);
                    }
                    "endif" => {
                        if condition.is_none() {
                            return Err(ComposeError::UnbalancedCondition {
                                module: module.name,
                                line: number,
                                reason: "`//!endif` without `//!if`".into(),
                            });
                        }
                        condition = None;
                    }
                    other => {
                        return Err(ComposeError::BadDirective {
                            module: module.name,
                            line: number,
                            reason: format!(
                                "unknown directive `//!{other}`; known: include, if, else, endif"
                            ),
                        });
                    }
                }
                continue;
            }

            if condition == Some(false) {
                continue;
            }

            // `enable` / `requires` are file-scoped and must precede every
            // other declaration, so they are lifted rather than emitted in
            // place. See the module docs.
            if trimmed.starts_with("enable ") || trimmed.starts_with("requires ") {
                enables.insert(trimmed.trim_end().to_string());
                continue;
            }

            *emitted_lines += 1;
            map.push(module.name, number, *emitted_lines);
            out.push_str(line);
            out.push('\n');
        }

        if condition.is_some() {
            return Err(ComposeError::UnbalancedCondition {
                module: module.name,
                line: module.source.lines().count(),
                reason: "`//!if` with no `//!endif` before end of file".into(),
            });
        }

        stack.pop();
        Ok(())
    }

    fn define_bit(&self, name: &str) -> Option<u32> {
        self.define_names
            .iter()
            .position(|entry| *entry == Some(name))
            .map(|index| index as u32)
    }
}

/// Extract `"name"` from a directive argument.
fn parse_quoted(argument: &str) -> Option<String> {
    let argument = argument.trim();
    let inner = argument.strip_prefix('"')?.strip_suffix('"')?;
    if inner.is_empty() || inner.contains('"') {
        return None;
    }
    Some(inner.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKINNED: u32 = 0;
    const ALPHA_CUTOUT: u32 = 1;

    fn registry() -> Registry {
        let mut r = Registry::default();
        r.register_define(SKINNED, "SKINNED");
        r.register_define(ALPHA_CUTOUT, "ALPHA_CUTOUT");
        r
    }

    #[test]
    fn an_include_is_emitted_before_the_including_module() {
        let mut r = registry();
        r.register("brdf.wgsl", "fn brdf() {}\n");
        let root = r.register("shading.wgsl", "//!include \"brdf.wgsl\"\nfn shade() {}\n");
        let out = r.resolve(root, Defines::NONE).unwrap();
        assert_eq!(out, "fn brdf() {}\nfn shade() {}\n");
    }

    /// A module included twice is emitted once.
    ///
    /// Not an optimisation: WGSL has no include guards, so a duplicated struct
    /// is a redefinition error. `shading.wgsl` and `restir_gi.wgsl` both want
    /// `brdf.wgsl` and both are included by the same root in the real tree.
    #[test]
    fn a_diamond_include_emits_the_shared_module_once() {
        let mut r = registry();
        r.register("brdf.wgsl", "struct Brdf { x: f32 }\n");
        r.register("a.wgsl", "//!include \"brdf.wgsl\"\nfn a() {}\n");
        r.register("b.wgsl", "//!include \"brdf.wgsl\"\nfn b() {}\n");
        let root = r.register(
            "root.wgsl",
            "//!include \"a.wgsl\"\n//!include \"b.wgsl\"\nfn root() {}\n",
        );
        let out = r.resolve(root, Defines::NONE).unwrap();
        assert_eq!(out.matches("struct Brdf").count(), 1);
        assert_eq!(
            out,
            "struct Brdf { x: f32 }\nfn a() {}\nfn b() {}\nfn root() {}\n"
        );
    }

    #[test]
    fn a_cycle_is_reported_as_a_path_not_a_stack_overflow() {
        let mut r = registry();
        r.register("a.wgsl", "//!include \"b.wgsl\"\n");
        let root = r.register("b.wgsl", "//!include \"a.wgsl\"\n");
        // Registering `a` first left it pointing at a `b` that did not exist
        // yet; re-register so both resolve.
        r.register("a.wgsl", "//!include \"b.wgsl\"\n");
        let error = r.resolve(root, Defines::NONE).unwrap_err();
        match error {
            ComposeError::Cycle(path) => {
                assert_eq!(path, vec!["b.wgsl", "a.wgsl", "b.wgsl"]);
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_include_names_both_ends() {
        let mut r = registry();
        let root = r.register("shading.wgsl", "//!include \"nope.wgsl\"\n");
        let error = r.resolve(root, Defines::NONE).unwrap_err();
        assert_eq!(
            error,
            ComposeError::UnknownModule {
                from: "shading.wgsl",
                wanted: "nope.wgsl".into(),
            }
        );
        assert!(error.to_string().contains("shading.wgsl"));
        assert!(error.to_string().contains("nope.wgsl"));
    }

    #[test]
    fn conditionals_select_by_define() {
        let mut r = registry();
        r.register("skin.wgsl", "fn skin() {}\n");
        let root = r.register(
            "shading.wgsl",
            "//!if SKINNED\n//!include \"skin.wgsl\"\nlet a = 1;\n//!else\nlet a = 2;\n//!endif\nfn shade() {}\n",
        );
        let off = r.resolve(root, Defines::NONE).unwrap();
        assert_eq!(off, "let a = 2;\nfn shade() {}\n");

        let on = r.resolve(root, Defines::bit(SKINNED)).unwrap();
        assert_eq!(on, "fn skin() {}\nlet a = 1;\nfn shade() {}\n");
    }

    #[test]
    fn a_negated_condition_inverts() {
        let mut r = registry();
        let root = r.register("m.wgsl", "//!if !SKINNED\nlet a = 1;\n//!endif\n");
        assert_eq!(r.resolve(root, Defines::NONE).unwrap(), "let a = 1;\n");
        assert_eq!(r.resolve(root, Defines::bit(SKINNED)).unwrap(), "");
    }

    /// A misspelled define is an error, not a silently-disabled block.
    ///
    /// This is the single most valuable check in the composer. `//!if SKINED`
    /// under a string-keyed system compiles cleanly, produces a variant with
    /// the block missing, and is found weeks later by someone wondering why
    /// skinned meshes render untransformed.
    #[test]
    fn an_unregistered_define_is_an_error() {
        let mut r = registry();
        let root = r.register("m.wgsl", "//!if SKINED\nlet a = 1;\n//!endif\n");
        let error = r.resolve(root, Defines::bit(SKINNED)).unwrap_err();
        assert!(matches!(error, ComposeError::BadDirective { line: 1, .. }));
        assert!(error.to_string().contains("SKINED"));
    }

    #[test]
    fn nesting_is_refused_with_a_reason() {
        let mut r = registry();
        let root = r.register(
            "m.wgsl",
            "//!if SKINNED\n//!if ALPHA_CUTOUT\nlet a = 1;\n//!endif\n//!endif\n",
        );
        let error = r.resolve(root, Defines::NONE).unwrap_err();
        assert!(matches!(
            error,
            ComposeError::UnbalancedCondition { line: 2, .. }
        ));
        assert!(error.to_string().contains("second module"));
    }

    #[test]
    fn an_unclosed_condition_is_caught() {
        let mut r = registry();
        let root = r.register("m.wgsl", "//!if SKINNED\nlet a = 1;\n");
        assert!(matches!(
            r.resolve(root, Defines::NONE).unwrap_err(),
            ComposeError::UnbalancedCondition { .. }
        ));
    }

    #[test]
    fn an_unknown_directive_lists_the_known_ones() {
        let mut r = registry();
        let root = r.register("m.wgsl", "//!includ \"x.wgsl\"\n");
        let error = r.resolve(root, Defines::NONE).unwrap_err();
        assert!(error.to_string().contains("include, if, else, endif"));
    }

    /// `enable` is hoisted above everything, and de-duplicated.
    ///
    /// The failure this prevents is the most confusing one the composer can
    /// produce: WGSL requires `enable` before all declarations, so an included
    /// module's `enable` landing mid-file is a naga parse error pointing at
    /// line 1 of a file nobody edited.
    #[test]
    fn enable_directives_are_hoisted_and_deduplicated() {
        let mut r = registry();
        r.register("a.wgsl", "enable f16;\nfn a() {}\n");
        r.register(
            "b.wgsl",
            "enable f16;\nrequires readonly_and_readwrite_storage_textures;\nfn b() {}\n",
        );
        let root = r.register(
            "root.wgsl",
            "struct Root { x: f32 }\n//!include \"a.wgsl\"\n//!include \"b.wgsl\"\n",
        );
        let out = r.resolve(root, Defines::NONE).unwrap();
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(
            &lines[..2],
            &[
                "enable f16;",
                "requires readonly_and_readwrite_storage_textures;"
            ],
            "every enable must precede every declaration, including the root's own struct"
        );
        assert_eq!(out.matches("enable f16;").count(), 1);
        assert!(lines.contains(&"struct Root { x: f32 }"));
    }

    #[test]
    fn re_registering_a_module_keeps_its_id() {
        let mut r = registry();
        let first = r.register("a.wgsl", "fn a() {}\n");
        let second = r.register("a.wgsl", "fn a() { let x = 1; }\n");
        assert_eq!(
            first, second,
            "hot reload replaces source in place — every cached key that named \
             this module must still name it"
        );
        assert!(
            r.resolve(first, Defines::NONE)
                .unwrap()
                .contains("let x = 1")
        );
    }

    #[test]
    fn generated_modules_are_independent_and_cannot_shadow_named_modules() {
        let mut r = registry();
        let named = r.register("material.wgsl", "fn named() {}\n");
        let first = r.register_generated("fn first() {}\n");
        let second = r.register_generated("fn second() {}\n");

        assert_ne!(first, second);
        assert_ne!(first, named);
        assert_eq!(r.id("material.wgsl"), Some(named));
        assert_eq!(r.id("generated-material-graph"), None);
        assert_eq!(r.resolve(first, Defines::NONE).unwrap(), "fn first() {}\n");
        assert_eq!(
            r.resolve(second, Defines::NONE).unwrap(),
            "fn second() {}\n"
        );
    }

    #[test]
    fn dependencies_follow_the_active_branch_only() {
        let mut r = registry();
        let skin = r.register("skin.wgsl", "fn skin() {}\n");
        let root = r.register(
            "m.wgsl",
            "//!if SKINNED\n//!include \"skin.wgsl\"\n//!endif\nfn m() {}\n",
        );
        let off = r.dependencies(root, Defines::NONE).unwrap();
        assert_eq!(off, vec![root]);
        let on = r.dependencies(root, Defines::bit(SKINNED)).unwrap();
        assert!(on.contains(&skin) && on.contains(&root));
    }

    #[test]
    fn defines_are_a_set() {
        let both = Defines::bit(SKINNED).with(Defines::bit(ALPHA_CUTOUT));
        assert!(both.contains(Defines::bit(SKINNED)));
        assert!(both.contains(Defines::bit(ALPHA_CUTOUT)));
        assert_eq!(both.count(), 2);
        assert!(!Defines::bit(SKINNED).contains(Defines::bit(ALPHA_CUTOUT)));
        assert!(Defines::NONE.contains(Defines::NONE));
    }

    // ── DREAMS-A: the source map ────────────────────────────────────────────

    #[test]
    fn a_line_maps_back_to_the_module_it_was_written_in() {
        // The measurement that opened DREAMS-A: an error on line 48 of a
        // 120-line module arrived as "line 195" of a 4,801-line text with no
        // file name, and the renderer labelled it with the *root* module,
        // which is a file the error is not in.
        let mut r = Registry::default();
        let child = r.register("child.wgsl", "// c1\n// c2\n// c3\n");
        let root = r.register("root.wgsl", "// r1\n//!include \"child.wgsl\"\n// r2\n");
        let (text, map) = r.resolve_mapped(root, Defines::NONE).unwrap();

        assert_eq!(text, "// r1\n// c1\n// c2\n// c3\n// r2\n");
        let _ = child;
        assert_eq!(
            map.locate(1),
            Some(Origin {
                module: "root.wgsl",
                line: 1
            })
        );
        assert_eq!(
            map.locate(2),
            Some(Origin {
                module: "child.wgsl",
                line: 1
            })
        );
        assert_eq!(
            map.locate(4),
            Some(Origin {
                module: "child.wgsl",
                line: 3
            })
        );
        assert_eq!(
            map.locate(5),
            Some(Origin {
                module: "root.wgsl",
                line: 3
            })
        );
        assert_eq!(map.locate(6), None, "past the end");
        assert_eq!(map.locate(0), None, "lines are 1-based");
    }

    #[test]
    fn the_hoisted_header_belongs_to_no_module() {
        // `enable` is lifted out of whichever module declared it and emitted
        // first. Attributing those lines to one module would name a file that
        // does not have them at that line.
        let mut r = Registry::default();
        r.register("child.wgsl", "enable f16;\n// c1\n");
        let root = r.register("root.wgsl", "//!include \"child.wgsl\"\n// r1\n");
        let (text, map) = r.resolve_mapped(root, Defines::NONE).unwrap();

        assert!(text.starts_with("enable f16;\n"));
        assert_eq!(map.locate(1), None, "the hoisted header");
        assert_eq!(
            map.locate(2),
            Some(Origin {
                module: "child.wgsl",
                line: 2
            })
        );
        assert_eq!(
            map.locate(3),
            Some(Origin {
                module: "root.wgsl",
                line: 2
            })
        );
    }

    #[test]
    fn a_skipped_block_does_not_shift_the_map() {
        // Lines a `//!if` removed are not emitted, so every line after them
        // must still resolve to the line somebody wrote, not to that line plus
        // the size of the block.
        let mut r = Registry::default();
        let root = r.register(
            "root.wgsl",
            "// a\n//!if SKINNED\n// gone\n// gone\n//!endif\n// b\n",
        );
        r.register_define(SKINNED, "SKINNED");
        let (text, map) = r.resolve_mapped(root, Defines::NONE).unwrap();

        assert_eq!(text, "// a\n// b\n");
        assert_eq!(
            map.locate(1),
            Some(Origin {
                module: "root.wgsl",
                line: 1
            })
        );
        assert_eq!(
            map.locate(2),
            Some(Origin {
                module: "root.wgsl",
                line: 6
            }),
            "`// b` is line 6 of the file even though it is line 2 of the output"
        );
    }

    #[test]
    fn the_map_is_runs_rather_than_lines() {
        // A map with an entry per line would be 4,801 entries for the shading
        // pass. Consecutive lines from one module collapse, so the size is the
        // number of times composition switched files, which is small.
        let mut r = Registry::default();
        let child = r.register("child.wgsl", &"// c\n".repeat(500));
        let root = r.register(
            "root.wgsl",
            &format!(
                "{}//!include \"child.wgsl\"\n{}",
                "// r\n".repeat(500),
                "// r\n".repeat(500)
            ),
        );
        let _ = child;
        let (text, map) = r.resolve_mapped(root, Defines::NONE).unwrap();
        assert_eq!(text.lines().count(), 1500);
        assert_eq!(map.len(), 3, "root, child, root");
    }

    #[test]
    fn resolve_and_resolve_mapped_produce_the_same_text() {
        // `resolve` is the old entry point and every caller of it must keep
        // getting exactly what it got before the map existed.
        let mut r = Registry::default();
        r.register("child.wgsl", "enable f16;\n// c\n");
        let root = r.register("root.wgsl", "//!include \"child.wgsl\"\n// r\n");
        let plain = r.resolve(root, Defines::NONE).unwrap();
        let (mapped, _) = r.resolve_mapped(root, Defines::NONE).unwrap();
        assert_eq!(plain, mapped);
    }
}
