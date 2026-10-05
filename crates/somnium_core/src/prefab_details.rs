//! Read-only prefab provenance and per-field overrides for Details.
use somnium_ecs::{Component, Entity, TypeRegistry, World, component_schema};

#[derive(Default, Clone)]
struct PrefabDetails {
    template: String,
    path: String,
    overrides: String,
    status: String,
}
impl Component for PrefabDetails {}
pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        PrefabDetails as "somnium.PrefabDetails", display "Prefab Instance", version 1,
        fields {
            template { display_name:"Template",read_only:true,flags:somnium_ecs::FieldFlags::EDIT },
            path { display_name:"Instance Path",read_only:true,flags:somnium_ecs::FieldFlags::EDIT },
            overrides { display_name:"Field Overrides",read_only:true,flags:somnium_ecs::FieldFlags::EDIT },
            status { display_name:"Editing",read_only:true,flags:somnium_ecs::FieldFlags::EDIT },
        }
    });
}
pub(crate) fn refresh(world: &mut World, entity: Entity, registry: &TypeRegistry) -> usize {
    let Some(member) = world.get::<crate::prefab::PrefabMember>(entity).cloned() else {
        let _ = world.remove_component::<PrefabDetails>(entity);
        return 0;
    };
    let overrides = crate::prefab::field_overrides(world, registry, entity).unwrap_or_default();
    let names = overrides
        .iter()
        .filter_map(|item| {
            let schema = registry.by_stable_id(item.component)?;
            let field = schema.field(item.field)?;
            Some(format!(
                "{}.{}",
                schema.display_name,
                field.display_name.unwrap_or(field.name)
            ))
        })
        .collect::<Vec<_>>();
    let editing = somnium_ecs::PersistentId::parse_hex(&member.root)
        .and_then(|id| world.entity_by_persistent_id(id))
        .is_some_and(|root| world.get::<crate::prefab::EditingInstance>(root).is_some());
    let detail = PrefabDetails {
        template: member.source,
        path: member.path.join(" / "),
        overrides: if names.is_empty() {
            "None — template defaults".into()
        } else {
            names.join(", ")
        },
        status: format!(
            "{}; {} retained orphan patches. Create menu: Revert, Propagate, Break Link. Rebuilding fields require Break Link.",
            if editing {
                "Editing this instance"
            } else {
                "Instance selected"
            },
            member.orphaned.len()
        ),
    };
    let _ = world.insert_component(entity, detail);
    overrides.len()
}
