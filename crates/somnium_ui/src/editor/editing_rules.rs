//! Conditional editing policy layered over component schemas.

use std::collections::HashMap;

use somnium_ecs::reflect::{FieldId, ReflectObject, ReflectValue, StableId, TypeRegistry};

/// A rule's opinion about editability. `Unhandled` deliberately differs from
/// `Editable`, allowing metadata and multiple policy layers to compose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editability {
    Editable,
    ReadOnly,
    Unhandled,
}

/// Runtime policy for conditions that do not belong in the schema macro.
pub trait EditingRules: Send + Sync {
    fn is_hidden(&self, _field: FieldId, _value: &ReflectObject) -> bool {
        false
    }
    fn is_read_only(&self, _field: FieldId, _value: &ReflectObject) -> Editability {
        Editability::Unhandled
    }
    fn validate(&self, _field: FieldId, _value: &ReflectObject) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
pub struct EditingRulesRegistry {
    rules: HashMap<StableId, Box<dyn EditingRules>>,
}

impl EditingRulesRegistry {
    pub fn register(&mut self, component: StableId, rules: impl EditingRules + 'static) {
        self.rules.insert(component, Box::new(rules));
    }

    pub fn for_component(&self, component: StableId) -> Option<&dyn EditingRules> {
        self.rules.get(&component).map(Box::as_ref)
    }
}

#[derive(Default)]
struct ConditionalRules {
    hidden: Vec<(FieldId, Condition)>,
}

enum Condition {
    Bool(FieldId, bool),
    Enum(FieldId, i64),
    Positive(FieldId),
}

impl Condition {
    fn matches(&self, values: &ReflectObject) -> bool {
        match self {
            Self::Bool(id, expected) => values.get(id) == Some(&ReflectValue::Bool(*expected)),
            Self::Enum(id, expected) => values.get(id) == Some(&ReflectValue::I64(*expected)),
            Self::Positive(id) => {
                matches!(values.get(id), Some(ReflectValue::F64(value)) if *value > 0.0)
            }
        }
    }
}

impl EditingRules for ConditionalRules {
    fn is_hidden(&self, field: FieldId, values: &ReflectObject) -> bool {
        self.hidden
            .iter()
            .find(|(candidate, _)| *candidate == field)
            .is_some_and(|(_, condition)| !condition.matches(values))
    }
}

/// Built-in conditional visibility policies. Names are resolved once through
/// the schema, while per-frame evaluation uses compact `FieldId`s.
pub fn standard_editing_rules(types: &TypeRegistry) -> EditingRulesRegistry {
    let mut registry = EditingRulesRegistry::default();
    let field = |component: &str, name: &str| {
        types
            .by_name(component)
            .and_then(|schema| schema.field_by_name(name))
            .map(|field| field.id)
    };

    if let Some(schema) = types.by_name("somnium.PostProcess") {
        let mut rules = ConditionalRules::default();
        let gated = [
            ("bloom_enabled", &["bloom_intensity"][..]),
            ("dof_enabled", &["dof_focus_distance"][..]),
            (
                "use_physical_camera",
                &["aperture_f_stops", "shutter_speed_s", "sensitivity_iso"][..],
            ),
            ("gtao_enabled", &["gtao_radius", "gtao_intensity"][..]),
            (
                "volumetrics_enabled",
                &[
                    "light_shafts",
                    "fog_density",
                    "fog_height_falloff",
                    "fog_asymmetry",
                    "shaft_intensity",
                ][..],
            ),
            ("cas_enabled", &["cas_sharpness", "cas_strength"][..]),
            ("motion_blur_enabled", &["motion_blur_shutter"][..]),
            ("world_cache", &["cache_intensity", "cache_cell_size"][..]),
            ("specular_gi", &["spec_roughness"][..]),
            ("path_tracer", &["path_bounces"][..]),
            ("probes", &["probe_intensity"][..]),
            ("fsr_enabled", &["fsr_sharpness"][..]),
        ];
        for (controller, targets) in gated {
            let Some(controller) = schema.field_by_name(controller).map(|item| item.id) else {
                continue;
            };
            for target in targets {
                if let Some(target) = schema.field_by_name(target).map(|item| item.id) {
                    rules
                        .hidden
                        .push((target, Condition::Bool(controller, true)));
                }
            }
        }
        registry.register(schema.stable_id, rules);
    }

    if let (Some(schema), Some(kind)) = (
        types.by_name("somnium.Light"),
        field("somnium.Light", "light_type"),
    ) {
        let mut rules = ConditionalRules::default();
        for name in ["inner_angle", "outer_angle"] {
            if let Some(target) = schema.field_by_name(name) {
                rules.hidden.push((target.id, Condition::Enum(kind, 2)));
            }
        }
        registry.register(schema.stable_id, rules);
    }

    if let (Some(schema), Some(blend)) = (
        types.by_name("somnium.Water"),
        field("somnium.Water", "spectrum_blend"),
    ) {
        let mut rules = ConditionalRules::default();
        for name in ["wind_speed", "foam_decay", "foam_threshold"] {
            if let Some(target) = schema.field_by_name(name) {
                rules.hidden.push((target.id, Condition::Positive(blend)));
            }
        }
        registry.register(schema.stable_id, rules);
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Hidden;
    impl EditingRules for Hidden {
        fn is_hidden(&self, field: FieldId, _: &ReflectObject) -> bool {
            field == FieldId(2)
        }
    }

    #[test]
    fn registry_is_keyed_by_durable_component_identity() {
        let id = StableId::new("test.Component");
        let mut registry = EditingRulesRegistry::default();
        registry.register(id, Hidden);
        assert!(
            registry
                .for_component(id)
                .unwrap()
                .is_hidden(FieldId(2), &ReflectObject::new())
        );
    }
}
