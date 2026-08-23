//! Registry mapping every reflected field shape to an inspector editor.

use somnium_ecs::reflect::{AssetRef, FieldType, ReflectValue};

/// Candidate supplied by CONTROL-C's asset database to the generic property editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCandidate {
    pub id: AssetRef,
    pub label: String,
    pub kind_bit: u64,
}

/// Asset-layer hook used by the otherwise engine-neutral Asset editor.
/// Implementations may query asynchronously; committing always returns the
/// same reflected value vocabulary used by serialization and undo.
pub trait AssetEditorContext: Send + Sync {
    fn query(&self, text: &str, kind_mask: u64) -> Vec<AssetCandidate>;

    fn commit(&self, id: Option<AssetRef>, kind_mask: u64) -> Result<ReflectValue, String> {
        if let Some(id) = id {
            let valid = self
                .query("", kind_mask)
                .iter()
                .any(|candidate| candidate.id == id && candidate.kind_bit & kind_mask != 0);
            if !valid {
                return Err("asset does not match the field's kind constraint".into());
            }
        }
        Ok(ReflectValue::Asset(id))
    }
}

impl AssetEditorContext for somnium_asset::database::AssetDbSnapshot {
    fn query(&self, text: &str, kind_mask: u64) -> Vec<AssetCandidate> {
        self.search(text, kind_mask)
            .into_iter()
            .map(|record| AssetCandidate {
                id: AssetRef::from_raw(record.id.raw()),
                label: record.relative_path,
                kind_bit: record.kind.bit(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyEditorKind {
    CheckBox,
    Integer,
    Number,
    Text,
    Vec2,
    Vec3,
    Vec4,
    Euler,
    ColorSwatch,
    EntityPicker,
    AssetPicker,
    ComboBox,
    Collection,
    Unsupported,
}

pub trait PropertyEditor: Send + Sync {
    fn accepts(&self, ty: &FieldType) -> bool;
    fn kind(&self) -> PropertyEditorKind;
}

struct StandardEditor {
    accepts: fn(&FieldType) -> bool,
    kind: PropertyEditorKind,
}

impl PropertyEditor for StandardEditor {
    fn accepts(&self, ty: &FieldType) -> bool {
        (self.accepts)(ty)
    }
    fn kind(&self) -> PropertyEditorKind {
        self.kind
    }
}

#[derive(Default)]
pub struct PropertyEditorRegistry {
    editors: Vec<Box<dyn PropertyEditor>>,
}

impl PropertyEditorRegistry {
    pub fn standard() -> Self {
        use FieldType::*;
        let mut registry = Self::default();
        macro_rules! add {
            ($pattern:pat, $kind:ident) => {
                registry.editors.push(Box::new(StandardEditor {
                    accepts: |ty| matches!(ty, $pattern),
                    kind: PropertyEditorKind::$kind,
                }));
            };
        }
        add!(Bool, CheckBox);
        add!(I64, Integer);
        add!(F64, Number);
        add!(Str, Text);
        add!(Vec2, Vec2);
        add!(Vec3, Vec3);
        add!(Vec4, Vec4);
        add!(Quat, Euler);
        add!(Color, ColorSwatch);
        add!(Entity, EntityPicker);
        add!(Asset, AssetPicker);
        add!(Enum(_), ComboBox);
        add!(Array(_), Collection);
        // Visible fallback is deliberately last: a newly added FieldType can
        // never disappear from Details merely because no custom editor exists.
        registry.editors.push(Box::new(StandardEditor {
            accepts: |_| true,
            kind: PropertyEditorKind::Unsupported,
        }));
        registry
    }

    pub fn add_first(&mut self, editor: Box<dyn PropertyEditor>) {
        self.editors.insert(0, editor);
    }

    pub fn for_type(&self, ty: &FieldType) -> &dyn PropertyEditor {
        self.editors
            .iter()
            .find(|editor| editor.accepts(ty))
            .expect("fallback editor")
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_type_has_a_visible_editor() {
        let registry = PropertyEditorRegistry::standard();
        let variants = [
            FieldType::Bool,
            FieldType::I64,
            FieldType::F64,
            FieldType::Str,
            FieldType::Vec2,
            FieldType::Vec3,
            FieldType::Vec4,
            FieldType::Quat,
            FieldType::Color,
            FieldType::Entity,
            FieldType::Asset,
            FieldType::Enum(&["A"]),
            FieldType::Array(Box::new(FieldType::Color)),
        ];
        for ty in variants {
            assert_ne!(
                registry.for_type(&ty).kind(),
                PropertyEditorKind::Unsupported
            );
        }
    }

    #[test]
    fn array_editor_recurses_to_the_inner_editor() {
        let registry = PropertyEditorRegistry::standard();
        let FieldType::Array(inner) = FieldType::Array(Box::new(FieldType::Color)) else {
            unreachable!()
        };
        assert_eq!(
            registry.for_type(&inner).kind(),
            PropertyEditorKind::ColorSwatch
        );
    }

    #[test]
    fn asset_commit_rejects_the_wrong_kind() {
        struct Catalog;
        impl AssetEditorContext for Catalog {
            fn query(&self, _: &str, _: u64) -> Vec<AssetCandidate> {
                vec![AssetCandidate {
                    id: AssetRef::from_raw(7),
                    label: "rock.png".into(),
                    kind_bit: 2,
                }]
            }
        }
        assert_eq!(Catalog.commit(None, 2).unwrap(), ReflectValue::Asset(None));
        assert!(Catalog.commit(Some(AssetRef::from_raw(7)), 4).is_err());
        assert_eq!(
            Catalog.commit(Some(AssetRef::from_raw(7)), 2).unwrap(),
            ReflectValue::Asset(Some(AssetRef::from_raw(7)))
        );
    }
}
