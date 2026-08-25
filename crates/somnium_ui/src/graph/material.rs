//! Material-graph compilation onto CONTROL-D's runtime material object.

use std::collections::{HashMap, HashSet};

use somnium_asset::material::MaterialAsset;
use somnium_shader::{ShaderKey, ShaderSystem};

use super::{Catalogue, Graph, NodeId, PinRef, PinType};

/// A material graph compiled into the same object property authoring uses.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledMaterialGraph {
    pub material: MaterialAsset,
    /// Standalone, deterministic WGSL. The renderer may compose this through
    /// MORROWIND-C without giving the graph surface a renderer dependency.
    pub wgsl: String,
}

impl CompiledMaterialGraph {
    /// Install the generated module in MORROWIND-C's ordinary variant cache.
    pub fn install(&self, shaders: &mut ShaderSystem) -> ShaderKey {
        ShaderKey::new(shaders.register_generated(self.wgsl.clone()))
    }
}

/// Why a material graph could not become a runtime material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialGraphError {
    MissingRoot,
    MultipleRoots,
    MissingArchetype(String),
    MissingInput { node: NodeId, pin: u16 },
    InvalidLiteral { node: NodeId, pin: u16 },
    UnsupportedNode(String),
    TypeMismatch { expected: PinType, found: PinType },
    Cycle,
}

#[derive(Clone, Debug)]
enum Value {
    Scalar {
        value: Option<f32>,
        expression: String,
    },
    Color {
        value: Option<[f32; 4]>,
        expression: String,
    },
}

impl Value {
    fn ty(&self) -> PinType {
        match self {
            Self::Scalar { .. } => PinType::Float,
            Self::Color { .. } => PinType::Color,
        }
    }

    fn expression(&self) -> &str {
        match self {
            Self::Scalar { expression, .. } | Self::Color { expression, .. } => expression,
        }
    }

    fn into_color(self) -> Result<Self, MaterialGraphError> {
        match self {
            Self::Color { .. } => Ok(self),
            Self::Scalar { value, expression } => Ok(Self::Color {
                value: value.map(|v| [v; 4]),
                expression: format!("vec4<f32>({expression})"),
            }),
        }
    }
}

/// Compile only the nodes that feed the material root. Dead layout fragments
/// cannot change source bytes or the shader cache key.
pub fn compile(
    graph: &Graph,
    catalogue: &Catalogue,
    base: &MaterialAsset,
) -> Result<CompiledMaterialGraph, MaterialGraphError> {
    let root_kind = catalogue.root().ok_or(MaterialGraphError::MissingRoot)?;
    let roots: Vec<_> = graph
        .nodes()
        .iter()
        .filter(|node| node.archetype == root_kind)
        .map(|node| node.id)
        .collect();
    let root = match roots.as_slice() {
        [] => return Err(MaterialGraphError::MissingRoot),
        [root] => *root,
        _ => return Err(MaterialGraphError::MultipleRoots),
    };
    let mut memo = HashMap::new();
    let mut visiting = HashSet::new();
    let base_color = input(graph, catalogue, root, 0, &mut memo, &mut visiting)?.into_color()?;
    let roughness = input(graph, catalogue, root, 1, &mut memo, &mut visiting)?;
    let metallic = input(graph, catalogue, root, 2, &mut memo, &mut visiting)?;
    if roughness.ty() != PinType::Float || metallic.ty() != PinType::Float {
        return Err(MaterialGraphError::TypeMismatch {
            expected: PinType::Float,
            found: if roughness.ty() != PinType::Float {
                roughness.ty()
            } else {
                metallic.ty()
            },
        });
    }

    let mut material = base.clone();
    if let Value::Color { value: Some(v), .. } = &base_color {
        material.base_color.0 = [v[0], v[1], v[2]];
        material.opacity = v[3];
    }
    if let Value::Scalar { value: Some(v), .. } = roughness {
        material.roughness = v.clamp(0.0, 1.0);
    }
    if let Value::Scalar { value: Some(v), .. } = metallic {
        material.metallic = v.clamp(0.0, 1.0);
    }
    let roughness_expression = memo
        .get(&(root, u16::MAX - 1))
        .map_or("0.5", Value::expression);
    let metallic_expression = memo
        .get(&(root, u16::MAX - 2))
        .map_or("0.0", Value::expression);
    let wgsl = format!(
        "struct GraphMaterial {{\n  base_color: vec4<f32>,\n  roughness: f32,\n  metallic: f32,\n}}\n\nfn evaluate_material_graph(uv: vec2<f32>) -> GraphMaterial {{\n  _ = uv;\n  return GraphMaterial({}, {}, {});\n}}\n",
        base_color.expression(),
        roughness_expression,
        metallic_expression
    );
    Ok(CompiledMaterialGraph { material, wgsl })
}

fn input(
    graph: &Graph,
    catalogue: &Catalogue,
    node: NodeId,
    pin: u16,
    memo: &mut HashMap<(NodeId, u16), Value>,
    visiting: &mut HashSet<(NodeId, u16)>,
) -> Result<Value, MaterialGraphError> {
    let target = PinRef::input(node, pin);
    let value = if let Some(source) = graph.input_source(target) {
        output(graph, catalogue, source.node, source.index, memo, visiting)?
    } else {
        let node_data = graph
            .node(node)
            .ok_or(MaterialGraphError::MissingInput { node, pin })?;
        let archetype = catalogue
            .get(&node_data.archetype)
            .ok_or_else(|| MaterialGraphError::MissingArchetype(node_data.archetype.clone()))?;
        let spec = archetype
            .inputs
            .get(pin as usize)
            .ok_or(MaterialGraphError::MissingInput { node, pin })?;
        let literal = node_data
            .literals
            .get(&pin)
            .map(String::as_str)
            .or(spec.default)
            .ok_or(MaterialGraphError::MissingInput { node, pin })?;
        parse_literal(node, pin, spec.ty, literal)?
    };
    // Keep the three root expressions available after their values are moved
    // into the runtime object.
    let memo_pin = match pin {
        1 => u16::MAX - 1,
        2 => u16::MAX - 2,
        _ => pin,
    };
    memo.insert((node, memo_pin), value.clone());
    Ok(value)
}

fn output(
    graph: &Graph,
    catalogue: &Catalogue,
    node: NodeId,
    pin: u16,
    memo: &mut HashMap<(NodeId, u16), Value>,
    visiting: &mut HashSet<(NodeId, u16)>,
) -> Result<Value, MaterialGraphError> {
    if let Some(value) = memo.get(&(node, pin)) {
        return Ok(value.clone());
    }
    if !visiting.insert((node, pin)) {
        return Err(MaterialGraphError::Cycle);
    }
    let node_data = graph
        .node(node)
        .ok_or(MaterialGraphError::MissingInput { node, pin })?;
    let archetype = catalogue
        .get(&node_data.archetype)
        .ok_or_else(|| MaterialGraphError::MissingArchetype(node_data.archetype.clone()))?;
    let output_type = archetype
        .outputs
        .get(pin as usize)
        .ok_or(MaterialGraphError::MissingInput { node, pin })?
        .ty;
    let value = match node_data.archetype.as_str() {
        "material.scalar" => input(graph, catalogue, node, 0, memo, visiting)?,
        "material.color" => input(graph, catalogue, node, 0, memo, visiting)?,
        "material.add" | "material.multiply" => {
            let a = input(graph, catalogue, node, 0, memo, visiting)?;
            let b = input(graph, catalogue, node, 1, memo, visiting)?;
            scalar_binary(node_data.archetype.ends_with("add"), a, b)?
        }
        "material.reroute.float" => input(graph, catalogue, node, 0, memo, visiting)?,
        "material.texture" => {
            return Err(MaterialGraphError::UnsupportedNode(
                node_data.archetype.clone(),
            ));
        }
        other => return Err(MaterialGraphError::UnsupportedNode(other.to_string())),
    };
    visiting.remove(&(node, pin));
    if value.ty() != output_type {
        return Err(MaterialGraphError::TypeMismatch {
            expected: output_type,
            found: value.ty(),
        });
    }
    memo.insert((node, pin), value.clone());
    Ok(value)
}

fn scalar_binary(add: bool, a: Value, b: Value) -> Result<Value, MaterialGraphError> {
    let (
        Value::Scalar {
            value: av,
            expression: ae,
        },
        Value::Scalar {
            value: bv,
            expression: be,
        },
    ) = (a, b)
    else {
        return Err(MaterialGraphError::TypeMismatch {
            expected: PinType::Float,
            found: PinType::Color,
        });
    };
    let operator = if add { "+" } else { "*" };
    Ok(Value::Scalar {
        value: av.zip(bv).map(|(a, b)| if add { a + b } else { a * b }),
        expression: format!("({ae} {operator} {be})"),
    })
}

fn parse_literal(
    node: NodeId,
    pin: u16,
    ty: PinType,
    literal: &str,
) -> Result<Value, MaterialGraphError> {
    match ty {
        PinType::Float => {
            let value: f32 = literal
                .trim()
                .parse()
                .map_err(|_| MaterialGraphError::InvalidLiteral { node, pin })?;
            if !value.is_finite() {
                return Err(MaterialGraphError::InvalidLiteral { node, pin });
            }
            Ok(Value::Scalar {
                value: Some(value),
                expression: wgsl_float(value),
            })
        }
        PinType::Color => {
            let parts: Result<Vec<f32>, _> = literal
                .split(',')
                .map(|part| part.trim().parse::<f32>())
                .collect();
            let mut parts = parts.map_err(|_| MaterialGraphError::InvalidLiteral { node, pin })?;
            if parts.len() == 3 {
                parts.push(1.0);
            }
            if parts.len() != 4 || parts.iter().any(|value| !value.is_finite()) {
                return Err(MaterialGraphError::InvalidLiteral { node, pin });
            }
            let value = [parts[0], parts[1], parts[2], parts[3]];
            Ok(Value::Color {
                value: Some(value),
                expression: format!(
                    "vec4<f32>({}, {}, {}, {})",
                    wgsl_float(value[0]),
                    wgsl_float(value[1]),
                    wgsl_float(value[2]),
                    wgsl_float(value[3])
                ),
            })
        }
        other => Err(MaterialGraphError::TypeMismatch {
            expected: PinType::Float,
            found: other,
        }),
    }
}

fn wgsl_float(value: f32) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    let text = value.to_string();
    if text.contains(['.', 'e', 'E']) {
        text
    } else {
        format!("{text}.0")
    }
}
