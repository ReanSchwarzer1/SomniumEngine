//! The pixel census (Phase DOOM-B).
//!
//! DOOM-A answered "where does the frame go" — `Shading` is 25.8 ms of a 38.4 ms
//! Coastal ground frame, at exactly one fragment invocation per pixel. What it
//! could not answer is *which* pixels, and that is the question every later
//! DOOM sub-phase depends on: a tile-classified shading pass (DOOM-C) is only
//! worth building if the screen actually divides into classes with different
//! costs, and an aerial terrain bin (DOOM-E) is only worth building if distant
//! terrain is a real share rather than a few pixels on the horizon.
//!
//! Every conclusion drawn about Coastal so far has rested on the sentence
//! "almost every pixel is ground" (`terrain_shading_occupancy_2026-08-14.md`).
//! This counts them.
//!
//! # Cost
//!
//! One compute dispatch over the visibility buffer, reduced through group-shared
//! counters so the global atomic traffic is per workgroup rather than per pixel.
//! It is bracketed by the profiler like any other pass, so if it ever stops
//! being cheap that shows up in the same table as everything else. Default
//! **off** — a measurement nobody is reading is a measurement not worth paying
//! for, which is the same argument §17.7 makes for the profiler's own toggle.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Counters written by `census.wgsl`. The order is the shader's `BIN_*`
/// constants and the two must not drift.
pub const BIN_COUNT: usize = 7;

/// Human-readable names, indexed by bin.
pub const BIN_NAMES: [&str; BIN_COUNT] = [
    "sky",
    "mesh",
    "foliage",
    "terrain_near",
    "terrain_mid",
    "terrain_far",
    "total",
];

/// Readback slots in flight. Same reasoning as the profiler's ring: reading the
/// counters in the frame that wrote them would stall on the GPU and change the
/// frame being measured.
const RING: usize = 3;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CensusParams {
    width: u32,
    height: u32,
    near_split: f32,
    far_split: f32,
}

struct Slot {
    readback: wgpu::Buffer,
    ready: Arc<AtomicBool>,
    in_flight: bool,
}

/// Per-bin pixel counts from the most recently collected frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CensusResult {
    pub counts: [u32; BIN_COUNT],
}

impl CensusResult {
    /// Share of counted pixels in `bin`, as a percentage.
    #[must_use]
    pub fn pct(&self, bin: usize) -> f32 {
        let total = self.counts[BIN_COUNT - 1];
        if total == 0 || bin >= BIN_COUNT {
            return 0.0;
        }
        self.counts[bin] as f32 / total as f32 * 100.0
    }

    /// Every terrain bin together.
    #[must_use]
    pub fn terrain(&self) -> u32 {
        self.counts[3] + self.counts[4] + self.counts[5]
    }

    /// Report lines, for the profiler overlay and the headless log.
    #[must_use]
    pub fn report(&self) -> Vec<String> {
        if self.counts[BIN_COUNT - 1] == 0 {
            return vec!["census: waiting for the first readback…".to_string()];
        }
        let mut out = vec!["census (pixels)".to_string()];
        for (i, name) in BIN_NAMES.iter().enumerate().take(BIN_COUNT - 1) {
            out.push(format!(
                "  {:<24} {:>10} {:>6.2}%",
                name,
                self.counts[i],
                self.pct(i)
            ));
        }
        out.push(format!(
            "  {:<24} {:>10}",
            "total",
            self.counts[BIN_COUNT - 1]
        ));
        out
    }
}

pub struct CensusPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    counters: wgpu::Buffer,
    params: wgpu::Buffer,
    slots: Vec<Slot>,
    next_slot: usize,
    bind_group: Option<wgpu::BindGroup>,
    width: u32,
    height: u32,
    /// Camera distance splitting the three terrain bins.
    pub near_split: f32,
    pub far_split: f32,
    pub enabled: bool,
    pub result: CensusResult,
    scratch: Arc<std::sync::Mutex<[u32; BIN_COUNT]>>,
}

impl CensusPass {
    const BYTES: u64 = (BIN_COUNT * std::mem::size_of::<u32>()) as u64;

    pub fn new(device: &wgpu::Device, global_layout: &wgpu::BindGroupLayout) -> Self {
        let source = format!(
            "{}\n{}\n{}",
            include_str!("../shaders/global_pool.wgsl"),
            include_str!("../shaders/pixel_class.wgsl"),
            include_str!("../shaders/census.wgsl"),
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("census.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Census BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Census PL"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Census"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let counters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Census counters"),
            size: Self::BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Census params"),
            size: std::mem::size_of::<CensusParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let slots = (0..RING)
            .map(|i| Slot {
                readback: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("Census readback {i}")),
                    size: Self::BYTES,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ready: Arc::new(AtomicBool::new(false)),
                in_flight: false,
            })
            .collect();

        Self {
            pipeline,
            layout,
            counters,
            params,
            slots,
            next_slot: 0,
            bind_group: None,
            width: 0,
            height: 0,
            // 100 m and 400 m: near is roughly where XV's hex tiling and POM
            // still resolve on the ground, far is past where any of it survives
            // a 1 km tile. Both are starting points for DOOM-E to sweep, not
            // contracts.
            near_split: 100.0,
            far_split: 400.0,
            enabled: std::env::var("SOMNIUM_CENSUS").as_deref() == Ok("1"),
            result: CensusResult::default(),
            scratch: Arc::new(std::sync::Mutex::new([0; BIN_COUNT])),
        }
    }

    /// Drop the cached bind group. Call whenever the visibility buffer or depth
    /// texture is recreated — a resize keeps the size the same in the
    /// window-restored case but still hands out new views, and a bind group
    /// holding the old ones reads a destroyed texture.
    pub fn invalidate(&mut self) {
        self.bind_group = None;
    }

    /// Rebuild the bind group when the vis buffer or depth view changes.
    pub fn ensure_bind_group(
        &mut self,
        device: &wgpu::Device,
        vis_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        if !self.enabled {
            return;
        }
        if self.bind_group.is_some() && self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Census BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vis_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.counters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
            ],
        }));
    }

    /// Dispatch the census. Records nothing when disabled.
    pub fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        global_bind_group: &wgpu::BindGroup,
    ) {
        if !self.enabled || self.width == 0 || self.height == 0 {
            return;
        }
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&CensusParams {
                width: self.width,
                height: self.height,
                near_split: self.near_split,
                far_split: self.far_split,
            }),
        );
        // Zeroed every frame: these are per-frame counts, and a buffer that
        // accumulated across frames would report a number that only grows and
        // whose percentages happen to still look right.
        encoder.clear_buffer(&self.counters, 0, None);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Census"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, global_bind_group, &[]);
            pass.set_bind_group(1, bind_group, &[]);
            pass.dispatch_workgroups(self.width.div_ceil(16), self.height.div_ceil(16), 1);
        }

        // Copy into whichever ring slot is free. If none is, this frame simply
        // is not read back — the previous count is still on screen and one
        // skipped sample is cheaper than a stall.
        if let Some(i) = self.slots.iter().position(|s| !s.in_flight) {
            encoder.copy_buffer_to_buffer(
                &self.counters,
                0,
                &self.slots[i].readback,
                0,
                Self::BYTES,
            );
            self.next_slot = i;
        } else {
            self.next_slot = usize::MAX;
        }
    }

    /// Start the read for the slot filled by [`CensusPass::record`]. Must be
    /// called after the submit that contains it.
    pub fn after_submit(&mut self) {
        if !self.enabled || self.next_slot == usize::MAX {
            return;
        }
        let i = self.next_slot;
        let ready = Arc::clone(&self.slots[i].ready);
        ready.store(false, Ordering::Release);
        self.slots[i].in_flight = true;
        self.slots[i]
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| {
                if res.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
    }

    /// Harvest any slot whose callback has fired. Cheap; call once a frame.
    pub fn collect(&mut self) {
        if !self.enabled {
            return;
        }
        for i in 0..self.slots.len() {
            if !self.slots[i].in_flight || !self.slots[i].ready.load(Ordering::Acquire) {
                continue;
            }
            {
                let view = self.slots[i].readback.slice(..).get_mapped_range();
                let mut counts = self.scratch.lock().expect("census scratch");
                for (bin, chunk) in view.chunks_exact(4).take(BIN_COUNT).enumerate() {
                    counts[bin] = u32::from_le_bytes(chunk.try_into().expect("4 bytes"));
                }
                self.result.counts = *counts;
            }
            self.slots[i].readback.unmap();
            self.slots[i].ready.store(false, Ordering::Release);
            self.slots[i].in_flight = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_are_taken_against_the_total_not_the_sum_of_shown_bins() {
        // The total is counted by the shader rather than summed here, so a
        // classification bug that dropped pixels would show as bins that do not
        // add to 100 — which is the point of carrying it.
        let r = CensusResult {
            counts: [25, 25, 0, 40, 0, 0, 100],
        };
        assert!((r.pct(0) - 25.0).abs() < 1e-3);
        assert!((r.pct(3) - 40.0).abs() < 1e-3);
        let shown: f32 = (0..BIN_COUNT - 1).map(|i| r.pct(i)).sum();
        assert!(shown < 100.0, "{shown} — 10 pixels are unaccounted for");
    }

    #[test]
    fn an_empty_census_reports_zero_rather_than_dividing_by_it() {
        let r = CensusResult::default();
        assert_eq!(r.pct(0), 0.0);
        assert!(r.report()[0].contains("waiting"));
    }

    #[test]
    fn terrain_is_the_three_distance_bins_together() {
        let r = CensusResult {
            counts: [10, 10, 10, 20, 30, 40, 120],
        };
        assert_eq!(r.terrain(), 90);
    }

    #[test]
    fn the_report_names_every_bin_the_shader_writes() {
        let r = CensusResult {
            counts: [1, 1, 1, 1, 1, 1, 6],
        };
        let text = r.report().join("\n");
        for name in BIN_NAMES {
            assert!(text.contains(name), "missing {name} in\n{text}");
        }
    }
}
