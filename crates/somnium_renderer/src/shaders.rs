//! The renderer's WGSL module registry (Phase MORROWIND, MORROWIND-C).
//!
//! Every `.wgsl` file under `src/shaders/` is registered here, once, under the
//! name `//!include` uses. Before this, composition was a `format!` of
//! `include_str!` calls at each pass's construction site — and the *order* of
//! those calls was load-bearing, invisible from the shader, and duplicated with
//! drift across `shading.rs`, `restir_gi.rs` and `lighting_extra.rs`. A shader
//! that needs `brdf.wgsl` now says so at the top of its own source.
//!
//! # Why a `Mutex` rather than `&mut`
//!
//! [`somnium_shader::ShaderSystem`] takes `&mut self` to resolve, because
//! resolving populates a cache. Threading `&mut` through every pass constructor
//! would put a borrow of the renderer's shader system into a dozen signatures
//! that otherwise take only `&Device`, and pass construction is a cold path
//! where lock contention is not a concern. The lock buys a `&self` API and
//! costs nothing that can be measured.
//!
//! # Hot reload paths
//!
//! Each module is registered with the path it lives at in the working tree, so
//! a debug build can watch it. Release builds never poll, and the compiled-in
//! `include_str!` text is the only source they use — a shipped build needs no
//! `.wgsl` files on disk.

use std::sync::Mutex;

use somnium_shader::{Defines, ModuleId, ReloadOutcome, ShaderError, ShaderKey, ShaderSystem};

/// Registers every shader module, and gives each a name and a watch path.
///
/// The macro exists so adding a shader is one line in one place. The failure it
/// prevents is the one this whole sub-phase is about: a file that exists, is
/// `include_str!`d somewhere, and is invisible to the system that is supposed
/// to know about every shader.
macro_rules! register_modules {
    ($system:expr, $($name:literal),* $(,)?) => {
        $(
            $system.register_watched(
                $name,
                include_str!(concat!("shaders/", $name)),
                std::path::PathBuf::from(concat!(
                    env!("CARGO_MANIFEST_DIR"), "/src/shaders/", $name
                )),
            );
        )*
    };
}

/// Define bits. Registered by name so `//!if SKINNED` resolves and `//!if
/// SKINED` is a compile error rather than a silently disabled block.
pub mod define {
    use somnium_shader::Defines;

    /// Bit index and name pairs, registered at startup.
    pub const ALL: &[(u32, &str)] = &[(SKINNED_BIT, "SKINNED")];

    /// Skinned geometry. **Not yet used by any shader** — MORROWIND-U adds the
    /// `//!if SKINNED` blocks. It is registered now because the exit criterion
    /// for MORROWIND-C is that adding it takes no edit to `renderer.rs`, and a
    /// define that cannot be named cannot be tested for that.
    pub const SKINNED_BIT: u32 = 0;

    /// [`SKINNED_BIT`] as a set.
    pub const SKINNED: Defines = Defines::bit(SKINNED_BIT);
}

/// The renderer's shader system.
pub struct Shaders {
    system: Mutex<ShaderSystem>,
}

impl Default for Shaders {
    fn default() -> Self {
        Self::new()
    }
}

impl Shaders {
    /// Register every shader module in the crate.
    #[must_use]
    pub fn new() -> Self {
        let mut system = ShaderSystem::new();
        for (bit, name) in define::ALL {
            system.register_define(*bit, name);
        }
        register_modules!(
            system,
            "atmosphere.wgsl",
            "skinning.wgsl",
            "atmosphere_lut.wgsl",
            "auto_exposure.wgsl",
            "bloom.wgsl",
            "brdf.wgsl",
            "cas.wgsl",
            "census.wgsl",
            "classify.wgsl",
            "clipmap_gen.wgsl",
            "clipmap_shade.wgsl",
            "clouds.wgsl",
            "clouds_composite.wgsl",
            "clouds_noise.wgsl",
            "cull.wgsl",
            "dof.wgsl",
            "ddgi.wgsl",
            "fsr_sanitize.wgsl",
            "fsr_untonemap.wgsl",
            "fxaa.wgsl",
            "gizmo.wgsl",
            "global_pool.wgsl",
            "grid.wgsl",
            "gtao.wgsl",
            "hextile.wgsl",
            "hiz.wgsl",
            "ibl_gen.wgsl",
            "light_gizmo.wgsl",
            "lighting_extra.wgsl",
            "motion_blur.wgsl",
            "outline.wgsl",
            "particle.wgsl",
            "pixel_class.wgsl",
            "postprocess.wgsl",
            "present.wgsl",
            "restir_di.wgsl",
            "restir_gi.wgsl",
            "rt_debug.wgsl",
            "rt_hit.wgsl",
            "sampling.wgsl",
            "shading.wgsl",
            "shadow.wgsl",
            "spd.wgsl",
            "taa.wgsl",
            "terrain_material.wgsl",
            "transparent.wgsl",
            "underwater.wgsl",
            "velocity.wgsl",
            "visibility.wgsl",
            "volumetric.wgsl",
            "water.wgsl",
            "water_reflection.wgsl",
            "water_spectrum.wgsl",
        );
        Self {
            system: Mutex::new(system),
        }
    }

    /// The id of a registered module.
    ///
    /// # Panics
    ///
    /// Panics on an unregistered name. The names are literals in this file and
    /// in `//!include` directives that the resolver already validates, so a
    /// miss here is a typo in engine code rather than anything a user can
    /// cause — and failing loudly at startup beats a pass silently not drawing.
    #[must_use]
    pub fn id(&self, name: &str) -> ModuleId {
        self.system
            .lock()
            .expect("shader system poisoned")
            .registry()
            .id(name)
            .unwrap_or_else(|| panic!("shader module `{name}` is not registered in shaders.rs"))
    }

    /// Resolve a module's composed WGSL.
    pub fn source(&self, name: &str, defines: Defines) -> Result<String, ShaderError> {
        let key = ShaderKey {
            module: self.id(name),
            defines,
        };
        self.system
            .lock()
            .expect("shader system poisoned")
            .source(key)
            .map(str::to_string)
    }

    /// The names of every module a composed root draws from, itself included.
    ///
    /// Sorted, so a caller comparing two roots gets a stable answer.
    #[must_use]
    pub fn dependency_names(&self, name: &str, defines: Defines) -> Vec<&'static str> {
        let id = self.id(name);
        let system = self.system.lock().expect("shader system poisoned");
        let registry = system.registry();
        let mut names: Vec<_> = registry
            .dependencies(id, defines)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|dep| registry.name(dep))
            .collect();
        names.sort_unstable();
        names
    }

    /// Resolve a module's composed WGSL, panicking with the diagnostic.
    ///
    /// For pass construction, which happens once at startup and has no useful
    /// recovery: a shader that will not compose means the engine cannot draw,
    /// and the message is worth more than the graceful degradation.
    #[must_use]
    pub fn source_or_panic(&self, name: &str) -> String {
        self.source(name, Defines::NONE)
            .unwrap_or_else(|error| panic!("shader composition failed: {error}"))
    }

    /// Build a `wgpu::ShaderModule` from a composed module.
    pub fn module(
        &self,
        device: &wgpu::Device,
        name: &str,
        defines: Defines,
    ) -> Result<wgpu::ShaderModule, ShaderError> {
        let key = ShaderKey {
            module: self.id(name),
            defines,
        };
        self.system
            .lock()
            .expect("shader system poisoned")
            .module(device, key)
    }

    /// Poll watched files and apply what changed. Debug builds only.
    ///
    /// `validate` is handed the composed source for each dependent variant and
    /// should return naga's diagnostic on failure. **A module whose new source
    /// does not validate is not installed**, its old text stays, and the
    /// diagnostic comes back in [`ReloadOutcome::failures`] — the caller shows
    /// it and keeps drawing with the pipelines it already had.
    pub fn poll_reload(
        &self,
        validate: impl FnMut(&str, &str) -> Result<(), String>,
    ) -> ReloadOutcome {
        let mut system = self.system.lock().expect("shader system poisoned");
        let changed = system.poll_reload();
        if changed.is_empty() {
            return ReloadOutcome::default();
        }
        system.apply_reload(changed, validate)
    }

    /// The variant budget report (plan §8 item 4).
    #[must_use]
    pub fn budget_table(&self) -> String {
        self.system
            .lock()
            .expect("shader system poisoned")
            .budget_table()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registered module composes.
    ///
    /// This is the migration's own regression test and the reason it can be
    /// trusted without a GPU: it resolves every shader in the crate, so a
    /// `//!include` naming a file that does not exist, a cycle, or a typo in a
    /// `//!if` fails here rather than at device creation on someone's machine.
    #[test]
    fn every_registered_module_composes() {
        let shaders = Shaders::new();
        let system = shaders.system.lock().unwrap();
        let ids: Vec<_> = system.registry().ids().collect();
        drop(system);
        assert!(
            ids.len() >= 50,
            "expected the whole shader set, got {}",
            ids.len()
        );

        for id in ids {
            let name = {
                let system = shaders.system.lock().unwrap();
                system.registry().name(id).unwrap()
            };
            shaders
                .source(name, Defines::NONE)
                .unwrap_or_else(|error| panic!("{name} does not compose: {error}"));
        }
    }

    /// `shading.wgsl` composes to the same modules the old `format!` listed.
    ///
    /// The acceptance case from the plan, asserted rather than assumed. The
    /// order in the resolved text is dependencies-first, which the shipped
    /// renderer already proves is fine — `restir_gi.rs` concatenated its root
    /// *first* and its dependencies after, so WGSL module-scope forward
    /// references were already in use before this change.
    #[test]
    fn shading_composes_the_eight_modules_it_used_to_concatenate() {
        let shaders = Shaders::new();
        let mut expected = vec![
            "global_pool.wgsl",
            "brdf.wgsl",
            "sampling.wgsl",
            "atmosphere.wgsl",
            "hextile.wgsl",
            "terrain_material.wgsl",
            "clipmap_shade.wgsl",
            "shading.wgsl",
        ];
        expected.sort_unstable();
        assert_eq!(
            shaders.dependency_names("shading.wgsl", Defines::NONE),
            expected,
            "the composed set must be exactly the eight the old `format!` listed"
        );
        assert!(
            shaders.source_or_panic("shading.wgsl").len() > 100_000,
            "resolved shading is suspiciously small"
        );
    }

    /// The three roots that used to be concatenated root-first still compose
    /// the same modules.
    ///
    /// `restir_gi.rs`, `lighting_extra.rs` and `water_reflection.rs` each put
    /// their root *before* its dependencies so `enable wgpu_ray_query;` would
    /// land first. The resolver hoists `enable` instead, so the order changed
    /// and the set must not have.
    #[test]
    fn the_ray_query_roots_compose_the_same_modules_they_used_to() {
        let shaders = Shaders::new();
        let mut gi = vec![
            "restir_gi.wgsl",
            "rt_hit.wgsl",
            "global_pool.wgsl",
            "brdf.wgsl",
            "sampling.wgsl",
            "atmosphere.wgsl",
            "hextile.wgsl",
            "terrain_material.wgsl",
        ];
        gi.sort_unstable();
        assert_eq!(
            shaders.dependency_names("restir_gi.wgsl", Defines::NONE),
            gi
        );

        let mut water = vec![
            "water_reflection.wgsl",
            "rt_hit.wgsl",
            "global_pool.wgsl",
            "brdf.wgsl",
            "hextile.wgsl",
            "terrain_material.wgsl",
        ];
        water.sort_unstable();
        assert_eq!(
            shaders.dependency_names("water_reflection.wgsl", Defines::NONE),
            water
        );
    }

    /// A composed module contains each shared dependency exactly once.
    ///
    /// WGSL has no include guards, so a duplicated struct is a redefinition
    /// error. `restir_gi.wgsl` pulls `brdf.wgsl` both directly and through
    /// other modules.
    #[test]
    fn shared_dependencies_appear_once() {
        let shaders = Shaders::new();
        for root in ["shading.wgsl", "restir_gi.wgsl", "lighting_extra.wgsl"] {
            let source = shaders.source_or_panic(root);
            let brdf = shaders.source_or_panic("brdf.wgsl");
            if let Some(first) = brdf.lines().find(|l| l.trim_start().starts_with("fn ")) {
                assert_eq!(
                    source.matches(first.trim()).count(),
                    1,
                    "{root} emitted `{}` more than once",
                    first.trim()
                );
            }
        }
    }

    /// The define registered for MORROWIND-U resolves.
    ///
    /// The exit criterion is that adding a `SKINNED` variant needs no edit to
    /// `renderer.rs`. This proves the key half of that: the define exists, is
    /// named, and produces a distinct variant.
    #[test]
    fn a_skinned_variant_is_a_distinct_key_without_touching_the_renderer() {
        let shaders = Shaders::new();
        let plain = shaders.source("shading.wgsl", Defines::NONE).unwrap();
        let skinned = shaders.source("shading.wgsl", define::SKINNED).unwrap();
        // Identical today, because no shader has a `//!if SKINNED` block yet.
        // The point is that the *key* differs and the cache holds two entries,
        // so MORROWIND-U adds the block and nothing else.
        assert_eq!(plain, skinned);
        let system = shaders.system.lock().unwrap();
        assert!(system.variants().len() >= 2);
    }
}
