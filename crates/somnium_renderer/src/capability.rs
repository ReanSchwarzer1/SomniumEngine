//! The wgpu 30 capability report (Phase MORROWIND, MORROWIND-A2).
//!
//! # Why this exists
//!
//! wgpu 30's changelog claims mesh shaders are "fully supported on Vulkan".
//! Two of the feature names involved still begin with `EXPERIMENTAL_`, and a
//! changelog is a claim rather than a measurement. MORROWIND-A2's rule is
//! **probe, do not trust**: the bump adds no feature, and the only thing it
//! leaves behind besides a version number is a checked-in record of what this
//! hardware actually grants.
//!
//! Three later sub-phases read that record rather than re-deriving it:
//!
//! - **MORROWIND-U** (skinning) needs to know whether
//!   [`ACCELERATION_STRUCTURE_BINDING_ARRAY`] is real before choosing between
//!   skin-to-buffer and skin-in-shader, because the second rebuilds the BLAS.
//! - **MORROWIND-Z / AA / AD** want [`EXPERIMENTAL_MESH_SHADER`], which is the
//!   native expression of what `meshlet.rs` plus `cull.wgsl` currently emulate.
//! - Anything wanting a wave-level optimisation needs the **subgroup sizes**,
//!   which wgpu 30 moved out of `Limits` and onto [`wgpu::AdapterInfo`].
//!
//! # How to record one
//!
//! The report is logged at startup at `info` level. To check one in:
//!
//! ```text
//! SOMNIUM_CAPABILITY_REPORT="dev records/phase MORROWIND/MORROWIND-A2_capabilities.md" \
//!   cargo run -p hello_engine
//! ```
//!
//! [`ACCELERATION_STRUCTURE_BINDING_ARRAY`]: wgpu::Features::ACCELERATION_STRUCTURE_BINDING_ARRAY
//! [`EXPERIMENTAL_MESH_SHADER`]: wgpu::Features::EXPERIMENTAL_MESH_SHADER

use std::fmt::Write as _;

/// One probed capability and what depends on it.
struct Probe {
    name: &'static str,
    feature: wgpu::Features,
    /// The sub-phase or system that is waiting on this bit. A capability with
    /// no named consumer does not belong in the report.
    wanted_by: &'static str,
}

/// Everything MORROWIND-A2 §8 item 2 names, plus the two Somnium already
/// depends on, so the report is a complete picture rather than a wishlist.
const PROBES: &[Probe] = &[
    Probe {
        name: "EXPERIMENTAL_MESH_SHADER",
        feature: wgpu::Features::EXPERIMENTAL_MESH_SHADER,
        wanted_by: "Track 7 — the native form of meshlet.rs + cull.wgsl's emulation",
    },
    Probe {
        name: "EXPERIMENTAL_MESH_SHADER_MULTIVIEW",
        feature: wgpu::Features::EXPERIMENTAL_MESH_SHADER_MULTIVIEW,
        wanted_by: "MORROWIND-Z — one mesh dispatch across shadow cascades",
    },
    Probe {
        name: "MULTI_DRAW_INDIRECT_COUNT",
        feature: wgpu::Features::MULTI_DRAW_INDIRECT_COUNT,
        wanted_by: "GPU-driven culling — draw count from the GPU, not a readback",
    },
    Probe {
        name: "ACCELERATION_STRUCTURE_BINDING_ARRAY",
        feature: wgpu::Features::ACCELERATION_STRUCTURE_BINDING_ARRAY,
        wanted_by: "MORROWIND-U — decides skin-to-buffer vs skin-in-shader (plan A.5)",
    },
    Probe {
        name: "EXPERIMENTAL_RAY_HIT_VERTEX_RETURN",
        feature: wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN,
        wanted_by: "MORROWIND-AB — hit attributes without a second pool fetch",
    },
    Probe {
        name: "EXPERIMENTAL_RAY_TRACING_PIPELINES",
        feature: wgpu::Features::EXPERIMENTAL_RAY_TRACING_PIPELINES,
        wanted_by: "not planned — recorded so the gap is visible",
    },
    Probe {
        name: "SHADER_I16",
        feature: wgpu::Features::SHADER_I16,
        wanted_by: "MORROWIND-W2 — quantised pose data without widening",
    },
    Probe {
        name: "SUBGROUP",
        feature: wgpu::Features::SUBGROUP,
        wanted_by: "Track 7 — wave-level reductions in cull and classify",
    },
    Probe {
        name: "SUBGROUP_BARRIER",
        feature: wgpu::Features::SUBGROUP_BARRIER,
        wanted_by: "Track 7 — as above, where a barrier is needed",
    },
    Probe {
        name: "EXPERIMENTAL_RAY_QUERY",
        feature: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
        wanted_by: "shipped — ReSTIR GI and water reflections degrade without it",
    },
    Probe {
        name: "TEXTURE_BINDING_ARRAY",
        feature: wgpu::Features::TEXTURE_BINDING_ARRAY,
        wanted_by: "shipped — the bindless resource pool; required, not optional",
    },
    Probe {
        name: "TEXTURE_COMPRESSION_BC",
        feature: wgpu::Features::TEXTURE_COMPRESSION_BC,
        wanted_by: "shipped — terrain BC7 packs; RGBA8 otherwise",
    },
    Probe {
        name: "PIPELINE_STATISTICS_QUERY",
        feature: wgpu::Features::PIPELINE_STATISTICS_QUERY,
        wanted_by: "shipped — the profiler's \"why\" beside its \"how long\"",
    },
];

/// A rendered capability report for one adapter.
pub struct CapabilityReport {
    /// Markdown, ready to write beside a phase record.
    pub markdown: String,
    /// One line, for the startup log.
    pub summary: String,
}

/// Probe `adapter` and render the report.
///
/// Note that this reports what the **adapter** offers, not what the device was
/// created with. Somnium's rule everywhere is *detect, do not demand* — a
/// feature is requested only when the adapter has it — so a granted feature is
/// always a subset of this table, never a superset.
pub fn probe(adapter: &wgpu::Adapter) -> CapabilityReport {
    let info = adapter.get_info();
    let features = adapter.features();
    let limits = adapter.limits();

    let mut md = String::new();
    let _ = writeln!(md, "# MORROWIND-A2 — wgpu 30 capability report");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "Generated by `somnium_renderer::capability::probe`. **Measured on one \
         machine.** Another GPU will produce a different table, and that is the \
         point: the wgpu changelog is a claim about what wgpu supports, not about \
         what this driver grants."
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "## Adapter");
    let _ = writeln!(md);
    let _ = writeln!(md, "| Field | Value |");
    let _ = writeln!(md, "|---|---|");
    let _ = writeln!(md, "| Name | {} |", info.name);
    let _ = writeln!(md, "| Backend | {:?} |", info.backend);
    let _ = writeln!(md, "| Device type | {:?} |", info.device_type);
    let _ = writeln!(md, "| Vendor / device | {:#06x} / {:#06x} |", info.vendor, info.device);
    let _ = writeln!(md, "| Driver | {} {} |", info.driver, info.driver_info);
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "**Subgroup size: {}–{}.** wgpu 30 moved these from `Limits` to \
         `AdapterInfo`; the plan (§6.9.1) predicted that move and it is the one \
         predicted breaking change that turned out to be real for Somnium — \
         though only in the abstract, since the tree contains no `subgroup_*` \
         reference to break.",
        info.subgroup_min_size, info.subgroup_max_size
    );
    let _ = writeln!(md);

    let _ = writeln!(md, "## Features");
    let _ = writeln!(md);
    let _ = writeln!(md, "| Feature | Granted | Wanted by |");
    let _ = writeln!(md, "|---|:---:|---|");
    let mut granted = 0usize;
    for probe in PROBES {
        let have = features.contains(probe.feature);
        granted += usize::from(have);
        let _ = writeln!(
            md,
            "| `{}` | {} | {} |",
            probe.name,
            if have { "**yes**" } else { "no" },
            probe.wanted_by
        );
    }
    let _ = writeln!(md);
    let _ = writeln!(md, "{granted} of {} probed features granted.", PROBES.len());
    let _ = writeln!(md);

    let _ = writeln!(md, "## Limits that bound a MORROWIND design");
    let _ = writeln!(md);
    let _ = writeln!(md, "| Limit | Value |");
    let _ = writeln!(md, "|---|---:|");
    let _ = writeln!(md, "| `max_binding_array_elements_per_shader_stage` | {} |", limits.max_binding_array_elements_per_shader_stage);
    let _ = writeln!(md, "| `max_storage_buffers_per_shader_stage` | {} |", limits.max_storage_buffers_per_shader_stage);
    let _ = writeln!(md, "| `max_storage_textures_per_shader_stage` | {} |", limits.max_storage_textures_per_shader_stage);
    let _ = writeln!(md, "| `max_compute_workgroup_storage_size` | {} |", limits.max_compute_workgroup_storage_size);
    let _ = writeln!(md, "| `max_compute_invocations_per_workgroup` | {} |", limits.max_compute_invocations_per_workgroup);
    let _ = writeln!(md, "| `max_buffer_size` | {} |", limits.max_buffer_size);
    let _ = writeln!(md, "| `max_texture_dimension_2d` | {} |", limits.max_texture_dimension_2d);
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "`max_binding_array_elements_per_shader_stage` is the one to watch: it \
         is the ceiling on the bindless pool, and Track 4's residency budget \
         and Track 1's UI texture array both spend from it."
    );

    let summary = format!(
        "wgpu 30 capabilities: {granted}/{} features on {} ({:?}), subgroups {}–{}",
        PROBES.len(),
        info.name,
        info.backend,
        info.subgroup_min_size,
        info.subgroup_max_size
    );

    CapabilityReport {
        markdown: md,
        summary,
    }
}

/// Log the report, and write it to `SOMNIUM_CAPABILITY_REPORT` when set.
///
/// A failed write is a warning, not a panic: this is diagnostics, and an
/// unwritable path must never stop the engine from starting.
pub fn report(adapter: &wgpu::Adapter) {
    let report = probe(adapter);
    tracing::info!("{}", report.summary);

    let Some(path) = std::env::var_os("SOMNIUM_CAPABILITY_REPORT") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    match std::fs::write(&path, &report.markdown) {
        Ok(()) => tracing::info!("capability report written to {}", path.display()),
        Err(error) => tracing::warn!(
            "capability report write to {} failed: {error}",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every probe names a consumer.
    ///
    /// The failure mode this catches is a report that grows into a dump of
    /// every feature wgpu has. A capability nobody is waiting for is noise, and
    /// noise is what makes a report stop being read.
    #[test]
    fn every_probe_names_who_wants_it() {
        for probe in PROBES {
            assert!(
                !probe.wanted_by.is_empty(),
                "{} has no named consumer",
                probe.name
            );
            assert!(
                probe.wanted_by.len() > 12,
                "{}: \"{}\" is too short to be a reason",
                probe.name,
                probe.wanted_by
            );
        }
    }

    /// No feature is probed twice.
    #[test]
    fn probes_are_distinct() {
        for (i, a) in PROBES.iter().enumerate() {
            for b in &PROBES[i + 1..] {
                assert_ne!(a.name, b.name, "duplicate probe name");
                assert_ne!(a.feature, b.feature, "{} and {} probe the same bit", a.name, b.name);
            }
        }
    }

    /// The features Somnium already requires are in the table.
    ///
    /// A report that only lists the things MORROWIND wants would let a
    /// *regression* in a shipped requirement pass unnoticed on new hardware.
    #[test]
    fn shipped_requirements_are_probed() {
        for required in [
            wgpu::Features::TEXTURE_BINDING_ARRAY,
            wgpu::Features::EXPERIMENTAL_RAY_QUERY,
            wgpu::Features::TEXTURE_COMPRESSION_BC,
        ] {
            assert!(
                PROBES.iter().any(|p| p.feature == required),
                "{required:?} is depended on but not probed"
            );
        }
    }
}
