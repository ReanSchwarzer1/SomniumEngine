//! The script-facing value model.
//!
//! This is deliberately the **same** type the component registry uses.
//! A second value model would mean a second set of conversions, a second
//! set of range checks, and eventually two answers to "what is a Vec3".
//! [`ScriptValue`] is therefore an alias, not a parallel enum.
//!
//! See [`somnium_ecs::reflect`] for the variants and the finiteness rule.

pub use somnium_ecs::reflect::{AssetRef, FieldId, FieldType, ReflectObject, ReflectValue};

/// A bounded, engine-neutral value as seen by script code.
///
/// Every value crossing the boundary — property, snapshot field, command
/// argument, event payload, migrated state — is one of these.
pub type ScriptValue = ReflectValue;

/// An ordered record of values.
pub type ScriptObject = ReflectObject;

/// Convenience constructors, so callers do not spell out the enum for the
/// common cases.
pub trait IntoScriptValue {
    /// Widen into a script value.
    fn into_script_value(self) -> ScriptValue;
}

impl IntoScriptValue for bool {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::Bool(self)
    }
}

impl IntoScriptValue for i64 {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::I64(self)
    }
}

impl IntoScriptValue for f64 {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::F64(self)
    }
}

impl IntoScriptValue for f32 {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::F64(f64::from(self))
    }
}

impl IntoScriptValue for &str {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::Str(self.to_owned())
    }
}

impl IntoScriptValue for String {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::Str(self)
    }
}

impl IntoScriptValue for [f32; 3] {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::Vec3(self)
    }
}

impl IntoScriptValue for [f32; 4] {
    fn into_script_value(self) -> ScriptValue {
        ScriptValue::Vec4(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convenience_constructors_produce_the_expected_variants() {
        assert_eq!(true.into_script_value(), ScriptValue::Bool(true));
        assert_eq!(3_i64.into_script_value(), ScriptValue::I64(3));
        assert_eq!(1.5_f32.into_script_value(), ScriptValue::F64(1.5));
        assert_eq!("hi".into_script_value(), ScriptValue::Str("hi".into()));
        assert_eq!(
            [1.0_f32, 2.0, 3.0].into_script_value(),
            ScriptValue::Vec3([1.0, 2.0, 3.0])
        );
    }

    #[test]
    fn non_finite_values_are_detectable_before_they_reach_the_world() {
        assert!(ScriptValue::F64(1.0).is_finite());
        assert!(!ScriptValue::F64(f64::NAN).is_finite());
        assert!(!ScriptValue::Vec3([1.0, f32::INFINITY, 3.0]).is_finite());
        assert!(
            !ScriptValue::Array(vec![ScriptValue::F64(f64::NAN)]).is_finite(),
            "nesting must not hide a NaN"
        );
    }
}
