//! WGSL composition, permutation keys, a pipeline cache, and hot reload.
//!
//! Phase MORROWIND, Seam 3, built by MORROWIND-C. **This crate replaces
//! `somnium_renderer::material::hlms.rs`**, which was twenty-nine lines under a
//! doc comment describing Ogre-Next's HLMS, containing one
//! underscore-prefixed field no code read and a trailing comment beginning
//! *"In a full implementation, this would…"*. The reference architecture was
//! documented and never built. This is the building of it.
//!
//! # Why this is a prerequisite rather than a tidy-up
//!
//! **Skinning is a permutation.** So is instancing, so is alpha cutout, so is
//! two-sided shading, so is every lighting-model variant Track 7 wants to add.
//! MORROWIND-U cannot integrate skinned meshes into `shading.wgsl` without
//! either this or a fifth uber-shader branch in a file that is already 1,979
//! lines. That is why Track 0 exists at all.
//!
//! # The four pieces
//!
//! - [`compose`] — `//!include`, `//!if`, cycle detection, `enable` hoisting.
//!   Where composition is *declared in the shader* instead of assembled by a
//!   `format!` of `include_str!` calls at each pass's construction site.
//! - [`cache`] — [`ShaderKey`], per-variant dependency tracking, and the
//!   variant budget report.
//! - [`watch`] — modification-time polling for hot reload.
//! - This module — the wgpu edge: modules to sources, sources to pipelines, and
//!   the reload contract.
//!
//! The first three are GPU-free and unit-tested. `wgpu` appears only here, and
//! only in the two methods that actually create something.
//!
//! # The rule about failure
//!
//! > *"a visible toast on failure with the naga diagnostic — **never a silent
//! > revert to the old pipeline**."*
//!
//! [`ShaderSystem::apply_reload`] returns [`ReloadOutcome`], which carries
//! either the swapped keys or the diagnostic. **A failed reload leaves the
//! previous pipelines bound and says so.** Appendix A.7 makes this the specific
//! check for MORROWIND-C: introduce a deliberate WGSL syntax error, and the
//! diagnostic must be shown while the old pipeline stays bound — not a black
//! screen, not a silent revert with no message.
//!
//! # References
//!
//! `terra-main/rshader/src/` (Apache-2.0) is the primary reference: three
//! files that are hot reload in development and baked variants in release,
//! behind one interface. `bevy_mod_outline`'s `pipeline_key.rs` (MIT/Apache) is
//! [`ShaderKey`] as an idiomatic Rust type. Panda3D's interned render-state
//! objects were read as the alternative answer and deliberately not taken.
//! Daemon's `gl_shader.cpp` is **GPL** and was not read for this. See
//! `ATTRIBUTION.md` §13H.5 and §13H.7.

#![deny(missing_docs)]

pub mod cache;
pub mod compose;
mod spirv;
pub mod watch;

pub use cache::{BudgetRow, ShaderKey, VariantCache, VariantRecord, budget, budget_table};
pub use compose::{ComposeError, Defines, ModuleId, Registry};
pub use compose::{Origin, SourceMap};
pub use spirv::SpirvEntryPoint;
pub use watch::SourceWatcher;

use std::collections::HashMap;

/// Why a variant could not be produced.
#[derive(Clone, Debug)]
pub enum ShaderError {
    /// The source could not be resolved.
    Compose(ComposeError),
    /// naga rejected the resolved source.
    ///
    /// The diagnostic is carried verbatim. It is the only useful thing in a
    /// shader failure, and a system that swallows it in favour of "shader
    /// compilation failed" has converted a two-minute fix into an afternoon.
    Validation {
        /// The module that failed.
        module: &'static str,
        /// naga's message.
        diagnostic: String,
    },
    /// A key was requested for a module that was never registered.
    UnknownModule(ModuleId),
    /// A named Slang/SPIR-V module was not registered.
    UnknownSpirvModule(String),
    /// A checked-in Slang/SPIR-V artifact was malformed or unsupported by the
    /// active adapter.
    Spirv {
        /// The authored Slang module.
        module: &'static str,
        /// The precise artifact or capability failure.
        diagnostic: String,
    },
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compose(error) => write!(f, "{error}"),
            Self::Validation { module, diagnostic } => write!(f, "{module}: {diagnostic}"),
            Self::UnknownModule(id) => write!(f, "module {} is not registered", id.0),
            Self::UnknownSpirvModule(name) => {
                write!(f, "Slang/SPIR-V module `{name}` is not registered")
            }
            Self::Spirv { module, diagnostic } => write!(f, "{module}: {diagnostic}"),
        }
    }
}

impl std::error::Error for ShaderError {}

impl From<ComposeError> for ShaderError {
    fn from(error: ComposeError) -> Self {
        Self::Compose(error)
    }
}

/// What one call to [`ShaderSystem::apply_reload`] did.
///
/// Deliberately not a `Result`: a reload that fails is not an error the caller
/// should propagate, it is a *state* the editor has to display while carrying
/// on with the pipelines it already had.
#[derive(Clone, Debug, Default)]
pub struct ReloadOutcome {
    /// Modules whose source was replaced.
    pub reloaded: Vec<&'static str>,
    /// Variants invalidated and needing recompilation.
    pub invalidated: Vec<ShaderKey>,
    /// Failures, each with the naga diagnostic.
    ///
    /// **Non-empty means the old pipelines are still bound.** That is the whole
    /// contract: a broken edit must be visible and non-destructive, in that
    /// order.
    pub failures: Vec<ShaderError>,
}

impl ReloadOutcome {
    /// Whether anything happened at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reloaded.is_empty() && self.invalidated.is_empty() && self.failures.is_empty()
    }

    /// A one-line summary for a toast.
    #[must_use]
    pub fn summary(&self) -> String {
        if let Some(first) = self.failures.first() {
            let more = self.failures.len().saturating_sub(1);
            return if more == 0 {
                format!("Shader reload failed — {first}")
            } else {
                format!("Shader reload failed — {first} (and {more} more)")
            };
        }
        format!(
            "Reloaded {} shader module(s), {} variant(s) recompiled",
            self.reloaded.len(),
            self.invalidated.len()
        )
    }
}

/// The module registry, the variant cache, and the pipelines.
///
/// One per renderer. `tools/ghostfence/run.py`'s `no-second-system` row forbids
/// a second type named `ShaderSystem` outside this crate, for the same reason
/// it forbids a second job system: two caches for one thing means two answers
/// to "which pipeline is bound", and they will disagree under exactly the
/// conditions that make it hardest to find out.
#[derive(Default)]
pub struct ShaderSystem {
    registry: Registry,
    variants: VariantCache,
    /// Resolved WGSL per key, so a recompile after a reload does not redo the
    /// composition work — and so a test can assert what was fed to naga.
    sources: HashMap<ShaderKey, String>,
    /// Where each variant's composed lines came from, so a diagnostic can name
    /// the file somebody has open rather than an offset into a 209 KB string.
    maps: HashMap<ShaderKey, compose::SourceMap>,
    /// Checked-in Slang cooks, registered beside WGSL rather than in a second
    /// shader system. Names are the authored `.slang` paths so diagnostics lead
    /// back to the file a developer edits.
    spirv: HashMap<&'static str, spirv::SpirvArtifact>,
    watcher: SourceWatcher,
}

impl ShaderSystem {
    /// An empty system.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a WGSL module under the name `//!include` will use.
    ///
    /// Re-registering keeps the id, so hot reload replaces source in place and
    /// every cached key that named the module still names it.
    pub fn register(&mut self, name: &'static str, source: impl Into<String>) -> ModuleId {
        self.registry.register(name, source)
    }

    /// Register an authored WGSL module under its own cache identity.
    pub fn register_generated(&mut self, source: impl Into<String>) -> ModuleId {
        self.registry.register_generated(source)
    }

    /// Register a module and watch its file for changes.
    ///
    /// `source` is the compiled-in text (from `include_str!`, so a release
    /// build needs no files at all) and `path` is where the same file lives in
    /// the working tree. Debug builds poll the path; release builds never call
    /// [`ShaderSystem::poll_reload`], so the path is inert.
    pub fn register_watched(
        &mut self,
        name: &'static str,
        source: impl Into<String>,
        path: std::path::PathBuf,
    ) -> ModuleId {
        let id = self.registry.register(name, source);
        self.watcher.watch(id, path);
        id
    }

    /// Register a checked-in SPIR-V cook of an authored Slang module.
    ///
    /// The artifact is structurally checked here. Semantic and byte-for-byte
    /// reproducibility checks belong to `tools/slangcook`, which has access to
    /// the compiler and runs before the artifact is committed.
    ///
    /// # Safety
    ///
    /// `bytes` must be a trusted, semantically valid artifact produced by the
    /// declared compiler/tooling. Passthrough modules bypass wgpu's normal
    /// validation and malformed input may crash a graphics driver. This API is
    /// unsafe so arbitrary callers cannot cross that boundary through a safe
    /// registry method.
    pub unsafe fn register_spirv(
        &mut self,
        name: &'static str,
        bytes: &[u8],
        entry_points: &[SpirvEntryPoint],
    ) -> Result<(), ShaderError> {
        let artifact = spirv::SpirvArtifact::parse(bytes, entry_points).map_err(|diagnostic| {
            ShaderError::Spirv {
                module: name,
                diagnostic,
            }
        })?;
        self.spirv.insert(name, artifact);
        Ok(())
    }

    /// Every shader name owned by the system, across WGSL and Slang.
    #[must_use]
    pub fn registered_names(&self) -> Vec<&'static str> {
        let mut names: Vec<_> = self
            .registry
            .ids()
            .filter_map(|id| self.registry.name(id))
            .collect();
        names.extend(self.spirv.keys().copied());
        names.sort_unstable();
        names
    }

    /// Create a passthrough shader module from a registered Slang/SPIR-V cook.
    pub fn spirv_module(
        &self,
        device: &wgpu::Device,
        name: &str,
    ) -> Result<wgpu::ShaderModule, ShaderError> {
        let (registered_name, artifact) = self
            .spirv
            .get_key_value(name)
            .ok_or_else(|| ShaderError::UnknownSpirvModule(name.to_string()))?;
        artifact
            .create_module(device, registered_name)
            .map_err(|diagnostic| ShaderError::Spirv {
                module: registered_name,
                diagnostic,
            })
    }

    /// Words in a registered SPIR-V artifact, for reports and GPU-free tests.
    #[must_use]
    pub fn spirv_words(&self, name: &str) -> Option<&[u32]> {
        self.spirv.get(name).map(spirv::SpirvArtifact::words)
    }

    /// Give a define bit a name, so `//!if NAME` resolves and a typo does not.
    pub fn register_define(&mut self, bit: u32, name: &'static str) {
        self.registry.register_define(bit, name);
    }

    /// The registry, for callers that need to resolve names to ids.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The variant cache, for reports.
    #[must_use]
    pub fn variants(&self) -> &VariantCache {
        &self.variants
    }

    /// Resolve a variant to WGSL, caching the resolved text.
    ///
    /// This is the whole system minus wgpu, and it is what
    /// `create_shader_module` is handed. Separating it out is what lets the
    /// composition be tested without an adapter.
    pub fn source(&mut self, key: ShaderKey) -> Result<&str, ShaderError> {
        if self.registry.name(key.module).is_none() {
            return Err(ShaderError::UnknownModule(key.module));
        }
        if !self.sources.contains_key(&key) {
            let (text, map) = self.registry.resolve_mapped(key.module, key.defines)?;
            let dependencies = self.registry.dependencies(key.module, key.defines)?;
            self.variants.insert(key, dependencies, text.len());
            self.sources.insert(key, text);
            self.maps.insert(key, map);
        }
        self.variants.lookup(key);
        Ok(self.sources.get(&key).expect("just inserted"))
    }

    /// Which module and line a line of a variant's composed source came from.
    ///
    /// DREAMS-A. `source` hands naga one string built from up to eight files,
    /// and naga reports against that string. Without this, an error on line 48
    /// of `brdf.wgsl` arrives as "line 195" of an unnamed 4,801-line text, and
    /// the renderer then labelled it with the *root* module's name, which is a
    /// file the error is not in.
    ///
    /// `None` for a line in the hoisted `enable` header, which belongs to no
    /// single module because it was lifted out of several, and for a variant
    /// that has not been resolved yet.
    #[must_use]
    pub fn locate(&self, key: ShaderKey, composed_line: usize) -> Option<Origin> {
        self.maps.get(&key)?.locate(composed_line)
    }

    /// Create a `wgpu::ShaderModule` for a variant.
    ///
    /// The wgpu edge, and one of only two places in the crate that touches it.
    ///
    /// Compilation is synchronous here and stalls one draw on a miss. Seam 3
    /// says a miss should be paid off-frame through `somnium_jobs`; the shape
    /// that allows it is [`ShaderSystem::source`], which does the expensive
    /// composition and can be called from a job — `wgpu::Device` is `Send` but
    /// module creation still belongs on the thread that owns the queue.
    pub fn module(
        &mut self,
        device: &wgpu::Device,
        key: ShaderKey,
    ) -> Result<wgpu::ShaderModule, ShaderError> {
        let name = self
            .registry
            .name(key.module)
            .ok_or(ShaderError::UnknownModule(key.module))?;
        let source = self.source(key)?.to_string();
        Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        }))
    }

    /// Poll watched files. Debug builds only; a no-op in release.
    ///
    /// Returns the modules whose text changed, without applying anything —
    /// applying is [`ShaderSystem::apply_reload`], and keeping them separate is
    /// what lets the poll run on a background job while the swap stays on the
    /// thread that owns the pipelines.
    pub fn poll_reload(&mut self) -> Vec<(ModuleId, String)> {
        if cfg!(debug_assertions) {
            self.watcher.poll()
        } else {
            Vec::new()
        }
    }

    /// Apply changed sources and report what needs recompiling.
    ///
    /// **A module whose new source does not resolve is not installed.** Its old
    /// text stays, its variants stay cached, and the failure is reported with
    /// naga's diagnostic in [`ReloadOutcome::failures`]. That is the difference
    /// between a hot reload people trust and one they turn off: a typo mid-edit
    /// must cost a toast, never the frame.
    ///
    /// Validation happens against the *dependent variants*, not the changed
    /// module alone — a module is a fragment, and a fragment does not have to
    /// be a valid WGSL program on its own.
    ///
    /// `validate` is handed the root's name, the composed source, and the map
    /// from that source's lines back to the files they were written in
    /// (DREAMS-A). The map is a parameter rather than something the caller
    /// looks up afterwards because this method holds the system while the
    /// closure runs, so the closure cannot ask for it.
    pub fn apply_reload(
        &mut self,
        changed: Vec<(ModuleId, String)>,
        mut validate: impl FnMut(&str, &str, &compose::SourceMap) -> Result<(), String>,
    ) -> ReloadOutcome {
        let mut outcome = ReloadOutcome::default();

        for (module, source) in changed {
            let Some(name) = self.registry.name(module) else {
                outcome.failures.push(ShaderError::UnknownModule(module));
                continue;
            };
            // The variants that would have to change, computed *before* the
            // swap so a failure can put everything back exactly.
            let affected = self.variants.invalidate(module);
            let saved: Vec<_> = affected
                .iter()
                .filter_map(|key| self.sources.get(key).map(|s| (*key, s.clone())))
                .collect();
            let old_source = self.module_source(module);

            self.registry.register(name, source);
            for key in &affected {
                self.sources.remove(key);
            }

            let mut failure = None;
            let mut recompiled = Vec::new();
            for key in &affected {
                match self.registry.resolve_mapped(key.module, key.defines) {
                    Ok((text, map)) => {
                        if let Err(diagnostic) = validate(name, &text, &map) {
                            failure = Some(ShaderError::Validation {
                                module: name,
                                diagnostic,
                            });
                            break;
                        }
                        recompiled.push((*key, text));
                    }
                    Err(error) => {
                        failure = Some(ShaderError::Compose(error));
                        break;
                    }
                }
            }

            match failure {
                Some(error) => {
                    // Put everything back. The old pipelines are still bound
                    // because nothing downstream was told to swap.
                    if let Some(old) = old_source {
                        self.registry.register(name, old);
                    }
                    for (key, text) in saved {
                        let dependencies = self
                            .registry
                            .dependencies(key.module, key.defines)
                            .unwrap_or_else(|_| vec![key.module]);
                        self.variants.insert(key, dependencies, text.len());
                        self.sources.insert(key, text);
                    }
                    outcome.failures.push(error);
                }
                None => {
                    for (key, text) in recompiled {
                        let dependencies = self
                            .registry
                            .dependencies(key.module, key.defines)
                            .unwrap_or_else(|_| vec![key.module]);
                        self.variants.insert(key, dependencies, text.len());
                        self.sources.insert(key, text);
                    }
                    outcome.reloaded.push(name);
                    outcome.invalidated.extend(affected);
                }
            }
        }

        outcome
    }

    /// The raw registered text of one module, before composition.
    fn module_source(&self, module: ModuleId) -> Option<String> {
        self.registry.raw_source(module)
    }

    /// The variant budget report (plan §8 item 4).
    #[must_use]
    pub fn budget(&self) -> Vec<BudgetRow> {
        let registry = &self.registry;
        budget(&self.variants, |id| {
            registry.name(id).unwrap_or("<unknown>")
        })
    }

    /// The budget as a plain-text table.
    #[must_use]
    pub fn budget_table(&self) -> String {
        budget_table(&self.budget())
    }
}

#[cfg(test)]
mod tests;
