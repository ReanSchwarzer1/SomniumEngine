//! Phase 16-A: the engine's built-in components, described once.
//!
//! Everything in this file exists so that no *other* file has to know
//! what fields a component has. Before this, the scene serializer knew,
//! and the inspector knew separately, and a script binding layer would
//! have had to know a third time. Three descriptions of the same struct
//! drift; this is one.
//!
//! Adding a component to the engine means adding one
//! [`component_schema!`](somnium_ecs::component_schema) block here.
//! Nothing else needs editing to make it saveable, inspectable and
//! script-visible.

use somnium_ecs::component_schema;
use somnium_ecs::reflect::{
    ComponentSchema, FieldFlags, FieldId, FieldSchema, FieldType, ReflectError, ReflectField,
    ReflectObject, ReflectValue, StableId, TypeRegistry,
};
use somnium_ecs::{Entity, World};

use crate::{
    FoliageComponent, LightComponent, LightType, MaterialComponent, MeshComponent, MeshKind, Name,
    Parent, TerrainComponent, Transform, VoxelTerrainComponent, WaterComponent,
};

// `MeshComponent` and `MaterialComponent` derive `Default` at their
// definitions — all-zero is right for both. The impls below are the ones
// where all-zero would be wrong.

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Defaults
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// A schema needs a default instance for two reasons: to fill in the
// declared field defaults, and to construct the component when a script
// or the editor attaches one. Several of these types had no `Default`
// because nothing needed one before — and `Transform` deliberately does
// not derive it, since a derived default would have zero scale and make
// every entity it touched invisible.

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::ONE,
        }
    }
}

impl Default for LightComponent {
    /// A midday-sun directional light. Directional is the only light kind
    /// that is fully functional, so a script or editor attaching a light
    /// with no further configuration gets one that is actually visible.
    fn default() -> Self {
        Self::directional(crate::light_units::lux::DIRECT_SUNLIGHT)
    }
}

impl Default for Parent {
    fn default() -> Self {
        Self {
            entity: Entity::DANGLING,
        }
    }
}

impl Default for TerrainComponent {
    fn default() -> Self {
        Self {
            terrain_id: 0,
            chunk_cells: 64,
            grid_x: 1,
            grid_z: 1,
            cell_size: 1.0,
            height_scale: 1.0,
        }
    }
}

impl Default for MeshKind {
    fn default() -> Self {
        Self::Cube
    }
}

impl Default for LightType {
    fn default() -> Self {
        Self::Directional
    }
}

impl Default for Name {
    fn default() -> Self {
        Self::new("")
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Field conversions for engine enums
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Enums travel as an integer restricted to a named variant list, so an
// editor can draw a dropdown and a script can compare against a name
// without either of them hard-coding the numbering.

/// Variant names for [`LightType`]. Order is the wire order; appending is
/// safe, reordering is not.
const LIGHT_TYPE_NAMES: &[&str] = &[
    "Directional",
    "Point",
    "Spot",
    "Rect",
    "Disc",
    "Tube",
];

impl ReflectField for LightType {
    fn field_type() -> FieldType {
        FieldType::Enum(LIGHT_TYPE_NAMES)
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::I64(match self {
            Self::Directional => 0,
            Self::Point => 1,
            Self::Spot => 2,
            Self::Rect => 3,
            Self::Disc => 4,
            Self::Tube => 5,
        })
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::I64(0) => Ok(Self::Directional),
            ReflectValue::I64(1) => Ok(Self::Point),
            ReflectValue::I64(2) => Ok(Self::Spot),
            ReflectValue::I64(3) => Ok(Self::Rect),
            ReflectValue::I64(4) => Ok(Self::Disc),
            ReflectValue::I64(5) => Ok(Self::Tube),
            ReflectValue::I64(_) => Err(ReflectError::OutOfRange {
                field,
                min: Some(0.0),
                max: Some(5.0),
            }),
            other => Err(ReflectError::TypeMismatch {
                field,
                expected: "LightType".into(),
                found: other.kind(),
            }),
        }
    }
}

/// Variant names for [`MeshKind`].
const MESH_KIND_NAMES: &[&str] = &["Cube", "Sphere", "Plane", "Cylinder"];

impl ReflectField for MeshKind {
    fn field_type() -> FieldType {
        FieldType::Enum(MESH_KIND_NAMES)
    }
    fn to_reflect(&self) -> ReflectValue {
        ReflectValue::I64(match self {
            Self::Cube => 0,
            Self::Sphere => 1,
            Self::Plane => 2,
            Self::Cylinder => 3,
        })
    }
    fn from_reflect(value: &ReflectValue, field: &'static str) -> Result<Self, ReflectError> {
        match value {
            ReflectValue::I64(0) => Ok(Self::Cube),
            ReflectValue::I64(1) => Ok(Self::Sphere),
            ReflectValue::I64(2) => Ok(Self::Plane),
            ReflectValue::I64(3) => Ok(Self::Cylinder),
            ReflectValue::I64(_) => Err(ReflectError::OutOfRange {
                field,
                min: Some(0.0),
                max: Some(3.0),
            }),
            other => Err(ReflectError::TypeMismatch {
                field,
                expected: "MeshKind".into(),
                found: other.kind(),
            }),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Hand-written schemas
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Two components do not fit the macro, and it is worth being explicit
// about why rather than bending the macro into something unreadable:
// `Name` is a tuple struct over a fixed byte array that presents as a
// string, and `MeshKind` is a fieldless enum that presents as a single
// value. Both are one-field schemas written out once.

/// `Name` presents its fixed 64-byte buffer as an ordinary string field.
fn name_schema() -> ComponentSchema {
    fn snapshot(world: &World, entity: Entity) -> Option<ReflectObject> {
        let name = world.get::<Name>(entity)?;
        let mut out = ReflectObject::new();
        out.insert(FieldId(0), ReflectValue::Str(name.as_str().to_owned()));
        Some(out)
    }

    fn apply(world: &mut World, entity: Entity, record: &ReflectObject) -> Result<(), ReflectError> {
        let Some(value) = record.get(&FieldId(0)) else {
            return Ok(());
        };
        let ReflectValue::Str(text) = value else {
            return Err(ReflectError::TypeMismatch {
                field: "value",
                expected: "string".into(),
                found: value.kind(),
            });
        };
        let slot = world
            .get_mut::<Name>(entity)
            .ok_or(ReflectError::MissingComponent(StableId::new("somnium.Name")))?;
        // `Name::new` truncates past 63 bytes rather than failing, which
        // is the existing contract and stays the contract here.
        *slot = Name::new(text);
        Ok(())
    }

    fn read_field(world: &World, entity: Entity, id: FieldId) -> Option<ReflectValue> {
        (id == FieldId(0))
            .then(|| world.get::<Name>(entity))
            .flatten()
            .map(|name| ReflectValue::Str(name.as_str().to_owned()))
    }

    fn insert_default(world: &mut World, entity: Entity) -> Result<(), ReflectError> {
        world.insert_component(entity, Name::default()).map_err(Into::into)
    }

    fn remove(world: &mut World, entity: Entity) -> Result<bool, ReflectError> {
        world.remove_component::<Name>(entity).map_err(Into::into)
    }

    ComponentSchema {
        stable_id: StableId::new("somnium.Name"),
        display_name: "Name",
        version: 1,
        fields: vec![FieldSchema {
            name: "value",
            id: FieldId(0),
            ty: FieldType::Str,
            default: ReflectValue::Str(String::new()),
            min: None,
            max: None,
            flags: FieldFlags::DEFAULT,
        }],
        component_id: somnium_ecs::ComponentId::of::<Name>(),
        snapshot,
        read_field,
        apply,
        insert_default,
        remove,
        migrate: None,
    }
}

/// `MeshKind` is a bare enum; its schema is the single `kind` field.
fn mesh_kind_schema() -> ComponentSchema {
    fn snapshot(world: &World, entity: Entity) -> Option<ReflectObject> {
        let kind = world.get::<MeshKind>(entity)?;
        let mut out = ReflectObject::new();
        out.insert(FieldId(0), kind.to_reflect());
        Some(out)
    }

    fn apply(world: &mut World, entity: Entity, record: &ReflectObject) -> Result<(), ReflectError> {
        let Some(value) = record.get(&FieldId(0)) else {
            return Ok(());
        };
        let parsed = MeshKind::from_reflect(value, "kind")?;
        let slot = world
            .get_mut::<MeshKind>(entity)
            .ok_or(ReflectError::MissingComponent(StableId::new(
                "somnium.MeshKind",
            )))?;
        *slot = parsed;
        Ok(())
    }

    fn read_field(world: &World, entity: Entity, id: FieldId) -> Option<ReflectValue> {
        (id == FieldId(0))
            .then(|| world.get::<MeshKind>(entity))
            .flatten()
            .map(ReflectField::to_reflect)
    }

    fn insert_default(world: &mut World, entity: Entity) -> Result<(), ReflectError> {
        world
            .insert_component(entity, MeshKind::default())
            .map_err(Into::into)
    }

    fn remove(world: &mut World, entity: Entity) -> Result<bool, ReflectError> {
        world.remove_component::<MeshKind>(entity).map_err(Into::into)
    }

    ComponentSchema {
        stable_id: StableId::new("somnium.MeshKind"),
        display_name: "Mesh Kind",
        version: 1,
        fields: vec![FieldSchema {
            name: "kind",
            id: FieldId(0),
            ty: FieldType::Enum(MESH_KIND_NAMES),
            default: ReflectValue::I64(0),
            min: None,
            max: None,
            flags: FieldFlags::DEFAULT,
        }],
        component_id: somnium_ecs::ComponentId::of::<MeshKind>(),
        snapshot,
        read_field,
        apply,
        insert_default,
        remove,
        migrate: None,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// The registry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build the engine's component registry.
///
/// Called once at startup. Registration order is irrelevant — the
/// registry sorts by stable id — but the list is kept alphabetical so a
/// reader can tell at a glance what is and is not described yet.
#[must_use]
pub fn component_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();

    registry.register(foliage_schema());
    registry.register(light_schema());
    registry.register(material_schema());
    registry.register(mesh_schema());
    registry.register(mesh_kind_schema());
    registry.register(name_schema());
    registry.register(parent_schema());
    registry.register(terrain_schema());
    registry.register(transform_schema());
    registry.register(voxel_terrain_schema());
    registry.register(water_schema());

    registry
}

/// Water is the largest schema in the engine and the reason the macro
/// exists: version 1 spells all of this out a second time in
/// [`scene_serial`](crate::scene_serial), by hand, and the two could
/// silently disagree.
///
/// The three `[f32; 4]` colours are `Vec4` rather than `Color` because
/// they carry alpha, and `bounds` is a `Vec4` used as
/// `[min_x, min_z, max_x, max_z]` — geometry, not a colour.
fn water_schema() -> ComponentSchema {
    component_schema! {
        WaterComponent as "somnium.Water", display "Water", version 1,
        fields {
            // Renderer-owned handles. Saved, because a body is meaningless
            // without them, but not script-writable: pointing a water body
            // at another terrain's bathymetry at runtime is not a thing
            // gameplay should be able to do.
            water_id { flags: FieldFlags::SERIALIZE.union(FieldFlags::SCRIPT_READ) },
            terrain_id { flags: FieldFlags::SERIALIZE.union(FieldFlags::SCRIPT_READ) },
            preset,
            body_kind,
            surface_level,
            max_depth,
            bounds,
            enabled,
            deep_color,
            shallow_color,
            edge_color,
            clarity { min: 0.0 },
            edge_scale { min: 0.0 },
            amplitude { min: 0.0 },
            coord_scale,
            coord_offset,
            wave_dir_a,
            wave_dir_b,
            wave_blend { min: 0.0, max: 1.0 },
            wave_length_a { min: 0.0 },
            wave_length_b { min: 0.0 },
            wave_speed { min: 0.0 },
            // Above ~1.0 a Gerstner wave self-intersects and the surface
            // turns inside out.
            wave_steepness { min: 0.0, max: 1.0 },
            absorption,
            scattering,
            roughness { min: 0.0, max: 1.0 },
            anisotropy { min: -1.0, max: 1.0 },
            ssr_strength { min: 0.0, max: 1.0 },
            rt_reflect_strength { min: 0.0, max: 1.0 },
            reflect_debug { min: 0.0, max: 2.0 },
            spectrum_blend { min: 0.0, max: 1.0 },
            wind_speed { min: 0.0 },
            foam_decay { min: 0.0, max: 10.0 },
            foam_threshold { min: 0.0 },
            caustic_strength { min: 0.0 },
            underwater_enabled,
        }
    }
}

/// Foliage scatter parameters.
///
/// Version 1 never saved these, so painting foliage and saving lost it.
/// Registering the component fixes that as a side effect of describing it
/// — which is the point of having one registry.
fn foliage_schema() -> ComponentSchema {
    component_schema! {
        FoliageComponent as "somnium.Foliage", display "Foliage", version 1,
        fields {
            enabled,
            density { min: 0.0 },
            seed,
            max_slope_deg { min: 0.0, max: 90.0 },
            layer,
            min_layer_weight { min: 0.0, max: 1.0 },
            scale_min { min: 0.0 },
            scale_max { min: 0.0 },
            radius { min: 0.0 },
            cull_distance { min: 0.0 },
            foliage_shadow_distance { min: 0.0 },
            lod_distance { min: 0.0 },
            impostor_distance { min: 0.0 },
            max_instances,
        }
    }
}

/// Voxel world handle. Like foliage, previously unsaved.
fn voxel_terrain_schema() -> ComponentSchema {
    component_schema! {
        VoxelTerrainComponent as "somnium.VoxelTerrain", display "Voxel Terrain", version 1,
        fields {
            radius_chunks { min: 1 },
            seed,
        }
    }
}

fn transform_schema() -> ComponentSchema {
    component_schema! {
        Transform as "somnium.Transform", display "Transform", version 1,
        fields { translation, rotation, scale }
    }
}

fn light_schema() -> ComponentSchema {
    component_schema! {
        LightComponent as "somnium.Light", display "Light", version 1,
        fields {
            light_type,
            color,
            intensity { min: 0.0 },
            color_temperature_k { min: 0.0, max: 20_000.0 },
            source_radius { min: 0.0 },
            range { min: 0.0 },
            inner_angle { min: 0.0 },
            outer_angle { min: 0.0 },
            moon_intensity { min: 0.0 },
            area_width { min: 0.0 },
            area_height { min: 0.0 },
        }
    }
}

fn mesh_schema() -> ComponentSchema {
    component_schema! {
        MeshComponent as "somnium.Mesh", display "Mesh", version 1,
        // GPU buffer offsets: derived state the renderer owns. Scripts and
        // the inspector may read them; nothing outside the upload path may
        // write them, and they are not saved — a reload recomputes them.
        fields {
            vertex_offset { flags: FieldFlags::RUNTIME_ONLY },
            index_offset { flags: FieldFlags::RUNTIME_ONLY },
            index_count { flags: FieldFlags::RUNTIME_ONLY },
        }
    }
}

fn material_schema() -> ComponentSchema {
    component_schema! {
        MaterialComponent as "somnium.Material", display "Material", version 1,
        fields { id }
    }
}

fn parent_schema() -> ComponentSchema {
    component_schema! {
        Parent as "somnium.Parent", display "Parent", version 1,
        fields { entity }
    }
}

fn terrain_schema() -> ComponentSchema {
    component_schema! {
        TerrainComponent as "somnium.Terrain", display "Terrain", version 1,
        fields {
            terrain_id,
            chunk_cells { min: 1 },
            grid_x { min: 1 },
            grid_z { min: 1 },
            cell_size { min: 0.001 },
            height_scale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_schema_registers_without_a_clash() {
        let registry = component_registry();
        assert_eq!(registry.len(), 11);
        let names: Vec<_> = registry.iter().map(|s| s.stable_id.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "somnium.Foliage",
                "somnium.Light",
                "somnium.Material",
                "somnium.Mesh",
                "somnium.MeshKind",
                "somnium.Name",
                "somnium.Parent",
                "somnium.Terrain",
                "somnium.Transform",
                "somnium.VoxelTerrain",
                "somnium.Water",
            ],
            "iteration is sorted by stable id, not by registration order"
        );
    }

    #[test]
    fn transform_round_trips_through_the_registry() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Transform").unwrap();
        let mut world = World::new();
        let original = Transform {
            translation: glam::Vec3::new(1.0, 2.0, 3.0),
            rotation: glam::Quat::from_rotation_y(0.5),
            scale: glam::Vec3::new(2.0, 2.0, 2.0),
        };
        let e = world.spawn((original,));

        let snap = (schema.snapshot)(&world, e).unwrap();
        *world.get_mut::<Transform>(e).unwrap() = Transform::default();
        (schema.apply)(&mut world, e, &snap).unwrap();

        let after = world.get::<Transform>(e).unwrap();
        assert!((after.translation - original.translation).length() < 1.0e-6);
        assert!(after.rotation.angle_between(original.rotation) < 1.0e-5);
        assert!((after.scale - original.scale).length() < 1.0e-6);
    }

    #[test]
    fn a_default_transform_is_identity_not_zero_scale() {
        let transform = Transform::default();
        assert!((transform.scale - glam::Vec3::ONE).length() < f32::EPSILON);
        assert!(transform.rotation.is_near_identity());
    }

    #[test]
    fn name_presents_its_byte_buffer_as_a_string() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Name").unwrap();
        let mut world = World::new();
        let e = world.spawn((Name::new("Sun Light"),));

        let snap = (schema.snapshot)(&world, e).unwrap();
        assert_eq!(snap[&FieldId(0)], ReflectValue::Str("Sun Light".into()));

        let mut patch = ReflectObject::new();
        patch.insert(FieldId(0), ReflectValue::Str("Moon".into()));
        (schema.apply)(&mut world, e, &patch).unwrap();
        assert_eq!(world.get::<Name>(e).unwrap().as_str(), "Moon");
    }

    #[test]
    fn a_name_longer_than_the_buffer_is_truncated_not_rejected() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Name").unwrap();
        let mut world = World::new();
        let e = world.spawn((Name::new("short"),));

        let long = "x".repeat(200);
        let mut patch = ReflectObject::new();
        patch.insert(FieldId(0), ReflectValue::Str(long));
        (schema.apply)(&mut world, e, &patch).unwrap();
        assert_eq!(world.get::<Name>(e).unwrap().as_str().len(), 63);
    }

    #[test]
    fn enum_fields_travel_as_named_variants() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Light").unwrap();
        let field = schema.field_by_name("light_type").unwrap();
        let FieldType::Enum(names) = &field.ty else {
            panic!("light_type should be an enum field");
        };
        assert_eq!(names[0], "Directional");
        assert_eq!(names[2], "Spot");

        let mut world = World::new();
        let e = world.spawn((LightComponent::point(3.0, 10.0),));
        let snap = (schema.snapshot)(&world, e).unwrap();
        assert_eq!(snap[&field.id], ReflectValue::I64(1), "Point is variant 1");
    }

    #[test]
    fn an_out_of_range_enum_value_is_rejected() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.MeshKind").unwrap();
        let mut world = World::new();
        let e = world.spawn((MeshKind::Cube,));

        let mut patch = ReflectObject::new();
        patch.insert(FieldId(0), ReflectValue::I64(99));
        assert!(matches!(
            (schema.apply)(&mut world, e, &patch),
            Err(ReflectError::OutOfRange { .. })
        ));
        assert_eq!(world.get::<MeshKind>(e), Some(&MeshKind::Cube));
    }

    #[test]
    fn declared_ranges_reject_a_negative_intensity() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Light").unwrap();
        let intensity = schema.field_by_name("intensity").unwrap();
        assert!(intensity.validate(&ReflectValue::F64(-1.0)).is_err());
        assert!(intensity.validate(&ReflectValue::F64(100_000.0)).is_ok());
    }

    #[test]
    fn runtime_only_fields_are_readable_but_not_saved_or_written() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Mesh").unwrap();
        for field in &schema.fields {
            assert!(
                field.flags.contains(FieldFlags::SCRIPT_READ),
                "{} should be readable",
                field.name
            );
            assert!(
                !field.flags.contains(FieldFlags::SERIALIZE),
                "{} is derived from the upload path and must not be saved",
                field.name
            );
            assert!(
                !field.flags.contains(FieldFlags::SCRIPT_WRITE),
                "{} must not be script-writable",
                field.name
            );
        }
    }

    #[test]
    fn a_parent_reference_survives_the_neutral_value_model() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Parent").unwrap();
        let mut world = World::new();
        let parent = world.spawn((Transform::default(),));
        let child = world.spawn((Transform::default(), Parent { entity: parent }));

        let snap = (schema.snapshot)(&world, child).unwrap();
        assert_eq!(snap[&FieldId(0)], ReflectValue::Entity(Some(parent)));

        // An unset parent is `DANGLING` on the way in and `None` on the
        // way out — the sentinel never escapes into script-visible data.
        *world.get_mut::<Parent>(child).unwrap() = Parent::default();
        let snap = (schema.snapshot)(&world, child).unwrap();
        assert_eq!(snap[&FieldId(0)], ReflectValue::Entity(None));
    }

    #[test]
    fn schemas_can_attach_and_detach_their_component() {
        let registry = component_registry();
        let schema = registry.by_name("somnium.Terrain").unwrap();
        let mut world = World::new();
        let e = world.spawn((Transform::default(),));

        assert!(world.get::<TerrainComponent>(e).is_none());
        (schema.insert_default)(&mut world, e).unwrap();
        assert_eq!(
            world.get::<TerrainComponent>(e),
            Some(&TerrainComponent::default())
        );
        assert!((schema.remove)(&mut world, e).unwrap());
        assert!(world.get::<TerrainComponent>(e).is_none());
        assert!(
            world.get::<Transform>(e).is_some(),
            "migration must not disturb the rest of the entity"
        );
    }
}
