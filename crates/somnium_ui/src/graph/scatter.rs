//! MORROWIND-P2 adapter: the shared graph surface authors scatter rules.

use super::{
    Catalogue, Graph, GraphSurface, Node, NodeArchetype, NodeElementArchetype, NodeId,
    PinArchetype, PinRef, PinType,
};
use glam::{Vec2, Vec3};
use somnium_asset::scatter::{Gradient, ScatterRule};

const GRADIENT: PinType = PinType::Opaque("scatter.gradient");

/// All source/filter/distribution authoring uses the existing graph widget,
/// literals, typed wiring, history and versioned document serialization.
#[must_use]
pub fn catalogue() -> Catalogue {
    let mut catalogue = Catalogue::new("somnium.scatter");
    let mut add = |id: &'static str,
                   title: &'static str,
                   group: &'static str,
                   inputs: &[(&'static str, PinType, &'static str)],
                   root: bool| {
        let mut node = NodeArchetype::new(id, title, group);
        for (index, (name, kind, default)) in inputs.iter().enumerate() {
            node = node.with_input(PinArchetype::new(name, *kind).with_default(default));
            if *kind != GRADIENT {
                node = node.with_element(NodeElementArchetype::Literal(index as u16));
            }
        }
        node = if root {
            node.as_root()
        } else {
            node.with_output(PinArchetype::new("Weight", GRADIENT))
        };
        catalogue.register(node);
    };
    use PinType::{Float, Int, Vec3 as Vector};
    add(
        "scatter.constant",
        "Constant",
        "Sources",
        &[("Value", Float, "1")],
        false,
    );
    add(
        "scatter.noise",
        "Noise",
        "Sources",
        &[("Seed", Int, "1"), ("Frequency", Float, "0.1")],
        false,
    );
    add(
        "scatter.image",
        "Image",
        "Sources",
        &[("Image asset", PinType::Texture, "mask.png")],
        false,
    );
    add(
        "scatter.altitude",
        "Altitude",
        "Sources",
        &[
            ("Minimum metres", Float, "0"),
            ("Maximum metres", Float, "100"),
        ],
        false,
    );
    add(
        "scatter.slope",
        "Slope",
        "Sources",
        &[
            ("Minimum degrees", Float, "0"),
            ("Maximum degrees", Float, "90"),
        ],
        false,
    );
    add(
        "scatter.distance",
        "Distance",
        "Sources",
        &[("Center", Vector, "0,0,0"), ("Radius", Float, "10")],
        false,
    );
    add(
        "scatter.shape",
        "Shape Volume",
        "Sources",
        &[
            ("Minimum", Vector, "-10,-10,-10"),
            ("Maximum", Vector, "10,10,10"),
        ],
        false,
    );
    add(
        "scatter.tag",
        "Surface Tag",
        "Filters",
        &[
            ("Tag", PinType::Opaque("scatter.tag"), "ground"),
            ("Minimum weight", Float, "0.5"),
        ],
        false,
    );
    add(
        "scatter.multiply",
        "Multiply",
        "Composition",
        &[("A", GRADIENT, ""), ("B", GRADIENT, "")],
        false,
    );
    add(
        "scatter.invert",
        "Invert",
        "Composition",
        &[("Source", GRADIENT, "")],
        false,
    );
    add(
        "scatter.filter",
        "Distribution Filter",
        "Filters",
        &[
            ("Source", GRADIENT, ""),
            ("Minimum", Float, "0.3"),
            ("Maximum", Float, "1"),
        ],
        false,
    );
    add(
        "scatter.exclude",
        "Exclusion Volume",
        "Filters",
        &[
            ("Source", GRADIENT, ""),
            ("Minimum", Vector, "-2,-100,-2"),
            ("Maximum", Vector, "2,100,2"),
        ],
        false,
    );
    add(
        "scatter.output",
        "Scatter Output",
        "Output",
        &[
            ("Density weight", GRADIENT, ""),
            ("Seed", Int, "1"),
            ("Spacing metres", Float, "2"),
            ("Density per m²", Float, "0.25"),
            ("Jitter", Float, "1"),
            ("Minimum scale", Float, "0.75"),
            ("Maximum scale", Float, "1.25"),
            ("Instance limit", Int, "10000"),
        ],
        true,
    );
    catalogue
}

/// An immediately evaluable starter graph, also used by the native shell.
#[must_use]
pub fn default_surface() -> GraphSurface {
    let mut surface = GraphSurface::new(catalogue());
    let source = surface
        .add("scatter.noise", Vec2::new(20.0, 40.0))
        .expect("registered source");
    let filter = surface
        .add("scatter.filter", Vec2::new(270.0, 40.0))
        .expect("registered filter");
    let output = surface
        .add("scatter.output", Vec2::new(520.0, 40.0))
        .expect("registered output");
    surface
        .connect(PinRef::output(source, 0), PinRef::input(filter, 0))
        .expect("typed source");
    surface
        .connect(PinRef::output(filter, 0), PinRef::input(output, 0))
        .expect("typed output");
    surface
}

/// Compile the authored graph at the runtime seam. Only one output is valid;
/// missing inputs, unsupported parameter wires and oversized expansion fail
/// visibly instead of silently producing a different scatter.
pub fn compile(graph: &Graph) -> Result<ScatterRule, String> {
    let catalogue = catalogue();
    let roots: Vec<_> = graph
        .nodes()
        .iter()
        .filter(|n| n.archetype == "scatter.output")
        .collect();
    if roots.len() != 1 {
        return Err("Scatter graph requires exactly one Scatter Output".into());
    }
    let root = roots[0];
    let mut budget = 4096;
    let gradient = source(graph, &catalogue, root.id, 0, 0, &mut budget)?;
    let rule = ScatterRule {
        gradient,
        seed: number(graph, root, &catalogue, 1)?,
        spacing: number(graph, root, &catalogue, 2)?,
        density: number(graph, root, &catalogue, 3)?,
        jitter: number(graph, root, &catalogue, 4)?,
        scale_min: number(graph, root, &catalogue, 5)?,
        scale_max: number(graph, root, &catalogue, 6)?,
        max_instances: number(graph, root, &catalogue, 7)?,
    };
    rule.validate()?;
    Ok(rule)
}

fn source(
    graph: &Graph,
    catalogue: &Catalogue,
    owner: NodeId,
    pin: u16,
    depth: usize,
    budget: &mut usize,
) -> Result<Gradient, String> {
    if depth > 128 || *budget == 0 {
        return Err("Scatter graph exceeds compilation budget".into());
    }
    *budget -= 1;
    let wire = graph
        .input_source(PinRef::input(owner, pin))
        .ok_or_else(|| format!("Node {} input {pin} needs a gradient", owner.0))?;
    if wire.index != 0 {
        return Err("Scatter gradient output must use pin zero".into());
    }
    let node = graph.node(wire.node).ok_or("Scatter node is missing")?;
    let value = |pin| number::<f32>(graph, node, catalogue, pin);
    Ok(match node.archetype.as_str() {
        "scatter.constant" => Gradient::Constant(value(0)?),
        "scatter.noise" => Gradient::Noise {
            seed: number(graph, node, catalogue, 0)?,
            frequency: value(1)?,
        },
        "scatter.image" => Gradient::Image {
            asset: literal(graph, node, catalogue, 0)?.into(),
        },
        "scatter.altitude" => Gradient::Altitude {
            min: value(0)?,
            max: value(1)?,
        },
        "scatter.slope" => Gradient::Slope {
            min: value(0)?,
            max: value(1)?,
        },
        "scatter.distance" => Gradient::Distance {
            center: vector(graph, node, catalogue, 0)?,
            radius: value(1)?,
        },
        "scatter.shape" => Gradient::Shape {
            min: vector(graph, node, catalogue, 0)?,
            max: vector(graph, node, catalogue, 1)?,
        },
        "scatter.tag" => Gradient::SurfaceTag {
            name: literal(graph, node, catalogue, 0)?.into(),
            minimum: value(1)?,
        },
        "scatter.multiply" => Gradient::Multiply(
            Box::new(source(graph, catalogue, node.id, 0, depth + 1, budget)?),
            Box::new(source(graph, catalogue, node.id, 1, depth + 1, budget)?),
        ),
        "scatter.invert" => Gradient::Invert(Box::new(source(
            graph,
            catalogue,
            node.id,
            0,
            depth + 1,
            budget,
        )?)),
        "scatter.filter" => Gradient::Filter {
            source: Box::new(source(graph, catalogue, node.id, 0, depth + 1, budget)?),
            min: value(1)?,
            max: value(2)?,
        },
        "scatter.exclude" => Gradient::Exclude {
            source: Box::new(source(graph, catalogue, node.id, 0, depth + 1, budget)?),
            min: vector(graph, node, catalogue, 1)?,
            max: vector(graph, node, catalogue, 2)?,
        },
        other => return Err(format!("Unsupported scatter node: {other}")),
    })
}

fn literal<'a>(
    graph: &Graph,
    node: &'a Node,
    catalogue: &'a Catalogue,
    pin: u16,
) -> Result<&'a str, String> {
    if graph.input_source(PinRef::input(node.id, pin)).is_some() {
        return Err(format!(
            "Node {} parameter {pin} requires an authored literal",
            node.id.0
        ));
    }
    node.literals
        .get(&pin)
        .map(String::as_str)
        .or_else(|| {
            catalogue
                .get(&node.archetype)?
                .inputs
                .get(pin as usize)?
                .default
        })
        .ok_or_else(|| format!("Node {} parameter {pin} is missing", node.id.0))
}
fn number<T: std::str::FromStr>(
    graph: &Graph,
    node: &Node,
    catalogue: &Catalogue,
    pin: u16,
) -> Result<T, String> {
    literal(graph, node, catalogue, pin)?
        .trim()
        .parse()
        .map_err(|_| format!("Node {} parameter {pin} is invalid", node.id.0))
}
fn vector(graph: &Graph, node: &Node, catalogue: &Catalogue, pin: u16) -> Result<Vec3, String> {
    let values = literal(graph, node, catalogue, pin)?
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Expected x,y,z")?;
    let [x, y, z] = values.as_slice() else {
        return Err("Expected exactly three vector coordinates".into());
    };
    Ok(Vec3::new(*x, *y, *z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_asset::scatter::SurfacePoint;
    use std::collections::BTreeMap;

    #[test]
    fn author_save_reload_undo_and_scatter_share_one_surface() {
        let mut surface = default_surface();
        let rule = compile(&surface.graph).unwrap();
        let root = surface
            .graph
            .nodes()
            .iter()
            .find(|n| n.archetype == "scatter.output")
            .unwrap()
            .id;
        surface.set_literal(root, 3, "0");
        assert_eq!(compile(&surface.graph).unwrap().density, 0.0);
        assert!(surface.undo());
        assert_eq!(compile(&surface.graph).unwrap(), rule);
        let bytes = super::super::serial::to_json(&surface.graph, &surface.catalogue).unwrap();
        let graph = super::super::serial::from_json(&bytes, &catalogue()).unwrap();
        assert_eq!(compile(&graph).unwrap(), rule);
        let instances = rule
            .scatter(Vec2::ZERO, Vec2::splat(30.0), &BTreeMap::new(), |p| {
                Some(SurfacePoint {
                    position: Vec3::new(p.x, 0.0, p.y),
                    normal: Vec3::Y,
                    tags: BTreeMap::new(),
                })
            })
            .unwrap();
        assert!(!instances.is_empty());
    }

    #[test]
    fn invalid_authoring_reports_the_input() {
        let mut surface = default_surface();
        let root = surface
            .graph
            .nodes()
            .iter()
            .find(|n| n.archetype == "scatter.output")
            .unwrap()
            .id;
        surface.set_literal(root, 2, "NaN");
        assert!(compile(&surface.graph).is_err());
        surface.add("scatter.output", Vec2::ZERO);
        assert!(compile(&surface.graph).is_err());
    }
}
