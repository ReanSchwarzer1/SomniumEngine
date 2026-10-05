//! Lossless JSON boundary; scene files retain their existing encoding.
use serde_json::{Value, json};
use somnium_ecs::{
    PersistentId, World,
    reflect::{FieldType, ReflectValue},
};

pub fn encode(world: &World, ty: &FieldType, value: &ReflectValue) -> Value {
    match (ty, value) {
        (FieldType::Enum(names), ReflectValue::I64(n)) => names
            .get(*n as usize)
            .map_or_else(|| json!(n.to_string()), |n| json!(n)),
        (_, ReflectValue::I64(n)) => json!(n.to_string()),
        (FieldType::Array(inner), ReflectValue::Array(items)) => {
            Value::Array(items.iter().map(|v| encode(world, inner, v)).collect())
        }
        _ => crate::scene_schema::value_to_json(world, value),
    }
}
pub fn decode(world: &World, ty: &FieldType, value: &Value) -> Result<ReflectValue, String> {
    let parsed = match ty {
        FieldType::I64 => {
            let n = if let Some(s) = value.as_str() {
                s.parse::<i64>().ok()
            } else {
                value
                    .as_i64()
                    .filter(|n| n.unsigned_abs() <= 9_007_199_254_740_991)
            };
            n.map(ReflectValue::I64)
        }
        FieldType::Enum(names) => value
            .as_str()
            .and_then(|name| names.iter().position(|n| *n == name))
            .map(|n| ReflectValue::I64(n as i64))
            .or_else(|| {
                value
                    .as_i64()
                    .filter(|n| *n >= 0 && (*n as usize) < names.len())
                    .map(ReflectValue::I64)
            }),
        FieldType::Array(inner) => match value.as_array() {
            Some(items) => Some(ReflectValue::Array(
                items
                    .iter()
                    .map(|v| decode(world, inner, v))
                    .collect::<Result<_, _>>()?,
            )),
            None => None,
        },
        _ => crate::scene_schema::value_from_json(
            &|id: PersistentId| world.entity_by_persistent_id(id),
            ty,
            value,
        ),
    }
    .ok_or_else(|| format!("expected {}", ty.name()))?;
    if !parsed.is_finite() {
        return Err("non-finite numeric value".into());
    }
    if let ReflectValue::Quat(q) = &parsed {
        let length = q.iter().map(|n| n * n).sum::<f32>();
        if (length - 1.0).abs() > 0.001 {
            return Err("quaternion must be normalized [x,y,z,w]".into());
        }
    }
    if let ReflectValue::Entity(None) = parsed {
        if !value.is_null() {
            return Err("entity reference is stale or unloaded".into());
        }
    }
    Ok(parsed)
}
pub fn type_schema(ty: &FieldType) -> Value {
    match ty {
        FieldType::Bool => json!({"type":"boolean"}),
        FieldType::I64 => {
            json!({"type":"string","pattern":"^-?[0-9]+$","description":"Lossless signed 64-bit integer"})
        }
        FieldType::F64 => json!({"type":"number"}),
        FieldType::Str => json!({"type":"string"}),
        FieldType::Enum(names) => json!({"type":"string","enum":names}),
        FieldType::Vec2
        | FieldType::Vec3
        | FieldType::Vec4
        | FieldType::Color
        | FieldType::Quat => {
            let n = match ty {
                FieldType::Vec2 => 2,
                FieldType::Vec3 | FieldType::Color => 3,
                _ => 4,
            };
            json!({"type":"array","items":{"type":"number"},"minItems":n,"maxItems":n,"description":ty.name()})
        }
        FieldType::Array(inner) => json!({"type":"array","items":type_schema(inner)}),
        FieldType::Entity => json!({"description":"null or {$entity: persistent hexadecimal ID}"}),
        FieldType::Asset => json!({"description":"null or {$asset: durable hexadecimal ID}"}),
        FieldType::Curve => {
            json!({"description":"{$curve:[time,value,in_tangent,out_tangent,interpolation,...]}"})
        }
        FieldType::Gradient => json!({"description":"{$gradient:[time,r,g,b,a,...]}"}),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lossless_integer_and_nested_array_roundtrip() {
        let world = World::new();
        for n in [i64::MIN, 0, i64::MAX] {
            let v = ReflectValue::I64(n);
            assert_eq!(
                decode(
                    &world,
                    &FieldType::I64,
                    &encode(&world, &FieldType::I64, &v)
                )
                .unwrap(),
                v
            );
        }
        assert!(decode(&world, &FieldType::I64, &json!(i64::MAX)).is_err());
        let ty = FieldType::Array(Box::new(FieldType::I64));
        let v = ReflectValue::Array(vec![ReflectValue::I64(i64::MAX)]);
        assert_eq!(decode(&world, &ty, &encode(&world, &ty, &v)).unwrap(), v);
    }
    #[test]
    fn invalid_rotation_and_stale_reference_are_errors() {
        let world = World::new();
        assert!(decode(&world, &FieldType::Quat, &json!([0, 0, 0, 0])).is_err());
        assert!(
            decode(
                &world,
                &FieldType::Entity,
                &json!({"$entity":"00000000000000000000000000000042"})
            )
            .is_err()
        );
    }
}
