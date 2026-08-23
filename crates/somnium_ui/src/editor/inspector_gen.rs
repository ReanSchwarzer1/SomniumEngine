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
    pub asset_kind_mask: u64,
    pub value: ReflectValue,
    pub default: ReflectValue,
    pub modified: bool,
    pub read_only: bool,
}

/// One reflected component section in Details. Core snapshots the world and
/// hands this renderer-neutral model to the UI; widgets never borrow the ECS.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedComponentPanel {
    pub component: StableId,
    pub label: String,
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
        rows: generate_property_rows(schema, values, editors, rules),
    }
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
                asset_kind_mask: field.asset_kind_mask,
                modified: value != field.default,
                default: field.default.clone(),
                value,
                read_only,
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
            asset_kind_mask: u64::MAX,
            value: ReflectValue::F64(1.0),
            default: ReflectValue::F64(0.0),
            modified: true,
            read_only: false,
        };
        assert_eq!(search_rows(&[row], "metres").len(), 1);
    }
}
