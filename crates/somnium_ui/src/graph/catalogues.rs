//! Built-in MORROWIND-K catalogues.
//!
//! These are deliberately only data. Material evaluation belongs to the
//! material compiler and animation evaluation belongs to `somnium_anim`; the
//! shared surface merely needs enough information to construct and validate
//! either graph.

use super::{
    Catalogue, GroupArchetype, NodeArchetype, NodeElementArchetype, PinArchetype, PinType,
};

/// The first MORROWIND-K consumer: a material graph layered over `.sommat`.
#[must_use]
pub fn material() -> Catalogue {
    const ADD_KEYWORDS: &[&str] = &["plus"];
    const MULTIPLY_KEYWORDS: &[&str] = &["times"];

    let mut catalogue = Catalogue::new("somnium.material");
    catalogue
        .register_group(GroupArchetype::new("inputs", "Inputs", 0))
        .register_group(GroupArchetype::new("math", "Math", 10))
        .register_group(GroupArchetype::new("layout", "Layout", 90))
        .register_group(GroupArchetype::new("output", "Output", 100));
    register_layout(&mut catalogue);

    catalogue.register(
        NodeArchetype::new("material.scalar", "Scalar", "Input")
            .in_group("inputs")
            .with_keywords(&["float", "number", "constant"])
            .with_input(PinArchetype::new("Value", PinType::Float).with_default("0.0"))
            .with_output(PinArchetype::new("Value", PinType::Float))
            .with_element(NodeElementArchetype::Literal(0)),
    );
    catalogue.register(
        NodeArchetype::new("material.color", "Colour", "Input")
            .in_group("inputs")
            .with_keywords(&["color", "constant", "rgb"])
            .with_input(PinArchetype::new("Value", PinType::Color).with_default("1.0,1.0,1.0,1.0"))
            .with_output(PinArchetype::new("Value", PinType::Color))
            .with_element(NodeElementArchetype::Literal(0)),
    );
    catalogue.register(
        NodeArchetype::new("material.texture", "Texture Sample", "Input")
            .in_group("inputs")
            .with_keywords(&["image", "sample", "uv"])
            .with_input(PinArchetype::new("Texture", PinType::Texture))
            .with_input(PinArchetype::new("UV", PinType::Vec2).with_default("0.0,0.0"))
            .with_output(PinArchetype::new("RGBA", PinType::Color)),
    );
    for (id, title, keywords) in [
        ("material.add", "Add", ADD_KEYWORDS),
        ("material.multiply", "Multiply", MULTIPLY_KEYWORDS),
    ] {
        catalogue.register(
            NodeArchetype::new(id, title, "Math")
                .in_group("math")
                .with_keywords(keywords)
                .with_input(PinArchetype::new("A", PinType::Float).with_default("0.0"))
                .with_input(PinArchetype::new("B", PinType::Float).with_default("0.0"))
                .with_output(PinArchetype::new("Value", PinType::Float)),
        );
    }
    catalogue.register(
        NodeArchetype::new("material.reroute.float", "Reroute", "Layout")
            .in_group("math")
            .with_input(PinArchetype::new("In", PinType::Float))
            .with_output(PinArchetype::new("Out", PinType::Float))
            .as_reroute(),
    );
    catalogue.register(
        NodeArchetype::new("material.surface", "Material Surface", "Output")
            .in_group("output")
            .with_input(
                PinArchetype::new("Base Colour", PinType::Color).with_default("1.0,1.0,1.0,1.0"),
            )
            .with_input(PinArchetype::new("Roughness", PinType::Float).with_default("0.5"))
            .with_input(PinArchetype::new("Metallic", PinType::Float).with_default("0.0"))
            .with_input(PinArchetype::new("Normal", PinType::Vec3).with_default("0.0,0.0,1.0"))
            .as_root(),
    );
    catalogue
}

/// A non-material proof that the surface is a framework rather than a shader
/// editor. MORROWIND-V can extend this catalogue without changing the model,
/// geometry, serialisation, or palette.
#[must_use]
pub fn animation() -> Catalogue {
    const POSE: PinType = PinType::Opaque("animation.pose");
    const PARAMETER: PinType = PinType::Opaque("animation.parameter");
    const SYNC_TRACK: PinType = PinType::Opaque("animation.sync_track");
    const BONE_MASK: PinType = PinType::Opaque("animation.bone_mask");
    const TRIANGLES: PinType = PinType::Opaque("animation.triangles");

    let mut catalogue = Catalogue::new("somnium.animation");
    catalogue
        .register_group(GroupArchetype::new("sources", "Sources", 0))
        .register_group(GroupArchetype::new("blend", "Blend", 10))
        .register_group(GroupArchetype::new("state", "State Machine", 20))
        .register_group(GroupArchetype::new("layout", "Layout", 90))
        .register_group(GroupArchetype::new("output", "Output", 100));
    register_layout(&mut catalogue);

    catalogue.register(
        NodeArchetype::new("animation.clip", "Clip", "Source")
            .in_group("sources")
            .with_keywords(&["animation", "sample", "play"])
            .with_input(
                PinArchetype::new("Clip Id", PinType::Int)
                    .with_default("0")
                    .with_tooltip("Animation clip asset identifier"),
            )
            .with_input(
                PinArchetype::new("Time Scale", PinType::Float)
                    .with_default("1.0")
                    .with_unit("×")
                    .with_tooltip("Playback-rate multiplier"),
            )
            .with_input(
                PinArchetype::new("Loop", PinType::Bool)
                    .with_default("true")
                    .with_tooltip("Repeat after reaching the clip duration"),
            )
            .with_output(PinArchetype::new("Pose", POSE))
            .with_element(NodeElementArchetype::Literal(0))
            .with_element(NodeElementArchetype::Literal(1))
            .with_element(NodeElementArchetype::Literal(2)),
    );
    catalogue.register(
        NodeArchetype::new("animation.blend1d", "Blend 1D", "Blend")
            .in_group("blend")
            .with_keywords(&["lerp", "locomotion"])
            .with_input(PinArchetype::new("A", POSE))
            .with_input(PinArchetype::new("B", POSE))
            .with_input(PinArchetype::new("A Position", PinType::Float).with_default("0.0"))
            .with_input(PinArchetype::new("B Position", PinType::Float).with_default("1.0"))
            .with_input(
                PinArchetype::new("Parameter", PARAMETER)
                    .with_default("speed")
                    .with_tooltip("Float parameter sampled along the blend axis"),
            )
            .with_input(
                PinArchetype::new("Sync Track", SYNC_TRACK)
                    .with_default("")
                    .with_tooltip("Optional marker track used to phase-align samples"),
            )
            .with_input(
                PinArchetype::new("Sync Leader", PinType::Int)
                    .with_default("0")
                    .with_range("0", "1")
                    .with_tooltip("Sample index that supplies the stable sync phase"),
            )
            .with_output(PinArchetype::new("Pose", POSE))
            .with_element(NodeElementArchetype::Literal(2))
            .with_element(NodeElementArchetype::Literal(3))
            .with_element(NodeElementArchetype::Literal(4))
            .with_element(NodeElementArchetype::Literal(5))
            .with_element(NodeElementArchetype::Literal(6)),
    );
    catalogue.register(
        NodeArchetype::new("animation.blend1d3", "Blend 1D (3 samples)", "Blend")
            .in_group("blend")
            .with_keywords(&["locomotion", "idle", "walk", "run"])
            .with_input(PinArchetype::new("A", POSE))
            .with_input(PinArchetype::new("B", POSE))
            .with_input(PinArchetype::new("C", POSE))
            .with_input(PinArchetype::new("A Position", PinType::Float).with_default("0.0"))
            .with_input(PinArchetype::new("B Position", PinType::Float).with_default("0.5"))
            .with_input(PinArchetype::new("C Position", PinType::Float).with_default("1.0"))
            .with_input(
                PinArchetype::new("Parameter", PARAMETER)
                    .with_default("speed")
                    .with_tooltip("Float parameter sampled along the blend axis"),
            )
            .with_input(
                PinArchetype::new("Sync Track", SYNC_TRACK)
                    .with_default("")
                    .with_tooltip("Optional marker track used to phase-align samples"),
            )
            .with_input(
                PinArchetype::new("Sync Leader", PinType::Int)
                    .with_default("0")
                    .with_range("0", "2")
                    .with_tooltip("Sample index that supplies the stable sync phase"),
            )
            .with_output(PinArchetype::new("Pose", POSE))
            .with_element(NodeElementArchetype::Literal(3))
            .with_element(NodeElementArchetype::Literal(4))
            .with_element(NodeElementArchetype::Literal(5))
            .with_element(NodeElementArchetype::Literal(6))
            .with_element(NodeElementArchetype::Literal(7))
            .with_element(NodeElementArchetype::Literal(8)),
    );
    catalogue.register(
        NodeArchetype::new("animation.blend2d", "Blend 2D", "Blend")
            .in_group("blend")
            .with_keywords(&["direction", "triangulation", "locomotion"])
            .with_input(PinArchetype::new("A", POSE))
            .with_input(PinArchetype::new("B", POSE))
            .with_input(PinArchetype::new("C", POSE))
            .with_input(PinArchetype::new("A X", PinType::Float).with_default("0.0"))
            .with_input(PinArchetype::new("A Y", PinType::Float).with_default("0.0"))
            .with_input(PinArchetype::new("B X", PinType::Float).with_default("1.0"))
            .with_input(PinArchetype::new("B Y", PinType::Float).with_default("0.0"))
            .with_input(PinArchetype::new("C X", PinType::Float).with_default("0.0"))
            .with_input(PinArchetype::new("C Y", PinType::Float).with_default("1.0"))
            .with_input(PinArchetype::new("X Parameter", PARAMETER).with_default("x"))
            .with_input(PinArchetype::new("Y Parameter", PARAMETER).with_default("y"))
            .with_input(PinArchetype::new("Sync Track", SYNC_TRACK).with_default(""))
            .with_input(
                PinArchetype::new("Sync Leader", PinType::Int)
                    .with_default("0")
                    .with_range("0", "2")
                    .with_tooltip("Sample index that supplies the stable sync phase"),
            )
            .with_output(PinArchetype::new("Pose", POSE))
            .with_element(NodeElementArchetype::Literal(3))
            .with_element(NodeElementArchetype::Literal(4))
            .with_element(NodeElementArchetype::Literal(5))
            .with_element(NodeElementArchetype::Literal(6))
            .with_element(NodeElementArchetype::Literal(7))
            .with_element(NodeElementArchetype::Literal(8))
            .with_element(NodeElementArchetype::Literal(9))
            .with_element(NodeElementArchetype::Literal(10))
            .with_element(NodeElementArchetype::Literal(11))
            .with_element(NodeElementArchetype::Literal(12)),
    );
    catalogue.register(
        NodeArchetype::new("animation.blend2d4", "Blend 2D (4 samples)", "Blend")
            .in_group("blend")
            .with_keywords(&["direction", "triangulation", "multi triangle"])
            .with_input(PinArchetype::new("A", POSE))
            .with_input(PinArchetype::new("B", POSE))
            .with_input(PinArchetype::new("C", POSE))
            .with_input(PinArchetype::new("D", POSE))
            .with_input(PinArchetype::new("A X", PinType::Float).with_default("-1.0"))
            .with_input(PinArchetype::new("A Y", PinType::Float).with_default("-1.0"))
            .with_input(PinArchetype::new("B X", PinType::Float).with_default("1.0"))
            .with_input(PinArchetype::new("B Y", PinType::Float).with_default("-1.0"))
            .with_input(PinArchetype::new("C X", PinType::Float).with_default("1.0"))
            .with_input(PinArchetype::new("C Y", PinType::Float).with_default("1.0"))
            .with_input(PinArchetype::new("D X", PinType::Float).with_default("-1.0"))
            .with_input(PinArchetype::new("D Y", PinType::Float).with_default("1.0"))
            .with_input(PinArchetype::new("X Parameter", PARAMETER).with_default("x"))
            .with_input(PinArchetype::new("Y Parameter", PARAMETER).with_default("y"))
            .with_input(
                PinArchetype::new("Triangles", TRIANGLES)
                    .with_default("0,1,2;0,2,3")
                    .with_tooltip("Authored triangle indices separated by semicolons"),
            )
            .with_input(
                PinArchetype::new("Sync Track", SYNC_TRACK)
                    .with_default("")
                    .with_tooltip("Optional marker track used to phase-align samples"),
            )
            .with_input(
                PinArchetype::new("Sync Leader", PinType::Int)
                    .with_default("0")
                    .with_range("0", "3")
                    .with_tooltip("Sample index that supplies the stable sync phase"),
            )
            .with_output(PinArchetype::new("Pose", POSE))
            .with_element(NodeElementArchetype::Literal(4))
            .with_element(NodeElementArchetype::Literal(5))
            .with_element(NodeElementArchetype::Literal(6))
            .with_element(NodeElementArchetype::Literal(7))
            .with_element(NodeElementArchetype::Literal(8))
            .with_element(NodeElementArchetype::Literal(9))
            .with_element(NodeElementArchetype::Literal(10))
            .with_element(NodeElementArchetype::Literal(11))
            .with_element(NodeElementArchetype::Literal(12))
            .with_element(NodeElementArchetype::Literal(13))
            .with_element(NodeElementArchetype::Literal(14))
            .with_element(NodeElementArchetype::Literal(15))
            .with_element(NodeElementArchetype::Literal(16)),
    );
    catalogue.register(
        NodeArchetype::new("animation.layer", "Layer", "Blend")
            .in_group("blend")
            .with_keywords(&["mask", "additive", "overlay"])
            .with_input(PinArchetype::new("Base", POSE))
            .with_input(PinArchetype::new("Overlay", POSE))
            .with_input(
                PinArchetype::new("Weight", PinType::Float)
                    .with_default("1.0")
                    .with_range("0", "1")
                    .with_tooltip("Constant layer contribution when no weight parameter is named"),
            )
            .with_input(
                PinArchetype::new("Weight Parameter", PARAMETER)
                    .with_default("")
                    .with_tooltip("Optional float parameter that drives layer contribution"),
            )
            .with_input(
                PinArchetype::new("Bone Mask", BONE_MASK)
                    .with_default("")
                    .with_tooltip("Comma-separated per-bone weights in skeleton order"),
            )
            .with_output(PinArchetype::new("Pose", POSE))
            .with_element(NodeElementArchetype::Literal(2))
            .with_element(NodeElementArchetype::Literal(3))
            .with_element(NodeElementArchetype::Literal(4)),
    );
    catalogue.register(
        NodeArchetype::new("animation.cache", "Pose Cache", "Blend")
            .in_group("blend")
            .with_keywords(&["memoize", "reuse"])
            .with_input(PinArchetype::new("Pose", POSE))
            .with_output(PinArchetype::new("Pose", POSE)),
    );
    catalogue.register(
        NodeArchetype::new("animation.state", "State", "State Machine")
            .in_group("state")
            .with_keywords(&["transition", "machine", "absm"])
            .with_input(PinArchetype::new("Pose", POSE))
            .with_input(
                PinArchetype::new("State Id", PinType::Int)
                    .with_default("0")
                    .with_range("0", "65535")
                    .with_tooltip("Durable machine-local state identifier"),
            )
            .with_element(NodeElementArchetype::Input(0))
            .with_element(NodeElementArchetype::Literal(1))
            .with_element(NodeElementArchetype::Label(
                "Alt-click: initial state; Shift-drag: add transition",
            )),
    );
    catalogue.register(
        NodeArchetype::new("animation.reroute.pose", "Reroute", "Layout")
            .in_group("blend")
            .with_input(PinArchetype::new("In", POSE))
            .with_output(PinArchetype::new("Out", POSE))
            .as_reroute(),
    );
    catalogue.register(
        NodeArchetype::new("animation.output", "Animation Output", "Output")
            .in_group("output")
            .with_input(PinArchetype::new("Pose", POSE))
            .as_root(),
    );
    catalogue
}

fn register_layout(catalogue: &mut Catalogue) {
    catalogue.register(
        NodeArchetype::new("graph.comment", "Comment", "Layout")
            .in_group("layout")
            .with_keywords(&["note", "annotation"])
            .with_element(NodeElementArchetype::Label("Annotation")),
    );
    catalogue.register(
        NodeArchetype::new("graph.group", "Group", "Layout")
            .in_group("layout")
            .with_keywords(&["frame", "section"])
            .with_element(NodeElementArchetype::Label(
                "Move the frame to move its members",
            )),
    );
}
