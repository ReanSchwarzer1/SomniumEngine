//! Phase 16-A: durable component schemas — the engine's one reflected
//! description of what a component *is*.
//!
//! # Why this exists, and why it is not `ComponentId`
//!
//! [`ComponentId`] is assigned lazily, in first-use order, and is
//! therefore **process-local**: the same component type can be id 3 in one
//! run and id 17 in the next. That is fine — it is a fast index into an
//! archetype's columns and nothing else. It is *not* something a scene
//! file, a save game, an undo record or a script may write down.
//!
//! [`StableId`] is the durable name that sits beside it. Two identifiers,
//! two jobs. Conflating them is the defect this module exists to prevent.
//!
//! # One registry, four consumers
//!
//! A [`ComponentSchema`] carries field metadata *and* the four functions
//! needed to move a component's data in and out of the world as neutral
//! values. That single description is what drives:
//!
//! * scene serialization (schema-driven, not a hand-written JSON walk);
//! * script property access (no per-component binding code);
//! * generated script API declarations;
//! * the reflection-driven inspector, when it is asked for.
//!
//! Anything that hand-writes a second description of the same component
//! has reintroduced the problem.
//!
//! # The value model
//!
//! [`ReflectValue`] is deliberately small and closed. Note in particular
//! that entity and asset references are **handles**, never numbers:
//! scripting runtimes in this family do not all have an exact 64-bit
//! integer type, so an id that round-trips through a script's number type
//! would silently lose precision.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;

use crate::component::ComponentId;
use crate::entity::Entity;
use crate::world::{EcsError, World};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Stable identity
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A durable, human-readable name for a component type.
///
/// Written into scene files and script assets, so it may never change
/// once shipped. Renaming a Rust type is free; changing its `StableId`
/// breaks every file that mentions it.
///
/// Convention: `"somnium.Transform"`, `"somnium.Water"`, `"game.Health"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableId(&'static str);

impl StableId {
    /// Wrap a static string as a stable id.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The underlying name, as written to files.
    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Index of a field within its component's schema.
///
/// Assigned in declaration order and stable for a given schema version.
/// Runtime code keys by `FieldId`; *files* key by field name, because a
/// reordered declaration must not silently reinterpret saved data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(pub u16);

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Values
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A reference to a content asset, as a stable 128-bit id.
///
/// Opaque on purpose: it crosses into scripts as a handle, never as a
/// number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetRef(pub u128);

impl AssetRef {
    /// Construct a reflected asset handle from its durable database value.
    #[must_use]
    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    /// Return the durable database value without exposing tuple layout at call sites.
    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }
}

/// A bounded, engine-neutral value.
///
/// Every value that crosses a reflection boundary — into a scene file,
/// into a script, into the inspector — is one of these. Nothing here is
/// borrowed and nothing here is runtime-specific, which is what lets the
/// same value travel to any of those destinations.
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectValue {
    /// Absent or null.
    Nil,
    /// Boolean.
    Bool(bool),
    /// Signed integer. Note the 2^53 caveat in the module docs when this
    /// is handed to a script.
    I64(i64),
    /// Double-precision float. Engine floats are widened on the way out
    /// and narrowed on the way back in.
    F64(f64),
    /// UTF-8 text.
    Str(String),
    /// Two floats.
    Vec2([f32; 2]),
    /// Three floats — also used for linear colour.
    Vec3([f32; 3]),
    /// Four floats.
    Vec4([f32; 4]),
    /// Rotation as `[x, y, z, w]`.
    Quat([f32; 4]),
    /// Live entity handle, or `None` for an unset reference.
    Entity(Option<Entity>),
    /// Content asset reference, or `None` for an unset reference.
    Asset(Option<AssetRef>),
    /// Homogeneous list.
    Array(Vec<ReflectValue>),
    /// Nested record, keyed by field id. Used where a schema is known.
    Object(ReflectObject),
    /// Nested record keyed by name.
    ///
    /// Distinct from [`Self::Object`] because it is used where there is no
    /// schema to key by: a script's declared save state and an event
    /// payload are records the *author* named, not the engine. No
    /// [`FieldType`] accepts a map, so one can never be written into a
    /// component field by accident.
    Map(std::collections::BTreeMap<String, ReflectValue>),
}

/// A record of field values. Ordered, so iteration is deterministic.
pub type ReflectObject = BTreeMap<FieldId, ReflectValue>;

impl ReflectValue {
    /// A short name for this value's kind, for diagnostics.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Bool(_) => "bool",
            Self::I64(_) => "integer",
            Self::F64(_) => "number",
            Self::Str(_) => "string",
            Self::Vec2(_) => "vec2",
            Self::Vec3(_) => "vec3",
            Self::Vec4(_) => "vec4",
            Self::Quat(_) => "quat",
            Self::Entity(_) => "entity",
            Self::Asset(_) => "asset",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
            Self::Map(_) => "map",
        }
    }

    /// Whether every float this value contains is finite.
    ///
    /// A NaN or infinity that reaches physics or a scene file is a defect
    /// that shows up much later and somewhere else, so values are checked
    /// at the boundary they cross rather than where they are used.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        fn all(xs: &[f32]) -> bool {
            xs.iter().all(|x| x.is_finite())
        }
        match self {
            Self::F64(v) => v.is_finite(),
            Self::Vec2(v) => all(v),
            Self::Vec3(v) => all(v),
            Self::Vec4(v) | Self::Quat(v) => all(v),
            Self::Array(items) => items.iter().all(Self::is_finite),
            Self::Object(fields) => fields.values().all(Self::is_finite),
            Self::Map(entries) => entries.values().all(Self::is_finite),
            _ => true,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Field metadata
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The declared type of a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    /// [`ReflectValue::Bool`].
    Bool,
    /// [`ReflectValue::I64`].
    I64,
    /// [`ReflectValue::F64`].
    F64,
    /// [`ReflectValue::Str`].
    Str,
    /// [`ReflectValue::Vec2`].
    Vec2,
    /// [`ReflectValue::Vec3`].
    Vec3,
    /// [`ReflectValue::Vec4`].
    Vec4,
    /// [`ReflectValue::Quat`].
    Quat,
    /// A [`ReflectValue::Vec3`] that means linear colour, so editors show
    /// a swatch rather than three spinners.
    Color,
    /// [`ReflectValue::Entity`].
    Entity,
    /// [`ReflectValue::Asset`].
    Asset,
    /// An [`ReflectValue::I64`] restricted to a named set of variants.
    Enum(&'static [&'static str]),
    /// A homogeneous [`ReflectValue::Array`].
    Array(Box<FieldType>),
}

impl FieldType {
    /// Whether `value` is an acceptable instance of this type.
    ///
    /// `Nil` is accepted for reference types only. Integers are accepted
    /// where a float is declared (a script that writes `1` for a speed of
    /// `1.0` is not making a mistake); the reverse is not.
    #[must_use]
    pub fn accepts(&self, value: &ReflectValue) -> bool {
        match (self, value) {
            (Self::Bool, ReflectValue::Bool(_))
            | (Self::I64, ReflectValue::I64(_))
            | (Self::Str, ReflectValue::Str(_))
            | (Self::Vec2, ReflectValue::Vec2(_))
            | (Self::Vec3 | Self::Color, ReflectValue::Vec3(_))
            | (Self::Vec4, ReflectValue::Vec4(_))
            | (Self::Quat, ReflectValue::Quat(_))
            | (Self::Entity, ReflectValue::Entity(_) | ReflectValue::Nil)
            | (Self::Asset, ReflectValue::Asset(_) | ReflectValue::Nil)
            // An integer is an acceptable number: `speed = 4` from a
            // script is not a mistake.
            | (Self::F64, ReflectValue::F64(_) | ReflectValue::I64(_)) => true,
            (Self::Enum(names), ReflectValue::I64(v)) => {
                usize::try_from(*v).is_ok_and(|v| v < names.len())
            }
            (Self::Array(inner), ReflectValue::Array(items)) => {
                items.iter().all(|item| inner.accepts(item))
            }
            _ => false,
        }
    }

    /// Re-tag a value the shape converter could not have got right.
    ///
    /// A script writes a rotation as four numbers, and four numbers are a
    /// [`ReflectValue::Vec4`] by shape — there is nothing in
    /// `{x=0,y=0,z=0,w=1}` that says "quaternion". The declared type is the
    /// only thing that knows, so the boundary asks the field rather than
    /// guessing, and [`Self::accepts`] stays strict.
    ///
    /// The second case is the same problem one step down: a script-family
    /// runtime with a single number type hands back `3` for a value that
    /// went in as `3.0`, so a float field would quietly start storing
    /// integers. [`Self::accepts`] already tolerates that; this stops it
    /// being *written down* that way, which is what a scene diff and a
    /// value comparison both care about.
    ///
    /// Deliberately narrow: these are the two ambiguities the value model
    /// has, and widening this into a general coercion table would turn
    /// every mistyped write into a silent reinterpretation.
    #[must_use]
    pub fn coerce(&self, value: ReflectValue) -> ReflectValue {
        match (self, value) {
            (Self::Quat, ReflectValue::Vec4(v)) => ReflectValue::Quat(v),
            #[allow(clippy::cast_precision_loss)]
            (Self::F64, ReflectValue::I64(v)) => ReflectValue::F64(v as f64),
            (_, value) => value,
        }
    }

    /// Human-readable name, for diagnostics and generated declarations.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Bool => "boolean".into(),
            Self::I64 => "integer".into(),
            Self::F64 => "number".into(),
            Self::Str => "string".into(),
            Self::Vec2 => "Vec2".into(),
            Self::Vec3 => "Vec3".into(),
            Self::Vec4 => "Vec4".into(),
            Self::Quat => "Quat".into(),
            Self::Color => "Color".into(),
            Self::Entity => "Entity".into(),
            Self::Asset => "Asset".into(),
            Self::Enum(_) => "enum".into(),
            Self::Array(inner) => format!("{{{}}}", inner.name()),
        }
    }
}

/// Per-field permissions. A field can be authored in the editor, saved,
/// read by scripts and written by scripts independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldFlags(u32);

impl FieldFlags {
    /// Written to and read from scene files.
    pub const SERIALIZE: Self = Self(1 << 0);
    /// Shown in the inspector.
    pub const EDIT: Self = Self(1 << 1);
    /// Readable from a script snapshot.
    pub const SCRIPT_READ: Self = Self(1 << 2);
    /// Writable through a script command.
    pub const SCRIPT_WRITE: Self = Self(1 << 3);
    /// The usual case: authored, saved, and fully script-accessible.
    pub const DEFAULT: Self = Self(0b1111);
    /// Derived state that is recomputed, not authored — readable only.
    pub const RUNTIME_ONLY: Self = Self(1 << 2);

    /// Whether all of `other`'s bits are set.
    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Union of two flag sets.
    #[inline]
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// How much state an edit must snapshot to undo safely.
///
/// Most fields are independent scalar values. Rebuilding fields can declare a
/// wider scope so a generic editor command never restores only half of their
/// derived state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChangeScope {
    /// Restore just the addressed field.
    #[default]
    Field,
    /// Restore the complete component record.
    Component,
    /// Restore every registered component on the entity.
    Entity,
    /// Restore the complete registered world state.
    Scene,
}

/// Everything the engine knows about one field of one component.
#[derive(Debug, Clone)]
pub struct FieldSchema {
    /// Durable name. Written to files; may not change once shipped.
    pub name: &'static str,
    /// Position in the schema, assigned in declaration order.
    pub id: FieldId,
    /// Declared type.
    pub ty: FieldType,
    /// Value used when the field is absent from a file or a script.
    pub default: ReflectValue,
    /// Inclusive lower bound for numeric fields.
    pub min: Option<f64>,
    /// Inclusive upper bound for numeric fields.
    pub max: Option<f64>,
    /// Preferred numeric increment.
    pub step: Option<f64>,
    /// Suggested slider lower bound; typing may exceed it.
    pub soft_min: Option<f64>,
    /// Suggested slider upper bound; typing may exceed it.
    pub soft_max: Option<f64>,
    /// Decimal places shown by numeric editors.
    pub precision: Option<u8>,
    /// Unit suffix, such as `m`, `deg`, or `ms`.
    pub unit: Option<&'static str>,
    /// Author-facing help copied from the component contract.
    pub doc: Option<&'static str>,
    /// Label override; otherwise generated from [`Self::name`].
    pub display_name: Option<&'static str>,
    /// Foldable inspector section.
    pub group: Option<&'static str>,
    /// Stable ordering hint within the group.
    pub order: Option<i32>,
    /// Hidden behind the inspector's advanced-properties affordance.
    pub advanced: bool,
    /// Visible but never writable from the editor.
    pub read_only: bool,
    /// Undo snapshot width required by this field.
    pub scope: ChangeScope,
    /// Engine-neutral asset-kind constraint. Each bit is assigned by the asset
    /// layer; `u64::MAX` accepts every kind and avoids an ECS → asset dependency.
    pub asset_kind_mask: u64,
    /// Permissions.
    pub flags: FieldFlags,
}

impl FieldSchema {
    /// Check a candidate value against this field's type and range.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the field, so the message a script
    /// author sees says which property was wrong.
    pub fn validate(&self, value: &ReflectValue) -> Result<(), ReflectError> {
        if !self.ty.accepts(value) {
            return Err(ReflectError::TypeMismatch {
                field: self.name,
                expected: self.ty.name(),
                found: value.kind(),
            });
        }
        if !value.is_finite() {
            return Err(ReflectError::NotFinite { field: self.name });
        }
        let scalar = match value {
            ReflectValue::F64(v) => Some(*v),
            #[allow(clippy::cast_precision_loss)]
            ReflectValue::I64(v) => Some(*v as f64),
            _ => None,
        };
        if let Some(v) = scalar {
            if self.min.is_some_and(|lo| v < lo) || self.max.is_some_and(|hi| v > hi) {
                return Err(ReflectError::OutOfRange {
                    field: self.name,
                    min: self.min,
                    max: self.max,
                });
            }
        }
        Ok(())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Errors
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Failure modes of reflected access.
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectError {
    /// The entity handle is stale.
    DeadEntity,
    /// The entity does not have the component being addressed.
    MissingComponent(StableId),
    /// No schema is registered under that name.
    UnknownComponent(String),
    /// The component has no such field.
    UnknownField {
        /// Component the field was looked for on.
        component: StableId,
        /// Field name as written by the caller.
        field: String,
    },
    /// The value's kind does not match the field's declared type.
    TypeMismatch {
        /// Field name.
        field: &'static str,
        /// Declared type.
        expected: String,
        /// What arrived instead.
        found: &'static str,
    },
    /// A numeric value fell outside the field's declared range.
    OutOfRange {
        /// Field name.
        field: &'static str,
        /// Declared minimum, if any.
        min: Option<f64>,
        /// Declared maximum, if any.
        max: Option<f64>,
    },
    /// A float was NaN or infinite.
    NotFinite {
        /// Field name.
        field: &'static str,
    },
    /// Stored data is from a schema version this build cannot read.
    UnsupportedVersion {
        /// Component being loaded.
        component: StableId,
        /// Version found in the data.
        found: u32,
        /// Version this build writes.
        current: u32,
    },
}

impl fmt::Display for ReflectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeadEntity => write!(f, "entity handle is stale"),
            Self::MissingComponent(id) => write!(f, "entity has no `{id}`"),
            Self::UnknownComponent(name) => write!(f, "no component schema named `{name}`"),
            Self::UnknownField { component, field } => {
                write!(f, "`{component}` has no field `{field}`")
            }
            Self::TypeMismatch {
                field,
                expected,
                found,
            } => write!(f, "field `{field}` expects {expected}, found {found}"),
            Self::OutOfRange { field, min, max } => {
                write!(f, "field `{field}` is out of range")?;
                if let Some(lo) = min {
                    write!(f, ", min {lo}")?;
                }
                if let Some(hi) = max {
                    write!(f, ", max {hi}")?;
                }
                Ok(())
            }
            Self::NotFinite { field } => write!(f, "field `{field}` is not finite"),
            Self::UnsupportedVersion {
                component,
                found,
                current,
            } => write!(
                f,
                "`{component}` data is version {found}; this build reads up to {current}"
            ),
        }
    }
}

impl std::error::Error for ReflectError {}

impl From<EcsError> for ReflectError {
    fn from(value: EcsError) -> Self {
        match value {
            EcsError::DeadEntity => Self::DeadEntity,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Component schema
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The reflected description of one component type.
///
/// Built once at startup — normally by the [`component_schema!`] macro,
/// which derives the field list and the four access functions from the
/// struct's own fields so there is nothing to keep in sync by hand.
pub struct ComponentSchema {
    /// Durable name used in files and scripts.
    pub stable_id: StableId,
    /// Name shown in the editor.
    pub display_name: &'static str,
    /// Schema version, bumped whenever the field set changes meaning.
    pub version: u32,
    /// Fields in declaration order; `fields[i].id == FieldId(i)`.
    pub fields: Vec<FieldSchema>,
    /// This process's runtime id for the type.
    pub component_id: ComponentId,
    /// Read every field into a neutral record.
    pub snapshot: fn(&World, Entity) -> Option<ReflectObject>,
    /// Read **one** field.
    ///
    /// Not a convenience wrapper over [`Self::snapshot`]: reading a single
    /// field is the overwhelmingly common case — every `ctx:get` a script
    /// makes is one — and going through `snapshot` allocates a `BTreeMap`
    /// and converts every other field to throw them away. Measured at
    /// 0.68 µs per host call before this existed, against a budget that
    /// allows 0.075 µs.
    pub read_field: fn(&World, Entity, FieldId) -> Option<ReflectValue>,
    /// Write the present fields of a (possibly partial) record.
    pub apply: fn(&mut World, Entity, &ReflectObject) -> Result<(), ReflectError>,
    /// Attach the component with all fields at their defaults.
    pub insert_default: fn(&mut World, Entity) -> Result<(), ReflectError>,
    /// Detach the component.
    pub remove: fn(&mut World, Entity) -> Result<bool, ReflectError>,
    /// Bring an older record forward, if this component has ever changed
    /// shape. `None` means every version reads the same.
    pub migrate: Option<MigrateFn>,
}

/// Rewrite a record saved under an older schema version into the shape
/// this build expects.
pub type MigrateFn = fn(&mut ReflectObject, u32) -> Result<(), ReflectError>;

impl ComponentSchema {
    /// Look a field up by its durable name.
    #[must_use]
    pub fn field_by_name(&self, name: &str) -> Option<&FieldSchema> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Look a field up by id.
    #[must_use]
    pub fn field(&self, id: FieldId) -> Option<&FieldSchema> {
        self.fields.get(id.0 as usize)
    }

    /// Validate a record against this schema, rejecting unknown fields.
    ///
    /// Partial records are fine — absent fields keep their current value.
    ///
    /// # Errors
    ///
    /// The first offending field, named.
    pub fn validate(&self, record: &ReflectObject) -> Result<(), ReflectError> {
        for (id, value) in record {
            let Some(field) = self.field(*id) else {
                return Err(ReflectError::UnknownField {
                    component: self.stable_id,
                    field: format!("#{}", id.0),
                });
            };
            field.validate(value)?;
        }
        Ok(())
    }

    /// Fill in defaults for every field the record does not mention.
    pub fn apply_defaults(&self, record: &mut ReflectObject) {
        for field in &self.fields {
            record
                .entry(field.id)
                .or_insert_with(|| field.default.clone());
        }
    }
}

impl fmt::Debug for ComponentSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComponentSchema")
            .field("stable_id", &self.stable_id)
            .field("display_name", &self.display_name)
            .field("version", &self.version)
            .field("fields", &self.fields.len())
            .field("component_id", &self.component_id)
            .finish_non_exhaustive()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Registry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Every component schema this build knows about.
///
/// Iteration order is sorted by [`StableId`] and never by registration
/// order, insertion order or hash order — a serializer that walks this
/// registry must produce the same file on every run.
#[derive(Default)]
pub struct TypeRegistry {
    /// Sorted by `stable_id`.
    schemas: Vec<ComponentSchema>,
    by_stable: HashMap<StableId, usize>,
    by_runtime: HashMap<ComponentId, usize>,
    /// Name lookup for the script boundary, where every `ctx:get` resolves
    /// a component by the string an author typed. A linear scan over the
    /// schema list was measurable at that call rate.
    by_text: HashMap<&'static str, usize>,
    decorators: Vec<Box<dyn SchemaDecorator>>,
}

impl fmt::Debug for TypeRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypeRegistry")
            .field("schemas", &self.schemas)
            .field("decorators", &self.decorators.len())
            .finish_non_exhaustive()
    }
}

/// Editor-only schema metadata extension point.
///
/// Decorators keep reflection authoritative while allowing a later tool or
/// plugin to attach presentation metadata without editing a component's
/// declaration.
pub trait SchemaDecorator: Send + Sync {
    /// Mutate a schema when it enters the registry.
    fn decorate(&self, schema: &mut ComponentSchema);
}

impl TypeRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a schema.
    ///
    /// # Panics
    ///
    /// Panics if the stable id or the runtime component id is already
    /// registered. Both are programmer errors that would otherwise show
    /// up as a component silently reading another one's data.
    pub fn register(&mut self, mut schema: ComponentSchema) {
        for decorator in &self.decorators {
            decorator.decorate(&mut schema);
        }
        assert!(
            !self.by_stable.contains_key(&schema.stable_id),
            "duplicate component stable id `{}`",
            schema.stable_id
        );
        assert!(
            !self.by_runtime.contains_key(&schema.component_id),
            "component `{}` is already registered under another stable id",
            schema.stable_id
        );
        debug_assert!(
            schema
                .fields
                .iter()
                .enumerate()
                .all(|(i, f)| usize::from(f.id.0) == i),
            "field ids must match declaration order in `{}`",
            schema.stable_id
        );

        let pos = self
            .schemas
            .partition_point(|s| s.stable_id < schema.stable_id);
        self.schemas.insert(pos, schema);
        self.reindex();
    }

    /// Add an editor metadata decorator and apply it to existing schemas.
    pub fn register_decorator(&mut self, decorator: Box<dyn SchemaDecorator>) {
        for schema in &mut self.schemas {
            decorator.decorate(schema);
        }
        self.decorators.push(decorator);
    }

    /// Look up by durable name.
    #[must_use]
    pub fn by_stable_id(&self, id: StableId) -> Option<&ComponentSchema> {
        self.by_stable.get(&id).map(|&i| &self.schemas[i])
    }

    /// Look up by durable name, as written in a file or by a script.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&ComponentSchema> {
        self.by_text.get(name).map(|&i| &self.schemas[i])
    }

    /// Look up by this process's runtime component id.
    #[must_use]
    pub fn by_component_id(&self, id: ComponentId) -> Option<&ComponentSchema> {
        self.by_runtime.get(&id).map(|&i| &self.schemas[i])
    }

    /// Every schema, sorted by stable id.
    pub fn iter(&self) -> impl Iterator<Item = &ComponentSchema> {
        self.schemas.iter()
    }

    /// Number of registered schemas.
    #[must_use]
    pub fn len(&self) -> usize {
        self.schemas.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
    }

    /// The schemas describing the components `entity` actually has, in
    /// stable-id order.
    ///
    /// This is the serializer's and the inspector's entry point. Returns
    /// an empty vec for a dead entity.
    #[must_use]
    pub fn schemas_on(&self, world: &World, entity: Entity) -> Vec<&ComponentSchema> {
        let mut found: Vec<&ComponentSchema> = world
            .component_ids(entity)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| self.by_component_id(id))
            .collect();
        found.sort_unstable_by_key(|s| s.stable_id);
        found
    }

    fn reindex(&mut self) {
        self.by_stable.clear();
        self.by_runtime.clear();
        self.by_text.clear();
        for (i, schema) in self.schemas.iter().enumerate() {
            self.by_stable.insert(schema.stable_id, i);
            self.by_runtime.insert(schema.component_id, i);
            self.by_text.insert(schema.stable_id.as_str(), i);
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Field conversion
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A Rust type that can appear as a reflected component field.
///
/// Implemented here for the primitives and fixed-size float arrays;
/// downstream crates implement it for their own math types (`glam::Vec3`
/// and friends) so the macro below works on real components.
pub trait ReflectField: Sized {
    /// The declared type this maps onto.
    fn field_type() -> FieldType;
    /// Widen into a neutral value.
    fn to_reflect(&self) -> ReflectValue;
    /// Narrow back, rejecting anything that does not fit.
    ///
    /// # Errors
    ///
    /// [`ReflectError::TypeMismatch`] with the caller-supplied field name.
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError>;
}

/// Build the "wrong type" error for a field, so implementations do not
/// each spell it out.
fn mismatch(field: &'static str, expected: &FieldType, found: &ReflectValue) -> ReflectError {
    ReflectError::TypeMismatch {
        field,
        expected: expected.name(),
        found: found.kind(),
    }
}

impl ReflectField for bool {
    fn field_type() -> FieldType {
        FieldType::Bool
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::Bool(*self)
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::Bool(v) => Ok(*v),
            other => Err(mismatch(field, &FieldType::Bool, other)),
        }
    }
}

impl ReflectField for String {
    fn field_type() -> FieldType {
        FieldType::Str
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::Str(self.clone())
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::Str(v) => Ok(v.clone()),
            other => Err(mismatch(field, &FieldType::Str, other)),
        }
    }
}

/// Integer field impls. Every integral component field widens to `i64`
/// and narrows back with a range check, so a script cannot write 70,000
/// into a `u16` and get 4,464.
macro_rules! impl_reflect_integer {
    ($($t:ty),* $(,)?) => { $(
        impl ReflectField for $t {
            fn field_type() -> FieldType { FieldType::I64 }
            fn to_reflect(&self) -> ReflectValue {
                ReflectValue::I64(i64::from(*self))
            }
            fn from_reflect(value: &ReflectValue, field: &'static str)
                -> Result<Self, ReflectError>
            {
                let raw = match value {
                    ReflectValue::I64(v) => *v,
                    other => return Err(mismatch(field, &FieldType::I64, other)),
                };
                Self::try_from(raw).map_err(|_| ReflectError::OutOfRange {
                    field,
                    min: Some(f64::from(Self::MIN)),
                    max: Some(f64::from(Self::MAX)),
                })
            }
        }
    )* };
}
impl_reflect_integer!(u8, u16, u32, i8, i16, i32);

impl ReflectField for i64 {
    fn field_type() -> FieldType {
        FieldType::I64
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::I64(*self)
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::I64(v) => Ok(*v),
            other => Err(mismatch(field, &FieldType::I64, other)),
        }
    }
}

/// Float impls. Integers are accepted where a float is declared, because
/// `speed = 4` from a script is not an error.
macro_rules! impl_reflect_float {
    ($($t:ty),* $(,)?) => { $(
        impl ReflectField for $t {
            fn field_type() -> FieldType { FieldType::F64 }
            #[allow(clippy::cast_lossless)]
            fn to_reflect(&self) -> ReflectValue { ReflectValue::F64(*self as f64) }
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            fn from_reflect(value: &ReflectValue, field: &'static str)
                -> Result<Self, ReflectError>
            {
                match value {
                    ReflectValue::F64(v) => Ok(*v as Self),
                    ReflectValue::I64(v) => Ok(*v as Self),
                    other => Err(mismatch(field, &FieldType::F64, other)),
                }
            }
        }
    )* };
}
impl_reflect_float!(f32, f64);

/// Fixed-size float array impls, for engine math types that are laid out
/// as plain arrays.
macro_rules! impl_reflect_farray {
    ($($n:literal => $variant:ident, $ty:ident);* $(;)?) => { $(
        impl ReflectField for [f32; $n] {
            fn field_type() -> FieldType { FieldType::$ty }
            fn to_reflect(&self) -> ReflectValue { ReflectValue::$variant(*self) }
            fn from_reflect(value: &ReflectValue, field: &'static str)
                -> Result<Self, ReflectError>
            {
                match value {
                    ReflectValue::$variant(v) => Ok(*v),
                    other => Err(mismatch(field, &FieldType::$ty, other)),
                }
            }
        }
    )* };
}
impl_reflect_farray!(2 => Vec2, Vec2; 3 => Vec3, Vec3; 4 => Vec4, Vec4);

/// Engine math types.
///
/// These impls live here rather than in `somnium_core` because the orphan
/// rule forbids implementing a foreign trait for a foreign type: the trait
/// is this crate's and `glam`'s types are not `somnium_core`'s.
macro_rules! impl_reflect_glam {
    ($($t:ty => $variant:ident, $ty:ident, $n:literal);* $(;)?) => { $(
        impl ReflectField for $t {
            fn field_type() -> FieldType { FieldType::$ty }
            fn to_reflect(&self) -> ReflectValue {
                ReflectValue::$variant(self.to_array())
            }
            fn from_reflect(value: &ReflectValue, field: &'static str)
                -> Result<Self, ReflectError>
            {
                match value {
                    ReflectValue::$variant(v) => Ok(Self::from_array(*v)),
                    other => Err(mismatch(field, &FieldType::$ty, other)),
                }
            }
        }
    )* };
}
impl_reflect_glam!(
    glam::Vec2 => Vec2, Vec2, 2;
    glam::Vec3 => Vec3, Vec3, 3;
    glam::Vec4 => Vec4, Vec4, 4;
);

impl ReflectField for glam::Quat {
    fn field_type() -> FieldType {
        FieldType::Quat
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::Quat(self.to_array())
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            // Normalising on the way in is deliberate: a script that
            // builds a rotation by hand will drift, and an unnormalised
            // quaternion reaching the transform hierarchy scales
            // everything below it.
            ReflectValue::Quat(v) => Ok(Self::from_array(*v).normalize()),
            other => Err(mismatch(field, &FieldType::Quat, other)),
        }
    }
}

impl ReflectField for Entity {
    fn field_type() -> FieldType {
        FieldType::Entity
    }
    fn to_reflect(&self) -> ReflectValue {
        if *self == Entity::DANGLING {
            ReflectValue::Entity(None)
        } else {
            ReflectValue::Entity(Some(*self))
        }
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::Entity(Some(e)) => Ok(*e),
            ReflectValue::Entity(None) | ReflectValue::Nil => Ok(Self::DANGLING),
            other => Err(mismatch(field, &FieldType::Entity, other)),
        }
    }
}

impl ReflectField for AssetRef {
    fn field_type() -> FieldType {
        FieldType::Asset
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::Asset(Some(*self))
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::Asset(Some(value)) => Ok(*value),
            other => Err(mismatch(field, &FieldType::Asset, other)),
        }
    }
}

impl ReflectField for Option<AssetRef> {
    fn field_type() -> FieldType {
        FieldType::Asset
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::Asset(*self)
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::Asset(value) => Ok(*value),
            ReflectValue::Nil => Ok(None),
            other => Err(mismatch(field, &FieldType::Asset, other)),
        }
    }
}

impl<T: ReflectField> ReflectField for Vec<T> {
    fn field_type() -> FieldType {
        FieldType::Array(Box::new(T::field_type()))
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::Array(self.iter().map(ReflectField::to_reflect).collect())
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::Array(items) => items
                .iter()
                .map(|item| T::from_reflect(item, field))
                .collect(),
            other => Err(mismatch(field, &Self::field_type(), other)),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Declaration macro
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Declare a [`ComponentSchema`] from a component's own fields.
///
/// This is what keeps the registry from becoming the hand-written binding
/// sprawl it exists to replace: the field list, the snapshot function and
/// the apply function all come from one declaration, so they cannot drift
/// apart.
///
/// ```
/// use somnium_ecs::{Component, component_schema};
///
/// #[derive(Debug, Clone, Copy, Default)]
/// pub struct Health { pub current: f32, pub max: f32, pub invulnerable: bool }
/// impl Component for Health {}
///
/// let schema = component_schema! {
///     Health as "game.Health", display "Health", version 1,
///     fields {
///         current { min: 0.0 },
///         max { min: 0.0 },
///         invulnerable,
///     }
/// };
/// assert_eq!(schema.fields.len(), 3);
/// assert_eq!(schema.field_by_name("current").unwrap().min, Some(0.0));
/// ```
///
/// The component must be `Default + Clone` (for `insert_default` and for
/// staged writes) and every named field must implement [`ReflectField`].
#[macro_export]
macro_rules! component_schema {
    (@doc) => { None };
    (@doc $first:literal $(, $rest:literal)*) => { Some(concat!($first $(, "\n", $rest)*)) };
    // Per-field options. These rules come first so that the recursive
    // `@opt` calls below can never be mistaken for a schema declaration.
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        min, $v:expr) => { $min = Some(f64::from($v)); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        max, $v:expr) => { $max = Some(f64::from($v)); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        advanced, $v:expr) => { $advanced = $v; };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        read_only, $v:expr) => { $read_only = $v; };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        scope, $v:expr) => { $scope = $v; };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        asset_kind_mask, $v:expr) => { $asset_kind_mask = $v; };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        flags, $v:expr) => { $flags = $v; };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        step, $v:expr) => { $step = Some(f64::from($v)); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        soft_min, $v:expr) => { $soft_min = Some(f64::from($v)); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        soft_max, $v:expr) => { $soft_max = Some(f64::from($v)); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        precision, $v:expr) => { $precision = Some($v); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        unit, $v:expr) => { $unit = Some($v); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        doc, $v:expr) => { $doc = Some($v); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        display_name, $v:expr) => { $display_name = Some($v); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        group, $v:expr) => { $group = Some($v); };
    (@opt $min:ident, $max:ident, $step:ident, $soft_min:ident, $soft_max:ident,
        $precision:ident, $unit:ident, $doc:ident, $display_name:ident, $group:ident,
        $order:ident, $advanced:ident, $read_only:ident, $scope:ident, $asset_kind_mask:ident, $flags:ident,
        order, $v:expr) => { $order = Some($v); };

    (
        $ty:ty as $stable:literal,
        display $display:literal,
        version $version:expr,
        fields { $( $(#[doc = $field_doc:literal])* $field:ident $({ $($opt:ident : $optval:expr),* $(,)? })? ),* $(,)? }
    ) => {{
        use $crate::reflect::{
            ChangeScope, ComponentSchema, FieldFlags, FieldId, FieldSchema, ReflectError, ReflectField,
            ReflectFieldTypeOf, ReflectObject, StableId,
        };

        const STABLE: StableId = StableId::new($stable);

        // Field ids are positions in the declaration, assigned here so no
        // two places can disagree about them.
        let mut fields: Vec<FieldSchema> = Vec::new();
        let defaults = <$ty>::default();
        $(
            {
                // `unused_assignments` is expected: a field that declares
                // an option overwrites the `None` seed unconditionally.
                #[allow(unused_mut, unused_assignments)]
                let mut min: Option<f64> = None;
                #[allow(unused_mut, unused_assignments)]
                let mut max: Option<f64> = None;
                #[allow(unused_mut, unused_assignments)] let mut step: Option<f64> = None;
                #[allow(unused_mut, unused_assignments)] let mut soft_min: Option<f64> = None;
                #[allow(unused_mut, unused_assignments)] let mut soft_max: Option<f64> = None;
                #[allow(unused_mut, unused_assignments)] let mut precision: Option<u8> = None;
                #[allow(unused_mut, unused_assignments)] let mut unit: Option<&'static str> = None;
                #[allow(unused_mut, unused_assignments)] let mut doc: Option<&'static str> =
                    $crate::component_schema!(@doc $($field_doc),*);
                #[allow(unused_mut, unused_assignments)] let mut display_name: Option<&'static str> = None;
                #[allow(unused_mut, unused_assignments)] let mut group: Option<&'static str> = None;
                #[allow(unused_mut, unused_assignments)] let mut order: Option<i32> = None;
                #[allow(unused_mut, unused_assignments)] let mut advanced = false;
                #[allow(unused_mut, unused_assignments)] let mut read_only = false;
                #[allow(unused_mut, unused_assignments)] let mut scope = ChangeScope::Field;
                #[allow(unused_mut, unused_assignments)] let mut asset_kind_mask = u64::MAX;
                #[allow(unused_mut, unused_assignments)]
                let mut flags: FieldFlags = FieldFlags::DEFAULT;
                $( $( $crate::component_schema!(@opt min, max, step, soft_min, soft_max,
                    precision, unit, doc, display_name, group, order, advanced, read_only,
                    scope, asset_kind_mask, flags, $opt, $optval); )* )?
                fields.push(FieldSchema {
                    name: stringify!($field),
                    id: FieldId(u16::try_from(fields.len()).expect("too many fields")),
                    ty: ReflectFieldTypeOf::field_type_of(&defaults.$field),
                    default: ReflectField::to_reflect(&defaults.$field),
                    min,
                    max,
                    step,
                    soft_min,
                    soft_max,
                    precision,
                    unit,
                    doc,
                    display_name,
                    group,
                    order,
                    advanced,
                    read_only,
                    scope,
                    asset_kind_mask,
                    flags,
                });
            }
        )*

        fn snapshot(world: &$crate::World, entity: $crate::Entity) -> Option<ReflectObject> {
            let value = world.get::<$ty>(entity)?;
            let mut out = ReflectObject::new();
            let mut next: u16 = 0;
            $(
                out.insert(FieldId(next), ReflectField::to_reflect(&value.$field));
                next += 1;
            )*
            let _ = next;
            Some(out)
        }

        fn read_field(
            world: &$crate::World,
            entity: $crate::Entity,
            id: FieldId,
        ) -> Option<$crate::reflect::ReflectValue> {
            let value = world.get::<$ty>(entity)?;
            let mut next: u16 = 0;
            $(
                if id == FieldId(next) {
                    return Some(ReflectField::to_reflect(&value.$field));
                }
                next += 1;
            )*
            let _ = next;
            None
        }

        fn apply(
            world: &mut $crate::World,
            entity: $crate::Entity,
            record: &ReflectObject,
        ) -> Result<(), ReflectError> {
            if !world.is_alive(entity) {
                return Err(ReflectError::DeadEntity);
            }
            // Narrow every field first, so a record that is wrong halfway
            // through leaves the component untouched rather than half
            // written.
            let mut staged = match world.get::<$ty>(entity) {
                Some(v) => v.clone(),
                None => return Err(ReflectError::MissingComponent(STABLE)),
            };
            let mut next: u16 = 0;
            $(
                if let Some(value) = record.get(&FieldId(next)) {
                    staged.$field =
                        ReflectField::from_reflect(value, stringify!($field))?;
                }
                next += 1;
            )*
            let _ = next;
            *world.get_mut::<$ty>(entity).ok_or(ReflectError::MissingComponent(STABLE))? = staged;
            Ok(())
        }

        fn insert_default(
            world: &mut $crate::World,
            entity: $crate::Entity,
        ) -> Result<(), ReflectError> {
            world.insert_component(entity, <$ty>::default()).map_err(Into::into)
        }

        fn remove(
            world: &mut $crate::World,
            entity: $crate::Entity,
        ) -> Result<bool, ReflectError> {
            world.remove_component::<$ty>(entity).map_err(Into::into)
        }

        ComponentSchema {
            stable_id: STABLE,
            display_name: $display,
            version: $version,
            fields,
            component_id: $crate::ComponentId::of::<$ty>(),
            snapshot,
            read_field,
            apply,
            insert_default,
            remove,
            migrate: None,
        }
    }};
}

/// Helper so the macro can ask for a value's field type without naming
/// the type. Blanket-implemented for every [`ReflectField`].
pub trait ReflectFieldTypeOf {
    /// The declared type of the value's Rust type.
    fn field_type_of(&self) -> FieldType;
}

impl<T: ReflectField> ReflectFieldTypeOf for T {
    fn field_type_of(&self) -> FieldType {
        T::field_type()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Component;

    #[derive(Debug, Clone, Copy, Default, PartialEq)]
    struct Motor {
        speed: f32,
        gear: u8,
        engaged: bool,
    }
    impl Component for Motor {}

    #[derive(Debug, Clone, Copy, Default, PartialEq)]
    struct Anchor {
        offset: [f32; 3],
    }
    impl Component for Anchor {}

    fn motor_schema() -> ComponentSchema {
        crate::component_schema! {
            Motor as "test.Motor", display "Motor", version 1,
            fields {
                speed { min: 0.0, max: 30.0 },
                gear { max: 6 },
                engaged,
            }
        }
    }

    fn anchor_schema() -> ComponentSchema {
        crate::component_schema! {
            Anchor as "test.Anchor", display "Anchor", version 1,
            fields { offset }
        }
    }

    #[test]
    fn schema_field_ids_follow_declaration_order() {
        let schema = motor_schema();
        assert_eq!(schema.fields.len(), 3);
        for (i, field) in schema.fields.iter().enumerate() {
            assert_eq!(field.id, FieldId(u16::try_from(i).unwrap()));
        }
        assert_eq!(schema.field_by_name("speed").unwrap().ty, FieldType::F64);
        assert_eq!(schema.field_by_name("gear").unwrap().ty, FieldType::I64);
        assert_eq!(schema.field_by_name("engaged").unwrap().ty, FieldType::Bool);
    }

    #[test]
    fn snapshot_then_apply_is_a_round_trip() {
        let schema = motor_schema();
        let mut world = World::new();
        let e = world.spawn((Motor {
            speed: 12.5,
            gear: 3,
            engaged: true,
        },));

        let snap = (schema.snapshot)(&world, e).unwrap();
        assert_eq!(snap[&FieldId(0)], ReflectValue::F64(12.5));
        assert_eq!(snap[&FieldId(1)], ReflectValue::I64(3));
        assert_eq!(snap[&FieldId(2)], ReflectValue::Bool(true));

        *world.get_mut::<Motor>(e).unwrap() = Motor::default();
        (schema.apply)(&mut world, e, &snap).unwrap();
        assert_eq!(
            world.get::<Motor>(e),
            Some(&Motor {
                speed: 12.5,
                gear: 3,
                engaged: true
            })
        );
    }

    #[test]
    fn apply_accepts_a_partial_record() {
        let schema = motor_schema();
        let mut world = World::new();
        let e = world.spawn((Motor {
            speed: 1.0,
            gear: 1,
            engaged: false,
        },));

        let mut patch = ReflectObject::new();
        patch.insert(FieldId(0), ReflectValue::F64(9.0));
        (schema.apply)(&mut world, e, &patch).unwrap();

        let motor = world.get::<Motor>(e).unwrap();
        assert!((motor.speed - 9.0).abs() < f32::EPSILON);
        assert_eq!(motor.gear, 1, "untouched fields keep their value");
        assert!(!motor.engaged);
    }

    #[test]
    fn apply_leaves_the_component_untouched_when_a_field_is_wrong() {
        let schema = motor_schema();
        let mut world = World::new();
        let before = Motor {
            speed: 4.0,
            gear: 2,
            engaged: true,
        };
        let e = world.spawn((before,));

        let mut bad = ReflectObject::new();
        bad.insert(FieldId(0), ReflectValue::F64(7.0)); // fine
        bad.insert(FieldId(2), ReflectValue::I64(1)); // bool field, not an int
        let err = (schema.apply)(&mut world, e, &bad).unwrap_err();
        assert!(matches!(
            err,
            ReflectError::TypeMismatch {
                field: "engaged",
                ..
            }
        ));
        assert_eq!(world.get::<Motor>(e), Some(&before), "no partial write");
    }

    #[test]
    fn narrowing_an_out_of_range_integer_is_rejected() {
        let schema = motor_schema();
        let mut world = World::new();
        let e = world.spawn((Motor::default(),));

        let mut patch = ReflectObject::new();
        patch.insert(FieldId(1), ReflectValue::I64(70_000)); // u8 field
        let err = (schema.apply)(&mut world, e, &patch).unwrap_err();
        assert!(matches!(
            err,
            ReflectError::OutOfRange { field: "gear", .. }
        ));
    }

    #[test]
    fn declared_ranges_are_enforced_by_validate() {
        let schema = motor_schema();
        let speed = schema.field_by_name("speed").unwrap();
        assert!(speed.validate(&ReflectValue::F64(15.0)).is_ok());
        assert!(matches!(
            speed.validate(&ReflectValue::F64(31.0)),
            Err(ReflectError::OutOfRange { .. })
        ));
        assert!(matches!(
            speed.validate(&ReflectValue::F64(f64::NAN)),
            Err(ReflectError::NotFinite { .. })
        ));
        // An integer is an acceptable number.
        assert!(speed.validate(&ReflectValue::I64(4)).is_ok());
    }

    #[test]
    fn insert_default_and_remove_go_through_migration() {
        let schema = motor_schema();
        let mut world = World::new();
        let e = world.spawn((Anchor::default(),));

        (schema.insert_default)(&mut world, e).unwrap();
        assert_eq!(world.get::<Motor>(e), Some(&Motor::default()));
        assert!((schema.remove)(&mut world, e).unwrap());
        assert_eq!(world.get::<Motor>(e), None);
        assert_eq!(world.get::<Anchor>(e), Some(&Anchor::default()));
    }

    #[test]
    fn applying_to_an_entity_without_the_component_reports_it() {
        let schema = motor_schema();
        let mut world = World::new();
        let e = world.spawn((Anchor::default(),));
        let err = (schema.apply)(&mut world, e, &ReflectObject::new()).unwrap_err();
        assert!(matches!(err, ReflectError::MissingComponent(_)));
    }

    #[test]
    fn registry_iteration_is_sorted_by_stable_id() {
        let mut registry = TypeRegistry::new();
        registry.register(motor_schema());
        registry.register(anchor_schema());

        let names: Vec<_> = registry.iter().map(|s| s.stable_id.as_str()).collect();
        assert_eq!(names, vec!["test.Anchor", "test.Motor"]);
        assert!(registry.by_name("test.Motor").is_some());
        assert!(
            registry
                .by_component_id(ComponentId::of::<Motor>())
                .is_some()
        );
        assert!(registry.by_name("test.Missing").is_none());
    }

    #[test]
    fn schemas_on_reports_only_the_components_present() {
        let mut registry = TypeRegistry::new();
        registry.register(motor_schema());
        registry.register(anchor_schema());

        let mut world = World::new();
        let e = world.spawn((Anchor::default(),));
        let names: Vec<_> = registry
            .schemas_on(&world, e)
            .iter()
            .map(|s| s.stable_id.as_str())
            .collect();
        assert_eq!(names, vec!["test.Anchor"]);

        world.insert_component(e, Motor::default()).unwrap();
        let names: Vec<_> = registry
            .schemas_on(&world, e)
            .iter()
            .map(|s| s.stable_id.as_str())
            .collect();
        assert_eq!(names, vec!["test.Anchor", "test.Motor"]);

        world.despawn(e);
        assert!(registry.schemas_on(&world, e).is_empty());
    }

    #[test]
    #[should_panic(expected = "duplicate component stable id")]
    fn registering_the_same_stable_id_twice_panics() {
        let mut registry = TypeRegistry::new();
        registry.register(motor_schema());
        registry.register(motor_schema());
    }

    #[test]
    fn array_fields_round_trip() {
        let schema = anchor_schema();
        let mut world = World::new();
        let e = world.spawn((Anchor {
            offset: [1.0, 2.0, 3.0],
        },));
        let snap = (schema.snapshot)(&world, e).unwrap();
        assert_eq!(snap[&FieldId(0)], ReflectValue::Vec3([1.0, 2.0, 3.0]));

        *world.get_mut::<Anchor>(e).unwrap() = Anchor::default();
        (schema.apply)(&mut world, e, &snap).unwrap();
        assert_eq!(world.get::<Anchor>(e).unwrap().offset, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn defaults_fill_only_absent_fields() {
        let schema = motor_schema();
        let mut record = ReflectObject::new();
        record.insert(FieldId(0), ReflectValue::F64(2.0));
        schema.apply_defaults(&mut record);
        assert_eq!(record.len(), 3);
        assert_eq!(record[&FieldId(0)], ReflectValue::F64(2.0));
        assert_eq!(record[&FieldId(1)], ReflectValue::I64(0));
    }
}
