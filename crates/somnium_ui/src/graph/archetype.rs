//! MORROWIND-K — archetypes: the data that makes one surface serve five tools.
//!
//! A [`Catalogue`] is what a feature contributes. The surface reads it and can
//! then create, connect, validate, lay out, serialise and draw that feature's
//! graphs without a line of feature-specific code.
//!
//! Flax's `NodeArchetype` / `NodeElementArchetype` / `GroupArchetype` is the
//! shape (§8), read as architecture only — the license audit reclassified Flax
//! as proprietary, so the implementable references are Godot's `GraphEdit` and
//! Fyrox's `absm` editor.

use std::collections::BTreeMap;

/// Which side of a node a pin is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PinDirection {
    Input,
    Output,
}

/// What flows along a wire.
///
/// **Deliberately a small closed set shared by every catalogue**, rather than a
/// per-feature type parameter. A shared set is what lets the surface colour a
/// wire, validate a connection and refuse a mismatch without knowing whose
/// graph it is; a type parameter would push all of that back into each feature
/// and there would be five implementations again.
///
/// A feature whose values do not fit uses [`PinType::Opaque`] with a name, and
/// the surface treats two opaques as compatible only when the names match. That
/// is the escape hatch, and it costs the feature nothing but its own honesty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PinType {
    Bool,
    Float,
    Vec2,
    Vec3,
    Vec4,
    Int,
    /// A colour. Distinct from `Vec4` on purpose: connecting a colour to a
    /// position is almost always a mistake, and a type system that cannot say
    /// so is not earning its keep.
    Color,
    /// A 2D texture.
    Texture,
    /// Anything a catalogue defines for itself — an animation pose, a behaviour
    /// tree node, a particle event.
    Opaque(&'static str),
    /// Execution flow rather than a value: a behaviour tree's edges, a VFX
    /// graph's stages. Never converts to or from a value type.
    Flow,
}

impl PinType {
    /// Whether a value of this type can drive a pin of type `other`.
    ///
    /// Widening scalar conversions are allowed because every shading language
    /// does them and refusing would make the graph more annoying than the code
    /// it replaces. **Narrowing is refused**: a `Vec3` into a `Float` is a
    /// question the author has to answer (which component?) rather than one the
    /// surface should guess.
    #[must_use]
    pub fn connects_to(self, other: PinType) -> bool {
        use PinType::*;
        if self == other {
            return true;
        }
        match (self, other) {
            // Flow and opaque are exact-match only.
            (Flow, _) | (_, Flow) => false,
            (Opaque(_), _) | (_, Opaque(_)) => false,
            // A texture is not a number.
            (Texture, _) | (_, Texture) => false,
            // Scalars splat into vectors: `1.0` into a `Vec3` is `(1,1,1)`,
            // which is what every shading language does.
            (Bool | Int | Float, Vec2 | Vec3 | Vec4 | Color) => true,
            // Integers and booleans promote.
            (Bool, Int | Float) | (Int, Float) => true,
            // A colour is a Vec4 with an opinion; the opinion is lost going out
            // and cannot be invented going in.
            (Color, Vec4) => true,
            (Vec4, Color) => false,
            _ => false,
        }
    }

    /// Durable name, written to a graph file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PinType::Bool => "bool",
            PinType::Float => "float",
            PinType::Vec2 => "vec2",
            PinType::Vec3 => "vec3",
            PinType::Vec4 => "vec4",
            PinType::Int => "int",
            PinType::Color => "color",
            PinType::Texture => "texture",
            PinType::Flow => "flow",
            PinType::Opaque(name) => name,
        }
    }

    /// Parse a durable name against a catalogue's opaque types.
    ///
    /// Opaque names have to be resolved against the catalogue rather than
    /// interned globally, because two catalogues may legitimately both define a
    /// `"pose"` and mean different things.
    #[must_use]
    pub fn parse(text: &str, opaques: &[&'static str]) -> Option<Self> {
        Some(match text {
            "bool" => PinType::Bool,
            "float" => PinType::Float,
            "vec2" => PinType::Vec2,
            "vec3" => PinType::Vec3,
            "vec4" => PinType::Vec4,
            "int" => PinType::Int,
            "color" => PinType::Color,
            "texture" => PinType::Texture,
            "flow" => PinType::Flow,
            other => PinType::Opaque(opaques.iter().find(|name| **name == other)?),
        })
    }
}

/// One pin on a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinArchetype {
    pub name: &'static str,
    pub ty: PinType,
    /// Default value as text, for an unconnected input. The catalogue's owner
    /// parses it; see [`crate::graph::Node::literals`] for why it is a string.
    pub default: Option<&'static str>,
}

/// One row in a node body.
///
/// Pins and literal editors are described here instead of being hard-coded by
/// a material or animation tool.  A renderer may choose its own widgets, but
/// it never has to ask which feature owns the node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeElementArchetype {
    /// Draw an input pin and its label.
    Input(u16),
    /// Draw an output pin and its label.
    Output(u16),
    /// Draw the editor for an unconnected input's literal value.
    Literal(u16),
    /// Static explanatory text.
    Label(&'static str),
    /// A visual break between related rows.
    Separator,
}

/// A stable palette group shared by a catalogue's nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupArchetype {
    /// Durable identifier; never localised or shown directly.
    pub id: &'static str,
    /// Display title in the node palette.
    pub title: &'static str,
    /// Lower values appear first.
    pub order: i16,
}

impl GroupArchetype {
    #[must_use]
    pub const fn new(id: &'static str, title: &'static str, order: i16) -> Self {
        Self { id, title, order }
    }
}

impl PinArchetype {
    #[must_use]
    pub const fn new(name: &'static str, ty: PinType) -> Self {
        Self {
            name,
            ty,
            default: None,
        }
    }

    #[must_use]
    pub const fn with_default(mut self, value: &'static str) -> Self {
        self.default = Some(value);
        self
    }
}

/// A kind of node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeArchetype {
    /// Durable identifier, written to files. Never shown to a user.
    pub id: &'static str,
    /// What the palette shows.
    pub title: &'static str,
    /// Which palette section it appears under.
    pub category: &'static str,
    /// Searchable synonyms, so a user typing "times" finds "Multiply".
    ///
    /// The single cheapest thing a node palette can do and the one most of them
    /// skip.
    pub keywords: &'static [&'static str],
    pub inputs: Vec<PinArchetype>,
    pub outputs: Vec<PinArchetype>,
    /// Data-driven node body. When empty, the surface lays inputs and outputs
    /// out as ordinary pin rows.
    pub elements: Vec<NodeElementArchetype>,
    /// Optional durable palette-group id.
    pub group: Option<&'static str>,
    /// A pass-through node the surface may insert on a wire.
    pub is_reroute: bool,
    /// At most one per catalogue: the node whose inputs are the graph's result.
    pub is_root: bool,
}

impl NodeArchetype {
    #[must_use]
    pub fn new(id: &'static str, title: &'static str, category: &'static str) -> Self {
        Self {
            id,
            title,
            category,
            keywords: &[],
            inputs: Vec::new(),
            outputs: Vec::new(),
            elements: Vec::new(),
            group: None,
            is_reroute: false,
            is_root: false,
        }
    }

    #[must_use]
    pub fn with_input(mut self, pin: PinArchetype) -> Self {
        self.inputs.push(pin);
        self
    }

    #[must_use]
    pub fn with_output(mut self, pin: PinArchetype) -> Self {
        self.outputs.push(pin);
        self
    }

    #[must_use]
    pub fn with_keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }

    #[must_use]
    pub fn with_element(mut self, element: NodeElementArchetype) -> Self {
        self.elements.push(element);
        self
    }

    #[must_use]
    pub fn in_group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    #[must_use]
    pub fn as_reroute(mut self) -> Self {
        self.is_reroute = true;
        self
    }

    #[must_use]
    pub fn as_root(mut self) -> Self {
        self.is_root = true;
        self
    }

    /// Whether `query` should surface this node in the palette.
    ///
    /// Case-insensitive substring over the title, the category and the
    /// keywords. Not a fuzzy matcher: a fuzzy matcher on a hundred nodes
    /// returns everything ranked, and a user who typed three letters wants the
    /// four things that contain them.
    #[must_use]
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }
        self.title.to_ascii_lowercase().contains(&query)
            || self.category.to_ascii_lowercase().contains(&query)
            || self
                .keywords
                .iter()
                .any(|k| k.to_ascii_lowercase().contains(&query))
    }
}

/// Every node kind one feature contributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    /// Durable name, written into a graph file so a loader knows which
    /// catalogue the archetype ids belong to.
    pub id: &'static str,
    /// `BTreeMap` rather than `HashMap`: the palette is listed in a stable
    /// order, and a node palette whose order changes between runs is one people
    /// stop building muscle memory for.
    archetypes: BTreeMap<&'static str, NodeArchetype>,
    groups: BTreeMap<&'static str, GroupArchetype>,
    opaques: Vec<&'static str>,
}

impl Catalogue {
    #[must_use]
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            archetypes: BTreeMap::new(),
            groups: BTreeMap::new(),
            opaques: Vec::new(),
        }
    }

    /// Register a palette group. The last registration under an id wins.
    pub fn register_group(&mut self, group: GroupArchetype) -> &mut Self {
        self.groups.insert(group.id, group);
        self
    }

    /// Register an archetype. The last registration under an id wins, so a
    /// project can override a built-in node.
    pub fn register(&mut self, archetype: NodeArchetype) -> &mut Self {
        for pin in archetype.inputs.iter().chain(archetype.outputs.iter()) {
            if let PinType::Opaque(name) = pin.ty {
                if !self.opaques.contains(&name) {
                    self.opaques.push(name);
                }
            }
        }
        self.archetypes.insert(archetype.id, archetype);
        self
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&NodeArchetype> {
        self.archetypes.get(id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.archetypes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.archetypes.is_empty()
    }

    /// Every archetype, in stable order.
    pub fn all(&self) -> impl Iterator<Item = &NodeArchetype> {
        self.archetypes.values()
    }

    /// Palette search.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&NodeArchetype> {
        self.archetypes
            .values()
            .filter(|a| a.matches(query))
            .collect()
    }

    /// The categories, in stable order, for a sectioned palette.
    #[must_use]
    pub fn categories(&self) -> Vec<&'static str> {
        let mut categories: Vec<&'static str> =
            self.archetypes.values().map(|a| a.category).collect();
        categories.sort_unstable();
        categories.dedup();
        categories
    }

    /// Palette groups in explicit display order, with id as the stable tie
    /// breaker.
    #[must_use]
    pub fn groups(&self) -> Vec<&GroupArchetype> {
        let mut groups: Vec<_> = self.groups.values().collect();
        groups.sort_by_key(|group| (group.order, group.id));
        groups
    }

    /// Look up a palette group by its durable id.
    #[must_use]
    pub fn group(&self, id: &str) -> Option<&GroupArchetype> {
        self.groups.get(id)
    }

    /// The reroute archetype's id, if this catalogue has one.
    #[must_use]
    pub fn reroute(&self) -> Option<&'static str> {
        self.archetypes
            .values()
            .find(|a| a.is_reroute)
            .map(|a| a.id)
    }

    /// The root archetype's id, if this catalogue has one.
    #[must_use]
    pub fn root(&self) -> Option<&'static str> {
        self.archetypes.values().find(|a| a.is_root).map(|a| a.id)
    }

    /// Opaque type names this catalogue uses, for deserialisation.
    #[must_use]
    pub fn opaques(&self) -> &[&'static str] {
        &self.opaques
    }
}
