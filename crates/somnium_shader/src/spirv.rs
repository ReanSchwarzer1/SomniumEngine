//! Trusted SPIR-V artifacts produced from authored Slang modules.
//!
//! Slang is a source language; the runtime consumes its checked-in SPIR-V
//! cook. Keeping the compiler out of `cargo build` makes ordinary builds
//! offline and reproducible, while `tools/slangcook` owns recooking and byte
//! comparison in the same way `tools/shadercook` owns WGSL variant budgets.

use std::borrow::Cow;

/// SPIR-V's little-endian magic word.
const SPIRV_MAGIC: u32 = 0x0723_0203;

/// One entry point declared by a passthrough shader module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpirvEntryPoint {
    /// Entry-point name, as emitted by Slang.
    pub name: &'static str,
    /// Compute workgroup size. Zeroes are accepted for non-compute stages.
    pub workgroup_size: (u32, u32, u32),
}

impl SpirvEntryPoint {
    /// Describe an entry point.
    #[must_use]
    pub const fn new(name: &'static str, workgroup_size: (u32, u32, u32)) -> Self {
        Self {
            name,
            workgroup_size,
        }
    }
}

/// A validated, renderer-authored SPIR-V artifact.
#[derive(Clone, Debug)]
pub(crate) struct SpirvArtifact {
    words: Vec<u32>,
    entry_points: Vec<SpirvEntryPoint>,
}

impl SpirvArtifact {
    pub(crate) fn parse(bytes: &[u8], entry_points: &[SpirvEntryPoint]) -> Result<Self, String> {
        if bytes.len() < 20 || !bytes.len().is_multiple_of(4) {
            return Err(format!(
                "SPIR-V artifact is {} bytes; expected a header and whole 32-bit words",
                bytes.len()
            ));
        }
        if entry_points.is_empty() {
            return Err("SPIR-V passthrough modules must declare an entry point".into());
        }
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes(word.try_into().expect("four-byte chunk")))
            .collect();
        if words[0] != SPIRV_MAGIC {
            return Err(format!(
                "SPIR-V artifact has magic 0x{:08x}, expected 0x{SPIRV_MAGIC:08x}",
                words[0]
            ));
        }
        if entry_points.iter().any(|entry| entry.name.is_empty()) {
            return Err("SPIR-V entry-point names cannot be empty".into());
        }
        Ok(Self {
            words,
            entry_points: entry_points.to_vec(),
        })
    }

    pub(crate) fn words(&self) -> &[u32] {
        &self.words
    }

    pub(crate) fn create_module(
        &self,
        device: &wgpu::Device,
        label: &'static str,
    ) -> Result<wgpu::ShaderModule, String> {
        if !device
            .features()
            .contains(wgpu::Features::PASSTHROUGH_SHADERS)
        {
            return Err(format!(
                "{label} is a Slang/SPIR-V module, but this adapter does not expose PASSTHROUGH_SHADERS"
            ));
        }
        let entry_points = self
            .entry_points
            .iter()
            .map(|entry| wgpu::PassthroughShaderEntryPoint {
                name: Cow::Borrowed(entry.name),
                workgroup_size: entry.workgroup_size,
            })
            .collect();

        // SAFETY: this is the only passthrough seam for Somnium-authored
        // shaders. `SpirvArtifact::parse` rejects malformed containers and
        // `tools/slangcook/run.py --check` recompiles every checked-in artifact
        // with the pinned Slang compiler and compares the bytes before it can
        // land. Sources are repository-authored, never user supplied. The
        // backend still owns semantic validation, which is why the feature is
        // capability-gated and failure remains a Result at this interface.
        Ok(unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some(label),
                entry_points: Cow::Owned(entry_points),
                spirv: Some(Cow::Borrowed(&self.words)),
                dxil: None,
                msl: None,
                hlsl: None,
                glsl: None,
                wgsl: None,
                metallib: None,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn malformed_or_unnamed_passthrough_modules_are_refused() {
        assert!(
            SpirvArtifact::parse(&[0; 19], &[SpirvEntryPoint::new("main", (1, 1, 1))]).is_err()
        );
        assert!(
            SpirvArtifact::parse(&bytes(&[0; 5]), &[SpirvEntryPoint::new("main", (1, 1, 1))])
                .is_err()
        );
        assert!(SpirvArtifact::parse(&bytes(&[SPIRV_MAGIC; 5]), &[]).is_err());
        assert!(
            SpirvArtifact::parse(
                &bytes(&[SPIRV_MAGIC; 5]),
                &[SpirvEntryPoint::new("", (1, 1, 1))]
            )
            .is_err()
        );
    }
}
