//! Narrow animated glTF intake shared by games and editor previews. Bake LINEAR keys;
//! other interpolation is rejected rather than silently changing authored motion.
use crate::{LoadedScene, Vertex};
use glam::{Mat4, Quat, Vec3};
use somnium_anim::{AnimationClip, ClipId, Keyframe, Pose, Skeleton, TransformTrack};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("asset intake: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_fixture_imports_and_deforms_through_named_clip() {
        let asset = AnimatedAsset::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models/animation_demo.gltf"),
        )
        .unwrap();
        assert_eq!(asset.skeleton.len(), 2);
        let clip = &asset.clips["Bend"];
        let first = clip
            .sample(&asset.skeleton, 0.0, somnium_anim::Playback::ONCE)
            .unwrap();
        let second = clip
            .sample(&asset.skeleton, 1.0, somnium_anim::Playback::ONCE)
            .unwrap();
        let mut a = vec![vec![]];
        let mut b = vec![vec![]];
        asset.deform(&first, &mut a).unwrap();
        asset.deform(&second, &mut b).unwrap();
        assert_eq!(a[0].len(), 252);
        assert!(
            a[0].iter()
                .zip(&b[0])
                .any(|(a, b)| (Vec3::from(a.position) - Vec3::from(b.position)).length() > 0.5)
        );
        assert!(asset.masked_indices(0, &["tip"]).len() < asset.scene.meshes[0].indices.len());
        assert_eq!(asset.masked_indices(0, &[]), asset.scene.meshes[0].indices);
    }
}

pub struct AnimatedAsset {
    pub scene: LoadedScene,
    pub skeleton: Skeleton,
    pub clips: BTreeMap<String, AnimationClip>,
    joints: BTreeMap<String, u16>,
}

impl AnimatedAsset {
    pub fn load(path: &Path) -> Result<Self, AssetError> {
        let scene = crate::load_gltf(path).map_err(AssetError::Invalid)?;
        if scene.skeletons.len() != 1 {
            return Err(AssetError::Invalid(
                "animated asset needs exactly one skeleton".into(),
            ));
        }
        let skeleton = scene.skeletons[0].clone();
        let (document, buffers, _) =
            gltf::import(path).map_err(|e| AssetError::Invalid(e.to_string()))?;
        let mut joints = BTreeMap::new();
        for (index, node) in document
            .skins()
            .next()
            .ok_or_else(|| AssetError::Invalid("missing skin".into()))?
            .joints()
            .enumerate()
        {
            let name = node
                .name()
                .ok_or_else(|| AssetError::Invalid("unnamed joint".into()))?;
            let joint = skeleton.find(&format!("{name}#{index}")).ok_or_else(|| {
                AssetError::Invalid(format!("missing imported joint {name}#{index}"))
            })?;
            if joints.insert(name.to_owned(), joint).is_some() {
                return Err(AssetError::Invalid(format!("ambiguous joint {name}")));
            }
        }
        let mut clips = BTreeMap::new();
        for animation in document.animations() {
            let mut tracks = BTreeMap::<u16, TransformTrack>::new();
            let mut duration = 0.0_f32;
            for channel in animation.channels() {
                let node = channel.target().node();
                let Some(joint) = node.name().and_then(|name| joints.get(name).copied()) else {
                    continue;
                };
                if channel.sampler().interpolation() == gltf::animation::Interpolation::CubicSpline
                {
                    return Err(AssetError::Invalid(format!(
                        "{} must be baked to LINEAR",
                        animation.name().unwrap_or("unnamed")
                    )));
                }
                let reader = channel.reader(|buffer| Some(&buffers[buffer.index()].0));
                let times: Vec<_> = reader
                    .read_inputs()
                    .ok_or_else(|| AssetError::Invalid("missing animation time".into()))?
                    .collect();
                duration = duration.max(times.last().copied().unwrap_or(0.0));
                let track = tracks.entry(joint).or_insert_with(|| TransformTrack {
                    joint,
                    ..Default::default()
                });
                match reader
                    .read_outputs()
                    .ok_or_else(|| AssetError::Invalid("missing animation values".into()))?
                {
                    gltf::animation::util::ReadOutputs::Translations(values) => {
                        track.translation = times
                            .into_iter()
                            .zip(values)
                            .map(|(t, v)| Keyframe::new(t, Vec3::from_array(v)))
                            .collect()
                    }
                    gltf::animation::util::ReadOutputs::Rotations(values) => {
                        track.rotation = times
                            .into_iter()
                            .zip(values.into_f32())
                            .map(|(t, v)| Keyframe::new(t, Quat::from_array(v).normalize()))
                            .collect()
                    }
                    gltf::animation::util::ReadOutputs::Scales(values) => {
                        track.scale = times
                            .into_iter()
                            .zip(values)
                            .map(|(t, v)| {
                                let v = Vec3::from_array(v);
                                Keyframe::new(
                                    t,
                                    if (v - Vec3::ONE).abs().max_element() < 0.0001 {
                                        Vec3::ONE
                                    } else {
                                        v
                                    },
                                )
                            })
                            .collect()
                    }
                    gltf::animation::util::ReadOutputs::MorphTargetWeights(_) => {
                        return Err(AssetError::Invalid(
                            "morph animation is outside this intake".into(),
                        ));
                    }
                }
                if channel.sampler().interpolation() == gltf::animation::Interpolation::Step {
                    let constant = match channel.target().property() {
                        gltf::animation::Property::Translation => track
                            .translation
                            .windows(2)
                            .all(|w| w[0].value == w[1].value),
                        gltf::animation::Property::Rotation => {
                            track.rotation.windows(2).all(|w| w[0].value == w[1].value)
                        }
                        gltf::animation::Property::Scale => {
                            track.scale.windows(2).all(|w| w[0].value == w[1].value)
                        }
                        _ => false,
                    };
                    if !constant {
                        return Err(AssetError::Invalid(
                            "Nonconstant STEP track must be baked before intake".into(),
                        ));
                    }
                }
            }
            let name = animation
                .name()
                .ok_or_else(|| AssetError::Invalid("unnamed clip".into()))?
                .to_owned();
            let clip = AnimationClip::new(
                ClipId(animation.index() as u64),
                &skeleton,
                duration,
                tracks.into_values().collect(),
                vec![],
            )
            .map_err(|e| AssetError::Invalid(format!("clip {name}: {e:?}")))?;
            if clips.insert(name.clone(), clip).is_some() {
                return Err(AssetError::Invalid(format!("duplicate clip {name}")));
            }
        }
        for mesh in &scene.meshes {
            if mesh
                .skin
                .as_ref()
                .is_none_or(|s| s.bindings.len() != mesh.vertices.len())
            {
                return Err(AssetError::Invalid("unskinned or mismatched mesh".into()));
            }
        }
        Ok(Self {
            scene,
            skeleton,
            clips,
            joints,
        })
    }

    pub fn joint(&self, name: &str) -> Option<u16> {
        self.joints.get(name).copied()
    }

    pub fn deform(&self, pose: &Pose, output: &mut [Vec<Vertex>]) -> Result<(), AssetError> {
        let mut palette = vec![Mat4::IDENTITY; self.skeleton.len()];
        if !pose.to_palette(&self.skeleton, &mut palette) {
            return Err(AssetError::Invalid("pose palette mismatch".into()));
        }
        let normal_palette: Vec<_> = palette
            .iter()
            .map(|matrix| matrix.inverse().transpose())
            .collect();
        if output.len() != self.scene.meshes.len() {
            return Err(AssetError::Invalid(
                "deformation mesh count mismatch".into(),
            ));
        }
        for (mesh, out) in self.scene.meshes.iter().zip(output) {
            let skin = mesh
                .skin
                .as_ref()
                .ok_or_else(|| AssetError::Invalid("missing skin".into()))?;
            out.clear();
            for (vertex, binding) in mesh.vertices.iter().zip(&skin.bindings) {
                let mut position = Vec3::ZERO;
                let mut normal = Vec3::ZERO;
                for (&joint, &weight) in binding.joints.iter().zip(&binding.weights) {
                    if weight <= 0.0 {
                        continue;
                    }
                    let matrix = palette[joint as usize];
                    position += matrix.transform_point3(Vec3::from_array(vertex.position)) * weight;
                    normal += normal_palette[joint as usize]
                        .transform_vector3(Vec3::from_array(vertex.normal))
                        * weight;
                }
                if !position.is_finite() || !normal.is_finite() {
                    return Err(AssetError::Invalid("non-finite deformed vertex".into()));
                }
                out.push(Vertex {
                    position: position.to_array(),
                    normal: normal.normalize_or_zero().to_array(),
                    uv: vertex.uv,
                });
            }
        }
        Ok(())
    }

    pub fn masked_indices(&self, mesh_index: usize, hidden_joints: &[&str]) -> Vec<u32> {
        let mesh = &self.scene.meshes[mesh_index];
        let Some(skin) = &mesh.skin else {
            return mesh.indices.clone();
        };
        let joints: Vec<_> = hidden_joints
            .iter()
            .filter_map(|name| self.joint(name))
            .collect();
        mesh.indices
            .chunks_exact(3)
            .filter(|tri| {
                !tri.iter().any(|&index| {
                    let binding = &skin.bindings[index as usize];
                    binding
                        .joints
                        .iter()
                        .zip(binding.weights)
                        .filter(|(joint, _)| joints.contains(joint))
                        .map(|(_, w)| w)
                        .sum::<f32>()
                        > 0.5
                })
            })
            .flatten()
            .copied()
            .collect()
    }
}
