//! What the ECS stores about scripts — and, just as importantly, what it
//! does not.
//!
//! The world holds **authored data only**: which asset, which attachment,
//! whether it is on, what order it runs in, and the author's exported
//! property values. Everything the VM knows lives in the runtime, keyed
//! by [`InstanceUuid`].
//!
//! Nothing here is a VM pointer, a closure, a coroutine or a table. That
//! is what makes a scene file portable, a hot reload survivable, and the
//! language replaceable.

use std::collections::BTreeMap;

use somnium_ecs::Component;

use crate::ids::{InstanceUuid, ScriptAssetId};
use crate::value::ScriptValue;

/// Author-declared property values for one attachment, keyed by the
/// property's declared name.
///
/// Keyed by **name**, not by index: a script author reordering their
/// `fields` table must not silently reinterpret saved values. (Engine
/// component fields go the other way and key by id at runtime, because
/// their declaration order is fixed by the Rust struct.)
pub type PropertyBag = BTreeMap<String, ScriptValue>;

/// The API generation this build of the engine speaks.
///
/// Bumped when the shape of the script-facing API changes in a way old
/// scripts cannot be run against. An attachment records the version it
/// was authored for so a mismatch is a diagnostic rather than a
/// mysterious runtime failure.
pub const CURRENT_API_VERSION: u32 = 1;

/// One script attached to one entity.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptAttachment {
    /// Durable identity of this attachment. Survives save, load and
    /// reload; it is what migrated state is keyed by.
    pub instance: InstanceUuid,
    /// The script asset this runs.
    pub asset: ScriptAssetId,
    /// Whether the attachment receives lifecycle calls.
    pub enabled: bool,
    /// Sort key for execution order. Lower runs first; ties break on the
    /// entity's persistent id, then on `instance`.
    pub execution_order: i32,
    /// Author-set values for the script's declared properties.
    pub properties: PropertyBag,
    /// Version of the script's own declared property schema, for
    /// migration when the author changes their field list.
    pub schema_version: u32,
    /// Engine API version this attachment was authored against.
    pub api_version: u32,
}

impl ScriptAttachment {
    /// A new attachment of `asset`, enabled, with no property overrides.
    #[must_use]
    pub fn new(asset: ScriptAssetId) -> Self {
        Self {
            instance: InstanceUuid::mint(),
            asset,
            enabled: true,
            execution_order: 0,
            properties: PropertyBag::new(),
            schema_version: 1,
            api_version: CURRENT_API_VERSION,
        }
    }
}

/// Every script attached to an entity.
///
/// A component rather than one-attachment-per-entity because an entity
/// commonly carries several behaviours, and because ordering between them
/// has to be authored somewhere.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptSet {
    /// Attachments in authored order. Execution order is computed from
    /// [`ScriptAttachment::execution_order`], not from this vec's order.
    pub attachments: Vec<ScriptAttachment>,
}

impl Component for ScriptSet {}

impl ScriptSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an attachment, returning its durable id.
    pub fn attach(&mut self, attachment: ScriptAttachment) -> InstanceUuid {
        let id = attachment.instance;
        self.attachments.push(attachment);
        id
    }

    /// Remove an attachment by its durable id.
    pub fn detach(&mut self, instance: InstanceUuid) -> Option<ScriptAttachment> {
        let index = self
            .attachments
            .iter()
            .position(|a| a.instance == instance)?;
        Some(self.attachments.remove(index))
    }

    /// Find an attachment by its durable id.
    #[must_use]
    pub fn get(&self, instance: InstanceUuid) -> Option<&ScriptAttachment> {
        self.attachments.iter().find(|a| a.instance == instance)
    }

    /// Find an attachment by its durable id, mutably.
    pub fn get_mut(&mut self, instance: InstanceUuid) -> Option<&mut ScriptAttachment> {
        self.attachments.iter_mut().find(|a| a.instance == instance)
    }

    /// Whether the set has no attachments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.attachments.is_empty()
    }

    /// Number of attachments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.attachments.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_and_detach_round_trip() {
        let asset = ScriptAssetId::mint();
        let mut set = ScriptSet::new();
        assert!(set.is_empty());

        let id = set.attach(ScriptAttachment::new(asset));
        assert_eq!(set.len(), 1);
        assert_eq!(set.get(id).unwrap().asset, asset);

        set.get_mut(id).unwrap().enabled = false;
        assert!(!set.get(id).unwrap().enabled);

        let removed = set.detach(id).unwrap();
        assert_eq!(removed.instance, id);
        assert!(set.is_empty());
        assert!(set.detach(id).is_none());
    }

    #[test]
    fn each_attachment_gets_its_own_durable_id() {
        let asset = ScriptAssetId::mint();
        let mut set = ScriptSet::new();
        let a = set.attach(ScriptAttachment::new(asset));
        let b = set.attach(ScriptAttachment::new(asset));
        assert_ne!(a, b, "two attachments of the same asset are still distinct");
    }

    #[test]
    fn a_new_attachment_records_the_current_api_version() {
        let attachment = ScriptAttachment::new(ScriptAssetId::mint());
        assert_eq!(attachment.api_version, CURRENT_API_VERSION);
        assert!(attachment.enabled);
        assert_eq!(attachment.execution_order, 0);
        assert!(attachment.properties.is_empty());
    }
}
