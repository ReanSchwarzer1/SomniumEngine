//! Schema-driven Details model.
//!
//! The widget builder consumes these rows. Keeping schema traversal pure makes
//! completeness, search, favourites, visibility, and revert semantics testable
//! without a window or renderer.

use somnium_ecs::reflect::{
    ComponentSchema, FieldFlags, FieldId, FieldType, ReflectObject, ReflectValue, StableId,
};

use super::editing_rules::{Editability, EditingRulesRegistry};
use super::property_editors::{PropertyEditorKind, PropertyEditorRegistry};

/// A generated `PropertyRow`, addressed durably rather than by a positional UI
/// enum. The actual control is selected by `editor`.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedPropertyRow {
    pub component: StableId,
    pub field: FieldId,
    pub name: &'static str,
    pub label: String,
    pub group: Option<&'static str>,
    pub doc: Option<&'static str>,
    pub editor: PropertyEditorKind,
    pub ty: FieldType,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub soft_min: Option<f64>,
    pub soft_max: Option<f64>,
    pub precision: Option<u8>,
    pub unit: Option<&'static str>,
    /// CONTROL-K: the declared response of this field's scrub track.
    pub slider: somnium_ecs::curve::SliderCurve,
    pub asset_kind_mask: u64,
    pub value: ReflectValue,
    pub default: ReflectValue,
    pub modified: bool,
    pub read_only: bool,
    /// The selection does not agree on this field's value.
    ///
    /// Unity's convention, and the reason it is a flag rather than a sentinel
    /// value: the row must be able to show that it *has* no single value while
    /// still knowing what the primary's value is, so an untouched mixed row is
    /// never written back.
    pub mixed: bool,
}

/// One reflected component section in Details. Core snapshots the world and
/// hands this renderer-neutral model to the UI; widgets never borrow the ECS.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedComponentPanel {
    pub component: StableId,
    pub label: String,
    /// Optional asset preview rendered above generated rows. This remains
    /// generic: any schema-backed asset type can opt into the shared atlas.
    pub preview_path: Option<std::path::PathBuf>,
    pub rows: Vec<GeneratedPropertyRow>,
}

pub fn generate_component_panel(
    schema: &ComponentSchema,
    values: &ReflectObject,
    editors: &PropertyEditorRegistry,
    rules: &EditingRulesRegistry,
) -> GeneratedComponentPanel {
    GeneratedComponentPanel {
        component: schema.stable_id,
        label: schema.display_name.to_owned(),
        preview_path: None,
        rows: generate_property_rows(schema, values, editors, rules),
    }
}

/// The synthetic multi-selection target, built Godot's way: the intersection
/// of the selection's schemas, matched on `StableId`, `FieldId` **and**
/// `FieldType`. `FieldType::accepts` is the type half, and it is the enum arm
/// that earns the strictness — two `I64`s are indistinguishable until you ask
/// which variant list they index, so a value outside the declared set drops
/// the row rather than merging into one that would write an invalid variant.
///
/// Details learns nothing from this. It receives the same
/// [`GeneratedComponentPanel`] it always receives; only `mixed` is new, and a
/// single selection simply never sets it.
pub fn generate_multi_component_panel(
    schema: &ComponentSchema,
    primary: &ReflectObject,
    others: &[ReflectObject],
    editors: &PropertyEditorRegistry,
    rules: &EditingRulesRegistry,
) -> GeneratedComponentPanel {
    let mut panel = generate_component_panel(schema, primary, editors, rules);
    panel.rows.retain_mut(|row| {
        let field = match schema.field(row.field) {
            Some(field) => field,
            None => return false,
        };
        for other in others {
            match other.get(&row.field) {
                // A member that lacks the field, or types the field
                // differently, drops the row out of the intersection entirely
                // rather than being silently overwritten.
                None => return false,
                Some(value) if !field.ty.accepts(value) => return false,
                Some(value) => row.mixed |= *value != row.value,
            }
        }
        true
    });
    // A mixed row has no single value, so the modified dot — which means
    // "differs from the default" — cannot honestly be lit.
    for row in &mut panel.rows {
        if row.mixed {
            row.modified = false;
        }
    }
    panel
}

pub fn generate_property_rows(
    schema: &ComponentSchema,
    values: &ReflectObject,
    editors: &PropertyEditorRegistry,
    rules: &EditingRulesRegistry,
) -> Vec<GeneratedPropertyRow> {
    let policy = rules.for_component(schema.stable_id);
    let mut rows: Vec<_> = schema
        .fields
        .iter()
        .filter(|field| field.flags.contains(FieldFlags::EDIT))
        .filter(|field| !policy.is_some_and(|rule| rule.is_hidden(field.id, values)))
        .filter_map(|field| {
            let value = values.get(&field.id)?.clone();
            let read_only = field.read_only
                || matches!(
                    policy.map(|rule| rule.is_read_only(field.id, values)),
                    Some(Editability::ReadOnly)
                );
            let label = field
                .display_name
                .map(str::to_owned)
                .unwrap_or_else(|| humanize(field.name));
            Some(GeneratedPropertyRow {
                component: schema.stable_id,
                field: field.id,
                name: field.name,
                label,
                group: field.group,
                doc: field.doc,
                editor: editors.for_type(&field.ty).kind(),
                ty: field.ty.clone(),
                min: field.min,
                max: field.max,
                step: field.step,
                soft_min: field.soft_min,
                soft_max: field.soft_max,
                precision: field.precision,
                unit: field.unit,
                slider: field.slider,
                asset_kind_mask: field.asset_kind_mask,
                modified: value != field.default,
                default: field.default.clone(),
                value,
                read_only,
                mixed: false,
            })
        })
        .collect();
    rows.sort_by_key(|row| {
        schema
            .field(row.field)
            .and_then(|field| field.order)
            .unwrap_or(i32::from(row.field.0))
    });
    rows
}

pub fn search_rows<'a>(
    rows: &'a [GeneratedPropertyRow],
    query: &str,
) -> Vec<&'a GeneratedPropertyRow> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return rows.iter().collect();
    }
    rows.iter()
        .filter(|row| {
            row.label.to_ascii_lowercase().contains(&needle)
                || row
                    .doc
                    .is_some_and(|doc| doc.to_ascii_lowercase().contains(&needle))
        })
        .collect()
}

fn humanize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = true;
    for ch in name.chars() {
        if ch == '_' {
            out.push(' ');
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::{Component, component_schema};

    #[derive(Clone, Default)]
    struct AllShapes {
        yes: bool,
        count: i32,
        amount: f32,
        title: String,
        p2: [f32; 2],
        p3: [f32; 3],
        p4: [f32; 4],
    }
    impl Component for AllShapes {}

    #[test]
    fn adding_a_component_needs_no_panel_code() {
        let schema = component_schema! {
            AllShapes as "test.AllShapes", display "All Shapes", version 1,
            fields { yes, count, amount, title, p2, p3, p4 }
        };
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((AllShapes::default(),));
        let values = (schema.snapshot)(&world, entity).unwrap();
        let rows = generate_property_rows(
            &schema,
            &values,
            &PropertyEditorRegistry::standard(),
            &EditingRulesRegistry::default(),
        );
        assert_eq!(rows.len(), schema.fields.len());
        assert!(
            rows.iter()
                .all(|row| row.editor != PropertyEditorKind::Unsupported)
        );
    }

    /// The intersection marks disagreements and leaves agreement alone.
    #[test]
    fn a_multi_panel_marks_only_the_rows_that_disagree() {
        let schema = somnium_asset::material::material_asset_schema();
        let mut world = somnium_ecs::World::new();
        let a = world.spawn((somnium_asset::material::MaterialAsset::default(),));
        let b = world.spawn((somnium_asset::material::MaterialAsset {
            roughness: 0.2,
            ..somnium_asset::material::MaterialAsset::default()
        },));
        let primary = (schema.snapshot)(&world, a).unwrap();
        let other = (schema.snapshot)(&world, b).unwrap();

        let panel = generate_multi_component_panel(
            &schema,
            &primary,
            std::slice::from_ref(&other),
            &PropertyEditorRegistry::standard(),
            &EditingRulesRegistry::default(),
        );
        let roughness = panel
            .rows
            .iter()
            .find(|row| row.name == "roughness")
            .expect("roughness is an editable material field");
        assert!(roughness.mixed, "differing values must read as mixed");
        assert!(
            !roughness.modified,
            "a mixed row has no single value, so it cannot claim to differ from the default"
        );
        assert!(
            panel
                .rows
                .iter()
                .any(|row| row.name == "metallic" && !row.mixed),
            "agreeing rows stay ordinary"
        );
    }

    /// One selected entity is the degenerate case and must produce exactly the
    /// single-selection panel, mixed flags included.
    #[test]
    fn a_multi_panel_of_one_is_the_single_selection_panel() {
        let schema = somnium_asset::material::material_asset_schema();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((somnium_asset::material::MaterialAsset::default(),));
        let values = (schema.snapshot)(&world, entity).unwrap();
        let editors = PropertyEditorRegistry::standard();
        let rules = EditingRulesRegistry::default();

        assert_eq!(
            generate_multi_component_panel(&schema, &values, &[], &editors, &rules),
            generate_component_panel(&schema, &values, &editors, &rules)
        );
    }

    /// A member missing the field drops the row out of the intersection
    /// rather than letting the primary's value stand in for it.
    #[test]
    fn a_member_missing_the_field_drops_the_row() {
        let schema = somnium_asset::material::material_asset_schema();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((somnium_asset::material::MaterialAsset::default(),));
        let primary = (schema.snapshot)(&world, entity).unwrap();
        let mut partial = primary.clone();
        let dropped = *primary.keys().next().unwrap();
        partial.remove(&dropped);

        let panel = generate_multi_component_panel(
            &schema,
            &primary,
            std::slice::from_ref(&partial),
            &PropertyEditorRegistry::standard(),
            &EditingRulesRegistry::default(),
        );
        assert!(panel.rows.iter().all(|row| row.field != dropped));
    }

    #[test]
    fn search_includes_schema_documentation() {
        let row = GeneratedPropertyRow {
            component: StableId::new("x"),
            field: FieldId(0),
            name: "speed",
            label: "Speed".into(),
            group: None,
            doc: Some("metres per second"),
            editor: PropertyEditorKind::Number,
            ty: FieldType::F64,
            min: None,
            max: None,
            step: None,
            soft_min: None,
            soft_max: None,
            precision: None,
            unit: None,
            slider: somnium_ecs::curve::SliderCurve::Linear,
            asset_kind_mask: u64::MAX,
            value: ReflectValue::F64(1.0),
            default: ReflectValue::F64(0.0),
            modified: true,
            read_only: false,
            mixed: false,
        };
        assert_eq!(search_rows(&[row], "metres").len(), 1);
    }

    #[test]
    fn material_details_are_entirely_schema_generated() {
        let schema = somnium_asset::material::material_asset_schema();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((somnium_asset::material::MaterialAsset::default(),));
        let values = (schema.snapshot)(&world, entity).unwrap();
        let panel = generate_component_panel(
            &schema,
            &values,
            &PropertyEditorRegistry::standard(),
            &EditingRulesRegistry::default(),
        );
        assert_eq!(panel.rows.len(), 16);
        assert!(
            panel
                .rows
                .iter()
                .all(|row| row.editor != PropertyEditorKind::Unsupported)
        );
        assert_eq!(
            panel
                .rows
                .iter()
                .filter(|row| row.ty == FieldType::Asset)
                .count(),
            5
        );
    }
}
