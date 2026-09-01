//! Named debug visualisations and pipeline switches — CONTROL-G.
//!
//! This lives in `somnium_ui` rather than in the renderer because it is
//! *editor-facing description*: menu ids, labels and Help lines for modes the
//! renderer already implements. The dependency edge runs renderer → ui, so
//! this is also the only side it can live on without inverting it. The
//! renderer reads [`DebugToggles`] for the live state; nothing here knows what
//! a pass is.
//!
//! Every one of these was already implemented and already reachable, provided
//! you knew a shell variable or a magic integer. The phase's complaint is not
//! that the visualisations are missing; it is that "type 24 into that field"
//! is the interface. This module is the table that fixes it, and it is
//! deliberately **all** this sub-phase costs on the renderer side: nothing
//! below changes what any pass does.
//!
//! Two kinds live here because they behave differently. A [`DebugView`] is a
//! *mode* — one at a time, mutually exclusive, selected by a code the shader
//! already understands. A [`Toggle`] is independent of every other toggle and
//! answers yes or no.

/// One named debug visualisation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugView {
    /// Stable id, and the suffix of the registry command that selects it.
    pub id: &'static str,
    /// Menu label.
    pub label: &'static str,
    /// The `shading_debug` code the shader branches on.
    pub code: f32,
    /// One line, shown in Help and as the menu tooltip.
    pub help: &'static str,
}

/// Every visualisation `shading.wgsl` implements, in code order.
///
/// The codes are the shader's, not a new numbering: renaming them here would
/// break every recorded repro in `dev records/` that names one.
pub const DEBUG_VIEWS: &[DebugView] = &[
    DebugView {
        id: "lit",
        label: "Lit",
        code: 0.0,
        help: "The ordinary shaded image.",
    },
    DebugView {
        id: "shadow_factor",
        label: "Shadow Factor",
        code: 1.0,
        help: "The scalar shadow term, before it multiplies anything.",
    },
    DebugView {
        id: "sun_only",
        label: "Sun Only",
        code: 2.0,
        help: "Direct sunlight alone, isolating where brightness comes from.",
    },
    DebugView {
        id: "ambient_only",
        label: "Ambient Only",
        code: 3.0,
        help: "Ambient light alone, isolating where brightness comes from.",
    },
    DebugView {
        id: "shadow_plumbing",
        label: "Shadow Plumbing",
        code: 4.0,
        help: "Red: cascade UV out of range. Green: a nearer depth in the atlas. Blue: correctly lit.",
    },
    DebugView {
        id: "blocker_search",
        label: "Blocker Search",
        code: 5.0,
        help: "Red: PCSS found no blocker. Green: it found one.",
    },
    DebugView {
        id: "shadow_hue",
        label: "Shadow Factor (hue)",
        code: 6.0,
        help: "The final shadow factor in hue, immune to exposure.",
    },
    DebugView {
        id: "dominant_term",
        label: "Dominant Light Term",
        code: 7.0,
        help: "Green: sun dominates. Red: ambient. Blue: the surface reads as metallic.",
    },
    DebugView {
        id: "occlusion",
        label: "Occlusion",
        code: 8.0,
        help: "The occlusion actually reaching the surface, greyscale.",
    },
    DebugView {
        id: "albedo",
        label: "Albedo",
        code: 9.0,
        help: "Base colour with no lighting.",
    },
    DebugView {
        id: "shading_normal",
        label: "Shading Normal",
        code: 10.0,
        help: "The normal used for shading.",
    },
    DebugView {
        id: "terrain_flag",
        label: "Terrain Flag",
        code: 11.0,
        help: "Terrain instances against everything else.",
    },
    DebugView {
        id: "terrain_taps",
        label: "Terrain Layer Taps",
        code: 12.0,
        help: "Texture taps as a fraction of the 36-tap worst case, written before exposure so the capture harness can average it.",
    },
    DebugView {
        id: "terrain_lod",
        label: "Terrain Chunk LOD",
        code: 13.0,
        help: "One colour per clipmap LOD; grey means a non-terrain instance.",
    },
    DebugView {
        id: "triangle_edges",
        label: "Triangle Edges",
        code: 14.0,
        help: "Analytic edges reconstructed from visibility barycentrics.",
    },
    DebugView {
        id: "geometric_normal",
        label: "Geometric Normal",
        code: 15.0,
        help: "The interpolated geometric normal.",
    },
    DebugView {
        id: "receiver_normal",
        label: "Receiver-bias Normal",
        code: 16.0,
        help: "The normal shadow receiver bias actually uses.",
    },
    DebugView {
        id: "contact_shadows",
        label: "Contact Shadows",
        code: 17.0,
        help: "The screen-space contact-shadow factor before cascade composition.",
    },
    DebugView {
        id: "splat_discarded",
        label: "Discarded Splat Weight",
        code: 18.0,
        help: "Splat weight thrown away by strongest-four selection.",
    },
    DebugView {
        id: "layer_indices",
        label: "Selected Layer Indices",
        code: 19.0,
        help: "The first three selected terrain layers, as colour.",
    },
    DebugView {
        id: "layer_weights",
        label: "Selected Layer Weights",
        code: 20.0,
        help: "Raw strongest-four weights of the first three selected layers.",
    },
    DebugView {
        id: "dominant_layer",
        label: "Dominant Layer Albedo",
        code: 21.0,
        help: "The strongest terrain layer, solo.",
    },
    DebugView {
        id: "cliff_blend",
        label: "Cliff Projection Blend",
        code: 22.0,
        help: "How much of the surface is shaded by the cliff projection.",
    },
    DebugView {
        id: "wetness",
        label: "Wetness",
        code: 23.0,
        help: "Moisture affinity times global wetness.",
    },
    DebugView {
        id: "luminance",
        label: "Luminance",
        code: 24.0,
        help: "Scene luminance on a log scale.",
    },
    DebugView {
        id: "restir_gi",
        label: "ReSTIR GI",
        code: 25.0,
        help: "The ReSTIR global-illumination contribution alone.",
    },
    DebugView {
        id: "cluster_heat",
        label: "Cluster Heat",
        code: 26.0,
        help: "Lights per cluster; blue is few, red is many.",
    },
    DebugView {
        id: "world_volume",
        label: "World Volume",
        code: 27.0,
        help: "The world-space irradiance volume sampled at this surface.",
    },
    DebugView {
        id: "lighting_aux",
        label: "Lighting Aux",
        code: 28.0,
        help: "The auxiliary lighting target, amplified.",
    },
    DebugView {
        id: "volume_alpha",
        label: "World Volume Alpha",
        code: 29.0,
        help: "Occupancy of the world volume.",
    },
    DebugView {
        id: "mip_level",
        label: "Texture Mip Level",
        code: 30.0,
        help: "Derivative-selected mip, as heat.",
    },
    DebugView {
        id: "lighting_aux_raw",
        label: "Lighting Aux (raw)",
        code: 31.0,
        help: "The auxiliary lighting target without amplification.",
    },
    DebugView {
        id: "clipmap_albedo",
        label: "Clipmap Albedo",
        code: 32.0,
        help: "The terrain clipmap cache's albedo; black off terrain, so a missed bind is obvious.",
    },
    DebugView {
        id: "clipmap_ring",
        label: "Clipmap Ring",
        code: 33.0,
        help: "Which clipmap ring a terrain pixel came from; 0 is finest.",
    },
];

/// Look up a view by id.
#[must_use]
pub fn debug_view(id: &str) -> Option<&'static DebugView> {
    DEBUG_VIEWS.iter().find(|view| view.id == id)
}

/// One independent renderer switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Toggle {
    /// Stable id, and the suffix of the registry command that flips it.
    pub id: &'static str,
    /// Menu label.
    pub label: &'static str,
    /// The variable that seeds it, and that overrides it when explicitly set.
    pub env: &'static str,
    /// One line, shown in Help and as the menu tooltip.
    pub help: &'static str,
}

/// Every pipeline switch that used to be reachable only from a shell.
pub const TOGGLES: &[Toggle] = &[
    Toggle {
        id: "meshlets",
        label: "Meshlet Draws",
        env: "SOMNIUM_NO_MESHLETS",
        help: "Draw through the meshlet path rather than whole-mesh draws.",
    },
    Toggle {
        id: "occlusion",
        label: "Occlusion Culling",
        env: "SOMNIUM_NO_OCCLUSION",
        help: "Two-phase Hi-Z occlusion culling.",
    },
    Toggle {
        id: "cascade_cull",
        label: "Shadow Cascade Culling",
        env: "SOMNIUM_CASCADE_CULL",
        help: "Cull draws per shadow cascade.",
    },
    Toggle {
        id: "spd",
        label: "Single-pass Downsample",
        env: "SOMNIUM_SPD",
        help: "Build the Hi-Z pyramid with SPD rather than a mip chain.",
    },
    Toggle {
        id: "aerial",
        label: "Aerial Terrain Pipeline",
        env: "SOMNIUM_AERIAL",
        help: "Shade distant terrain through the reduced aerial pipeline.",
    },
    Toggle {
        id: "aerial_hero",
        label: "Aerial Hero Bank",
        env: "SOMNIUM_AERIAL_HERO",
        help: "Cut the aerial pipeline's layer scan to the hero bank.",
    },
    Toggle {
        id: "cloud_jitter",
        label: "Cloud Ray Jitter",
        env: "SOMNIUM_CLOUD_JITTER",
        help: "Offset each cloud ray's start by blue noise. Measured both ways \u{2014} see CONTROL-M.",
    },
    Toggle {
        id: "dreams_grain",
        label: "DREAMS Shared Grain",
        env: "SOMNIUM_DREAMS_GRAIN",
        help: "Share the Slang-cooked temporal masks across GTAO, volumetrics, ReSTIR and TAA.",
    },
    Toggle {
        id: "dreams_stf",
        label: "DREAMS Terrain STF",
        env: "SOMNIUM_DREAMS_STF",
        help: "Use the shared masks for stochastic terrain mip filtering.",
    },
    Toggle {
        id: "hex_tiling",
        label: "Hex Tiling",
        env: "SOMNIUM_HEXTILE",
        help: "Break up terrain texture repetition with hex tiling.",
    },
    Toggle {
        id: "terrain_clipmap",
        label: "Terrain Clipmap Cache",
        env: "SOMNIUM_TERRAIN_CLIPMAP",
        help: "Cache terrain shading into clipmap rings.",
    },
    Toggle {
        id: "terrain_triplanar",
        label: "Terrain Triplanar",
        env: "SOMNIUM_TERRAIN_TRIPLANAR",
        help: "Project terrain textures triplanarly on steep ground.",
    },
    Toggle {
        id: "terrain_height_blend",
        label: "Terrain Height Blend",
        env: "SOMNIUM_TERRAIN_HEIGHT_BLEND",
        help: "Blend terrain layers by height rather than by weight alone.",
    },
    Toggle {
        id: "terrain_parallax",
        label: "Terrain Parallax",
        env: "SOMNIUM_TERRAIN_PARALLAX",
        help: "March a parallax offset through terrain relief.",
    },
    Toggle {
        id: "terrain_macro",
        label: "Terrain Macro Variation",
        env: "SOMNIUM_TERRAIN_MACRO",
        help: "Large-scale variation over terrain layers.",
    },
    Toggle {
        id: "terrain_detail_fade",
        label: "Terrain Detail Fade",
        env: "SOMNIUM_TERRAIN_DETAIL_FADE",
        help: "Fade terrain detail textures out with distance.",
    },
    Toggle {
        id: "terrain_lod_morph",
        label: "Terrain LOD Morph",
        env: "SOMNIUM_LOD_MORPH",
        help: "Morph terrain vertices between LODs instead of popping.",
    },
    Toggle {
        id: "rt_terrain",
        label: "Ray-traced Terrain",
        env: "SOMNIUM_RT_TERRAIN",
        help: "Include terrain in the ray-tracing acceleration structure.",
    },
    Toggle {
        id: "pixel_census",
        label: "Pixel Census",
        env: "SOMNIUM_CENSUS",
        help: "Count pixels per prospective shading bin.",
    },
    Toggle {
        id: "cull_stats",
        label: "Cull Statistics",
        env: "SOMNIUM_CULL_STATS",
        help: "Read indirect args back after each cull phase. Stalls the pipeline.",
    },
    Toggle {
        id: "shading_bins",
        label: "Shading Bin Routing",
        env: "SOMNIUM_SHADE_BINS",
        help: "Route tiles to per-bin shading pipelines.",
    },
    Toggle {
        id: "taa_debug",
        label: "TAA Debug Overlay",
        env: "SOMNIUM_TAA_DEBUG",
        help: "Visualise what temporal anti-aliasing rejected and why.",
    },
    Toggle {
        id: "taa_material_debug",
        label: "TAA Material Debug",
        env: "SOMNIUM_TAA_MATDBG",
        help: "Log TAA's material inputs for the first few frames.",
    },
    Toggle {
        id: "rt_debug",
        label: "Ray Tracing Debug",
        env: "SOMNIUM_RT_DEBUG",
        help: "Draw the ray-tracing pass's own diagnostic output.",
    },
];

/// Look up a toggle by id.
#[must_use]
pub fn toggle(id: &str) -> Option<&'static Toggle> {
    TOGGLES.iter().find(|toggle| toggle.id == id)
}

/// Live state for [`TOGGLES`], seeded from the environment.
///
/// The seeding is what keeps every recorded repro working: a variable that was
/// set still decides the initial value, and — because `overridden` remembers
/// that it was set — the menu entry is disabled and names it, exactly as
/// CONTROL-H's settings do. The editor and the shell therefore cannot disagree
/// about which one is in charge.
#[derive(Debug, Clone, Default)]
pub struct DebugToggles {
    on: std::collections::BTreeMap<&'static str, bool>,
    overridden: std::collections::BTreeSet<&'static str>,
}

impl DebugToggles {
    /// Seed from a variable reader. Injected rather than reading the process
    /// environment directly so the defaults are testable.
    #[must_use]
    pub fn seed(mut read: impl FnMut(&str) -> Option<String>) -> Self {
        let mut state = Self::default();
        for toggle in TOGGLES {
            let default = default_for(toggle.id);
            match read(toggle.env) {
                Some(raw) => {
                    state.on.insert(toggle.id, interpret(toggle.id, &raw));
                    state.overridden.insert(toggle.id);
                }
                None => {
                    state.on.insert(toggle.id, default);
                }
            }
        }
        state
    }

    /// Seed from the real process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::seed(|name| std::env::var(name).ok())
    }

    /// Whether a toggle is on. An unknown id reads as off rather than
    /// panicking: a stale command should do nothing, not crash the editor.
    #[must_use]
    pub fn is_on(&self, id: &str) -> bool {
        self.on.get(id).copied().unwrap_or(false)
    }

    /// The variable that took this toggle over, if one did.
    #[must_use]
    pub fn override_of(&self, id: &str) -> Option<&'static str> {
        self.overridden
            .contains(id)
            .then(|| toggle(id).map(|toggle| toggle.env))
            .flatten()
    }

    /// Flip a toggle. Refuses when the environment owns it, so the menu's
    /// disabled state and the actual behaviour cannot disagree.
    pub fn set(&mut self, id: &str, value: bool) -> Result<(), String> {
        if let Some(name) = self.override_of(id) {
            return Err(format!("overridden by {name}"));
        }
        if !self.on.contains_key(id) {
            return Err(format!("unknown toggle {id}"));
        }
        self.on.insert(
            TOGGLES
                .iter()
                .find(|toggle| toggle.id == id)
                .expect("checked above")
                .id,
            value,
        );
        Ok(())
    }
}

/// The value a toggle takes when its variable is unset.
///
/// These reproduce the shipped defaults exactly — several are "on unless the
/// variable says otherwise", which is why this is a table rather than `false`.
fn default_for(id: &str) -> bool {
    matches!(
        id,
        "meshlets"
            | "occlusion"
            | "cascade_cull"
            | "spd"
            | "terrain_height_blend"
            | "terrain_macro"
            | "terrain_detail_fade"
            | "terrain_clipmap"
            | "rt_terrain"
            | "shading_bins"
            | "dreams_grain"
            | "dreams_stf"
    )
}

/// How a variable's text becomes a boolean.
///
/// Two of them are negative switches — `SOMNIUM_NO_MESHLETS=1` means *off* —
/// and getting that backwards would silently invert a recorded repro, so the
/// inversion is named here rather than inferred.
fn interpret(id: &str, raw: &str) -> bool {
    let set = raw == "1";
    match id {
        "meshlets" | "occlusion" => !set,
        "cascade_cull"
        | "spd"
        | "terrain_height_blend"
        | "terrain_macro"
        | "terrain_detail_fade"
        | "rt_terrain"
        | "dreams_grain"
        | "dreams_stf" => raw != "0",
        _ => set,
    }
}

/// The viewport statistics overlay's contents.
///
/// Zeta-G's status bar reports "objects, because that is what it can state
/// honestly". These are the numbers it could not: they come off the frame the
/// renderer actually submitted, so they are what a regression shows up in
/// first rather than what the scene *contains*.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewportStats {
    /// Draw calls submitted.
    pub draw_calls: u32,
    /// Instances that survived culling.
    pub instances: u32,
    /// Triangles in the submitted draws.
    pub triangles: u32,
    /// Terrain chunks submitted.
    pub terrain_chunks: u32,
    /// Draws that survived shadow-caster culling.
    pub shadow_casters: u32,
    /// Render resolution in physical pixels, after dynamic scaling.
    pub resolution: (u32, u32),
    /// Dynamic-resolution scale, `1.0` when it is off.
    pub resolution_scale: f32,
    /// Bytes of GPU memory the renderer has allocated, when it can say.
    pub vram_bytes: u64,
    /// Where the camera is and which way it points.
    ///
    /// `(x, y, z, yaw, pitch)`, in the units `SOMNIUM_CAMERA_POS`,
    /// `SOMNIUM_CAMERA_YAW` and `SOMNIUM_CAMERA_PITCH` take, so a viewpoint
    /// somebody is looking at can be handed to a capture run verbatim. A bug
    /// that only appears from one place is otherwise reported as a screenshot
    /// and reproduced by guesswork, which is exactly as slow as it sounds.
    pub camera: Option<[f32; 5]>,
}

impl ViewportStats {
    /// The overlay's lines, longest-lived facts first.
    ///
    /// Rendered as text rather than a table because the overlay sits over the
    /// image and every pixel it covers is a pixel of the thing being judged;
    /// six short lines is the most it can honestly take.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let (width, height) = self.resolution;
        let mut lines = vec![
            format!(
                "{width} x {height}  ({:.0}%)",
                self.resolution_scale * 100.0
            ),
            format!("{} draws  {} instances", self.draw_calls, self.instances),
            format!("{} triangles", thousands(u64::from(self.triangles))),
        ];
        if self.terrain_chunks > 0 {
            lines.push(format!("{} terrain chunks", self.terrain_chunks));
        }
        if self.shadow_casters > 0 {
            lines.push(format!("{} shadow casters", self.shadow_casters));
        }
        if self.vram_bytes > 0 {
            lines.push(format!("{} VRAM", mebibytes(self.vram_bytes)));
        }
        if let Some([x, y, z, yaw, pitch]) = self.camera {
            // One line, in the order the environment variables take, so it can
            // be copied across without rearranging.
            lines.push(format!("cam {x:.0},{y:.0},{z:.0}  yaw {yaw:.0}  pitch {pitch:.0}"));
        }
        lines
    }
}

/// Group digits so a seven-figure triangle count is readable at a glance.
fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

fn mebibytes(bytes: u64) -> String {
    let mib = bytes as f64 / (1024.0 * 1024.0);
    if mib >= 1024.0 {
        format!("{:.2} GiB", mib / 1024.0)
    } else {
        format!("{mib:.0} MiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlay states what the frame submitted, and omits what it cannot
    /// honestly report rather than printing a zero that reads as a fact.
    #[test]
    fn the_overlay_omits_numbers_it_does_not_have() {
        let stats = ViewportStats {
            draw_calls: 12,
            instances: 12,
            triangles: 1_234_567,
            resolution: (1920, 1080),
            resolution_scale: 1.0,
            ..ViewportStats::default()
        };
        let lines = stats.lines();
        assert_eq!(lines[0], "1920 x 1080  (100%)");
        assert_eq!(lines[1], "12 draws  12 instances");
        assert_eq!(lines[2], "1 234 567 triangles");
        assert_eq!(lines.len(), 3, "absent counters produce no line at all");

        let stats = ViewportStats {
            terrain_chunks: 40,
            shadow_casters: 7,
            vram_bytes: 3 * 1024 * 1024 * 1024,
            ..stats
        };
        let lines = stats.lines();
        assert!(lines.iter().any(|line| line == "40 terrain chunks"));
        assert!(lines.iter().any(|line| line == "7 shadow casters"));
        assert!(lines.iter().any(|line| line == "3.00 GiB VRAM"));
    }

    #[test]
    fn a_scaled_resolution_says_so() {
        let stats = ViewportStats {
            resolution: (1280, 720),
            resolution_scale: 0.667,
            ..ViewportStats::default()
        };
        assert_eq!(stats.lines()[0], "1280 x 720  (67%)");
    }

    /// The magic integers get names, and the names address the codes the
    /// shader and every recorded repro already use.
    #[test]
    fn every_shader_debug_code_has_exactly_one_name() {
        let mut codes: Vec<_> = DEBUG_VIEWS.iter().map(|view| view.code as i32).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), DEBUG_VIEWS.len(), "codes must be unique");
        assert_eq!(codes.first(), Some(&0), "0 is the ordinary lit image");
        assert_eq!(
            codes.last(),
            Some(&33),
            "33 is the highest code shading.wgsl branches on"
        );
        for (index, code) in codes.iter().enumerate() {
            assert_eq!(*code, index as i32, "the code space must have no gaps");
        }
    }

    #[test]
    fn ids_and_labels_are_unique_and_documented() {
        for view in DEBUG_VIEWS {
            assert!(
                !view.help.trim().is_empty(),
                "{} needs a Help line",
                view.id
            );
            assert_eq!(
                DEBUG_VIEWS
                    .iter()
                    .filter(|other| other.id == view.id)
                    .count(),
                1
            );
        }
        for toggle in TOGGLES {
            assert!(
                !toggle.help.trim().is_empty(),
                "{} needs a Help line",
                toggle.id
            );
            assert!(toggle.env.starts_with("SOMNIUM_"));
        }
    }

    /// Unset variables reproduce the shipped defaults, so turning the menu on
    /// does not silently change what a default build renders.
    #[test]
    fn an_unset_environment_reproduces_the_shipped_defaults() {
        let toggles = DebugToggles::seed(|_| None);
        assert!(toggles.is_on("meshlets"));
        assert!(toggles.is_on("occlusion"));
        assert!(toggles.is_on("spd"));
        assert!(toggles.is_on("dreams_grain"));
        assert!(toggles.is_on("dreams_stf"));
        assert!(!toggles.is_on("aerial"));
        assert!(!toggles.is_on("hex_tiling"));
        assert!(!toggles.is_on("pixel_census"));
        assert!(toggles.overridden.is_empty());
    }

    /// The negative switches are the ones worth a test: `NO_MESHLETS=1` means
    /// meshlets are *off*, and inverting that would break a recorded repro.
    #[test]
    fn negative_switches_are_not_inverted_by_accident() {
        let toggles =
            DebugToggles::seed(|name| (name == "SOMNIUM_NO_MESHLETS").then(|| "1".to_string()));
        assert!(!toggles.is_on("meshlets"));

        let toggles =
            DebugToggles::seed(|name| (name == "SOMNIUM_CASCADE_CULL").then(|| "0".to_string()));
        assert!(!toggles.is_on("cascade_cull"));
    }

    /// Craft defect C8 again, in the renderer: an overridden toggle says which
    /// variable took it over and refuses the write.
    #[test]
    fn an_environment_override_disables_the_menu_entry() {
        let mut toggles =
            DebugToggles::seed(|name| (name == "SOMNIUM_HEXTILE").then(|| "1".to_string()));
        assert!(toggles.is_on("hex_tiling"));
        assert_eq!(toggles.override_of("hex_tiling"), Some("SOMNIUM_HEXTILE"));
        assert_eq!(
            toggles.set("hex_tiling", false),
            Err("overridden by SOMNIUM_HEXTILE".into())
        );
        assert!(
            toggles.is_on("hex_tiling"),
            "the refusal must not half-apply"
        );

        assert_eq!(toggles.override_of("aerial"), None);
        assert_eq!(toggles.set("aerial", true), Ok(()));
        assert!(toggles.is_on("aerial"));
    }

    #[test]
    fn an_unknown_toggle_is_inert_rather_than_fatal() {
        let mut toggles = DebugToggles::seed(|_| None);
        assert!(!toggles.is_on("no_such_toggle"));
        assert!(toggles.set("no_such_toggle", true).is_err());
    }
}
