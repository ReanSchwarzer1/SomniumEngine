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

    let mut catalogue = Catalogue::new("somnium.animation");
    catalogue
        .register_group(GroupArchetype::new("sources", "Sources", 0))
        .register_group(GroupArchetype::new("blend", "Blend", 10))
        .register_group(GroupArchetype::new("layout", "Layout", 90))
        .register_group(GroupArchetype::new("output", "Output", 100));
    register_layout(&mut catalogue);

    catalogue.register(
        NodeArchetype::new("animation.clip", "Clip", "Source")
            .in_group("sources")
            .with_keywords(&["animation", "sample", "play"])
            .with_input(PinArchetype::new("Time", PinType::Float).with_default("0.0"))
            .with_output(PinArchetype::new("Pose", POSE)),
    );
    catalogue.register(
        NodeArchetype::new("animation.blend1d", "Blend 1D", "Blend")
            .in_group("blend")
            .with_keywords(&["lerp", "locomotion"])
            .with_input(PinArchetype::new("A", POSE))
            .with_input(PinArchetype::new("B", POSE))
            .with_input(PinArchetype::new("Weight", PinType::Float).with_default("0.5"))
            .with_output(PinArchetype::new("Pose", POSE)),
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
