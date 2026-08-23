//! Moving values across the boundary.
//!
//! # Handles are opaque, and that is not stylistic
//!
//! Luau numbers are doubles. Integers are exact only to 2^53, so a 64- or
//! 128-bit id that made a round trip through a script's number type could
//! come back *changed* — silently, and only for large values, which is the
//! worst possible failure shape. Entity and asset references therefore
//! cross as userdata that Luau can hold, compare and pass back but cannot
//! do arithmetic on or forge.
//!
//! # Vec3 uses Luau's native vector
//!
//! Luau has a built-in three-float vector with its own VM opcodes. Using
//! it rather than a table means a script doing transform maths is not
//! allocating a table per operation, which is most of what gameplay code
//! does.

use mlua::{FromLua, IntoLua, Lua, UserData, UserDataMethods, Value, Vector};
use somnium_ecs::Entity;
use somnium_ecs::reflect::AssetRef;
use somnium_script::value::ScriptValue;

/// An entity reference as a script sees it.
///
/// Opaque: no arithmetic, no field access to the index or generation, and
/// no way to construct one from a number. A script can only obtain one
/// from the engine, which means it cannot fabricate a handle to an entity
/// it was never given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityHandle(pub Entity);

// `mlua` has a blanket `IntoLua` for userdata but no blanket `FromLua`,
// so a handle coming back from a script has to be unwrapped explicitly.
// Doing it here rather than at every call site is what lets host functions
// take `EntityHandle` directly and get a typed error — "expected Entity,
// got number" — instead of a borrow failure deep in the argument decoder.
macro_rules! impl_handle_from_lua {
    ($t:ty, $name:literal) => {
        impl FromLua for $t {
            fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
                match value {
                    Value::UserData(ud) => ud.borrow::<Self>().map(|handle| *handle),
                    other => Err(mlua::Error::FromLuaConversionError {
                        from: other.type_name(),
                        to: $name.to_string(),
                        message: Some(
                            concat!(
                                "expected an engine ",
                                $name,
                                " handle; these cannot be constructed from a number"
                            )
                            .to_string(),
                        ),
                    }),
                }
            }
        }
    };
}
impl_handle_from_lua!(EntityHandle, "Entity");
impl_handle_from_lua!(AssetHandle, "Asset");

impl UserData for EntityHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Equality, so `if e == ctx.entity then` works.
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: Self| {
            Ok(this.0 == other.0)
        });
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("Entity({})", this.0))
        });
    }
}

/// An asset reference as a script sees it. Opaque for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetHandle(pub AssetRef);

impl UserData for AssetHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: Self| {
            Ok(this.0 == other.0)
        });
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("Asset({:032x})", this.0.0))
        });
    }
}

/// Widen an engine value into Luau.
///
/// # Errors
///
/// Whatever the VM raises building the value — in practice only an
/// allocation failure once the memory ceiling is reached.
pub fn to_lua(lua: &Lua, value: &ScriptValue) -> mlua::Result<Value> {
    Ok(match value {
        ScriptValue::Bool(v) => Value::Boolean(*v),
        ScriptValue::I64(v) => Value::Integer(*v),
        ScriptValue::F64(v) => Value::Number(*v),
        ScriptValue::Str(v) => Value::String(lua.create_string(v)?),
        ScriptValue::Vec2(v) => {
            let table = lua.create_table()?;
            table.set("x", v[0])?;
            table.set("y", v[1])?;
            Value::Table(table)
        }
        ScriptValue::Vec3(v) => Value::Vector(Vector::new(v[0], v[1], v[2])),
        ScriptValue::Vec4(v) | ScriptValue::Quat(v) => {
            let table = lua.create_table()?;
            table.set("x", v[0])?;
            table.set("y", v[1])?;
            table.set("z", v[2])?;
            table.set("w", v[3])?;
            Value::Table(table)
        }
        ScriptValue::Entity(Some(e)) => EntityHandle(*e).into_lua(lua)?,
        ScriptValue::Asset(Some(a)) => AssetHandle(*a).into_lua(lua)?,
        // An unset reference is `nil`, the same as an absent value: a
        // script tests both with `if x then`.
        ScriptValue::Nil | ScriptValue::Entity(None) | ScriptValue::Asset(None) => Value::Nil,
        ScriptValue::Array(items) => {
            let table = lua.create_table_with_capacity(items.len(), 0)?;
            for (i, item) in items.iter().enumerate() {
                table.set(i + 1, to_lua(lua, item)?)?;
            }
            Value::Table(table)
        }
        // CONTROL-K. A curve and a gradient widen into ordinary Luau arrays
        // of key tables, which is everything a script needs in order to read
        // one. They deliberately do **not** narrow back: `from_lua` produces
        // an `Array` by shape, and `FieldType::Curve::accepts` rejects it with
        // a named type mismatch rather than silently reinterpreting a list of
        // tables as authored keyframes. Authoring a curve is the editor's job.
        ScriptValue::Curve(curve) => {
            let table = lua.create_table_with_capacity(curve.len(), 0)?;
            for (i, key) in curve.keys().iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("t", key.t)?;
                entry.set("v", key.v)?;
                entry.set("inTangent", key.in_tangent)?;
                entry.set("outTangent", key.out_tangent)?;
                entry.set("interpolation", key.interpolation.as_str())?;
                table.set(i + 1, entry)?;
            }
            Value::Table(table)
        }
        ScriptValue::Gradient(gradient) => {
            let table = lua.create_table_with_capacity(gradient.len(), 0)?;
            for (i, stop) in gradient.stops().iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("t", stop.t)?;
                entry.set(
                    "color",
                    Value::Vector(Vector::new(stop.color[0], stop.color[1], stop.color[2])),
                )?;
                entry.set("alpha", stop.color[3])?;
                table.set(i + 1, entry)?;
            }
            Value::Table(table)
        }
        ScriptValue::Object(fields) => {
            let table = lua.create_table()?;
            for (id, value) in fields {
                table.set(id.0, to_lua(lua, value)?)?;
            }
            Value::Table(table)
        }
        ScriptValue::Map(entries) => {
            let table = lua.create_table()?;
            for (key, value) in entries {
                table.set(key.as_str(), to_lua(lua, value)?)?;
            }
            Value::Table(table)
        }
    })
}

/// Narrow a Luau value into an engine value.
///
/// This is a *shape* conversion only. It does not know what the receiving
/// field's declared type is, so `{x=1, y=2, z=3, w=4}` becomes a `Vec4`
/// and never a `Quat`; the schema check that follows is what decides
/// whether that is acceptable. Doing it in this order means one
/// conversion serves every destination and only one place enforces types.
///
/// # Errors
///
/// A message naming what could not be converted. Functions, threads,
/// unknown userdata and cyclic tables are all refused — none of them is
/// durable data, and letting one through is how a VM reference ends up
/// in a save file.
pub fn from_lua(value: &Value) -> Result<ScriptValue, String> {
    from_lua_depth(value, 0)
}

/// Nesting cap. A cyclic table would otherwise recurse until the stack
/// runs out, and a script can build one in two lines.
const MAX_DEPTH: u32 = 16;

fn from_lua_depth(value: &Value, depth: u32) -> Result<ScriptValue, String> {
    if depth > MAX_DEPTH {
        return Err(format!("value nests deeper than {MAX_DEPTH} levels"));
    }
    Ok(match value {
        Value::Nil => ScriptValue::Nil,
        Value::Boolean(v) => ScriptValue::Bool(*v),
        Value::Integer(v) => ScriptValue::I64(*v),
        Value::Number(v) => ScriptValue::F64(*v),
        Value::String(v) => ScriptValue::Str(
            v.to_str()
                .map_err(|_| "string is not valid UTF-8".to_string())?
                .to_string(),
        ),
        Value::Vector(v) => ScriptValue::Vec3([v.x(), v.y(), v.z()]),
        Value::UserData(ud) => {
            if let Ok(handle) = ud.borrow::<EntityHandle>() {
                ScriptValue::Entity(Some(handle.0))
            } else if let Ok(handle) = ud.borrow::<AssetHandle>() {
                ScriptValue::Asset(Some(handle.0))
            } else {
                return Err("userdata of an unknown type cannot cross the boundary".into());
            }
        }
        Value::Table(table) => return table_from_lua(table, depth),
        Value::Function(_) => {
            return Err("a function is not data and cannot be stored or sent".into());
        }
        Value::Thread(_) => {
            return Err("a coroutine is not data and cannot be stored or sent".into());
        }
        other => return Err(format!("`{}` cannot cross the boundary", other.type_name())),
    })
}

/// Tables become a `Vec2`/`Vec4` if they are shaped like one, an array if
/// they have a sequence part, or a string-keyed map otherwise.
///
/// Note what is deliberately missing: there is no path from a table to a
/// [`ScriptValue::Object`]. Records keyed by `FieldId` are the engine's
/// representation and the script does not know those ids; a script naming
/// component fields uses strings, and the caller resolves them against the
/// schema.
fn table_from_lua(table: &mlua::Table, depth: u32) -> Result<ScriptValue, String> {
    let has = |key: &str| table.contains_key(key).unwrap_or(false);
    if has("x") && has("y") {
        let get = |key: &str| -> Result<f32, String> {
            table
                .get::<f32>(key)
                .map_err(|_| format!("component `{key}` is not a number"))
        };
        if has("w") {
            return Ok(ScriptValue::Vec4([
                get("x")?,
                get("y")?,
                get("z")?,
                get("w")?,
            ]));
        }
        if has("z") {
            return Ok(ScriptValue::Vec3([get("x")?, get("y")?, get("z")?]));
        }
        return Ok(ScriptValue::Vec2([get("x")?, get("y")?]));
    }

    let len = table.raw_len();
    if len > 0 {
        let mut items = Vec::with_capacity(len);
        for i in 1..=len {
            let item: Value = table
                .get(i)
                .map_err(|e| format!("array element {i}: {e}"))?;
            items.push(from_lua_depth(&item, depth + 1)?);
        }
        return Ok(ScriptValue::Array(items));
    }

    // No sequence part: treat it as a record. Only string keys survive —
    // a table keyed by a function or another table has no durable
    // representation and saying so is better than dropping it quietly.
    let mut entries = std::collections::BTreeMap::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair.map_err(|e| format!("record entry: {e}"))?;
        let Value::String(key) = key else {
            return Err(format!(
                "a record key must be a string, found {}",
                key.type_name()
            ));
        };
        let key = key
            .to_str()
            .map_err(|_| "record key is not valid UTF-8".to_string())?
            .to_string();
        entries.insert(key, from_lua_depth(&value, depth + 1)?);
    }
    Ok(ScriptValue::Map(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_script::value::ScriptValue;

    fn lua() -> Lua {
        crate::new_sandboxed_state(64 * 1024 * 1024).unwrap()
    }

    fn round_trip(value: &ScriptValue) -> ScriptValue {
        let lua = lua();
        let as_lua = to_lua(&lua, value).unwrap();
        from_lua(&as_lua).unwrap()
    }

    #[test]
    fn scalars_round_trip() {
        for value in [
            ScriptValue::Nil,
            ScriptValue::Bool(true),
            ScriptValue::I64(-42),
            ScriptValue::F64(1.5),
            ScriptValue::Str("hello".into()),
        ] {
            assert_eq!(round_trip(&value), value);
        }
    }

    #[test]
    fn vec3_uses_the_native_vector_type() {
        let lua = lua();
        let value = to_lua(&lua, &ScriptValue::Vec3([1.0, 2.0, 3.0])).unwrap();
        assert!(
            matches!(value, Value::Vector(_)),
            "Vec3 must be a Luau vector, not a table"
        );
        assert_eq!(
            from_lua(&value).unwrap(),
            ScriptValue::Vec3([1.0, 2.0, 3.0])
        );
    }

    #[test]
    fn vec2_and_vec4_round_trip_through_tables() {
        assert_eq!(
            round_trip(&ScriptValue::Vec2([1.0, 2.0])),
            ScriptValue::Vec2([1.0, 2.0])
        );
        assert_eq!(
            round_trip(&ScriptValue::Vec4([1.0, 2.0, 3.0, 4.0])),
            ScriptValue::Vec4([1.0, 2.0, 3.0, 4.0])
        );
    }

    #[test]
    fn a_quat_arrives_as_a_vec4_and_the_schema_decides() {
        // Documented above: shape conversion cannot distinguish them, and
        // the field's declared type is what resolves it.
        assert_eq!(
            round_trip(&ScriptValue::Quat([0.0, 0.0, 0.0, 1.0])),
            ScriptValue::Vec4([0.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn entity_handles_are_opaque_and_compare_by_value() {
        let lua = lua();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((somnium_ecs::PersistentId::mint(),));

        let value = to_lua(&lua, &ScriptValue::Entity(Some(entity))).unwrap();
        assert!(matches!(value, Value::UserData(_)), "must be userdata");
        assert_eq!(from_lua(&value).unwrap(), ScriptValue::Entity(Some(entity)));

        // A script cannot do arithmetic on it, and cannot read an index out.
        lua.globals().set("e", value).unwrap();
        assert!(
            lua.load("return e + 1").eval::<mlua::Value>().is_err(),
            "an entity handle must not be a number in disguise"
        );
        assert!(lua.load("return e == e").eval::<bool>().unwrap());
    }

    #[test]
    fn an_unset_reference_is_nil_both_ways() {
        let lua = lua();
        assert!(matches!(
            to_lua(&lua, &ScriptValue::Entity(None)).unwrap(),
            Value::Nil
        ));
        assert_eq!(from_lua(&Value::Nil).unwrap(), ScriptValue::Nil);
    }

    #[test]
    fn arrays_round_trip() {
        let value = ScriptValue::Array(vec![
            ScriptValue::I64(1),
            ScriptValue::I64(2),
            ScriptValue::I64(3),
        ]);
        assert_eq!(round_trip(&value), value);
    }

    #[test]
    fn functions_and_coroutines_are_refused() {
        let lua = lua();
        let function: Value = lua.load("return function() end").eval().unwrap();
        assert!(
            from_lua(&function).unwrap_err().contains("function"),
            "a closure must never become durable data"
        );

        let thread: Value = lua
            .load("return coroutine.create(function() end)")
            .eval()
            .unwrap();
        assert!(from_lua(&thread).unwrap_err().contains("coroutine"));
    }

    #[test]
    fn a_cyclic_table_is_refused_rather_than_overflowing_the_stack() {
        let lua = lua();
        let cyclic: Value = lua.load("local t = {} t[1] = {t} return t").eval().unwrap();
        let err = from_lua(&cyclic).unwrap_err();
        assert!(err.contains("nests deeper"), "got: {err}");
    }
}
