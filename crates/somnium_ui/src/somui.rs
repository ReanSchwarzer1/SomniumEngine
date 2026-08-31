//! `.somui` documents and the widget-kind registry (MORROWIND-M2, step 1).
//!
//! Track 1 gave a game a UI framework and left authoring to code. M2's claim is
//! that Somnium is unusually well placed to close that gap because **the editor
//! is the framework** — same retained tree, same paint layer, same measure and
//! arrange — so authoring game UI is mostly plumbing rather than a second
//! system. This module is that plumbing's first half: a widget tree that can be
//! written to disk, read back, checked, and turned into real widgets.
//!
//! # The registry comes first, and that is the point
//!
//! > *"The widget palette generated from the registered widget types — not a
//! > second hand-written list, per CONTROL-A2's command registry precedent."*
//!
//! There was no such registry. Thirty-one builders each knew how to construct
//! themselves and nothing knew they existed, so a palette would have been a
//! hand-written list that silently rots the first time a widget is added or
//! renamed. [`KINDS`] is that list once: the palette reads it, the loader reads
//! it, and a document naming something not in it fails with the name rather than
//! dropping the element and rendering a hole.
//!
//! # What a document is not
//!
//! It is not a scene. Anchors, offsets and a pivot per element, properties as
//! values, children in order — and nothing about entities, transforms or the
//! world. A `.somui` is loaded *into* a canvas, and the canvas
//! ([`crate::runtime::canvas`], MORROWIND-E) is what knows about screens and
//! world space.
//!
//! # Versioning
//!
//! [`UiDocument::version`] is written on save and checked on load. A document
//! from the future is refused — reading it as if it were current is how a
//! field that changed meaning corrupts a file people edited. A document from
//! the past is migrated, and there is nothing to migrate yet, which is exactly
//! when the mechanism has to exist.

use crate::message::NodeHandle;
use crate::runtime::{Anchoring, Anchors, Offsets, UiCanvas};
use crate::types::Rect;
use crate::widget::WidgetBuilder;
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The document format this build writes.
pub const CURRENT_VERSION: u32 = 1;

/// A property value on an element.
///
/// Deliberately small. CONTROL-B's schema seam is what will eventually type
/// these per widget kind; until it does, a document that carries a colour where
/// a number belongs is caught by the widget that reads it, not here. Adding a
/// variant is a format change and takes a version bump.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
    /// Linear RGBA, matching `crate::color`.
    Color([f32; 4]),
    Vec2([f32; 2]),
}

impl Value {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

/// One element of a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiElement {
    /// A [`KINDS`] id. Not a Rust type name: a type can be renamed without
    /// invalidating every document that mentions it.
    pub kind: String,
    /// The author's name for this element, unique within the document.
    ///
    /// This is what script and Rust look an element up by, so a duplicate is a
    /// validation error rather than a shrug — two elements answering to one
    /// name means `find("HealthBar")` returns whichever the walk reached first.
    pub name: String,
    /// Anchor fractions of the parent: `(0,0)` top-left, `(1,1)` bottom-right.
    pub anchor_min: [f32; 2],
    pub anchor_max: [f32; 2],
    /// Insets from the anchored edges, or position and size when pinned.
    pub offsets: [f32; 4],
    /// The point of the element its offsets position, in its own fractions.
    pub pivot: [f32; 2],
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<UiElement>,
}

impl UiElement {
    /// An element of `kind`, filling its parent.
    #[must_use]
    pub fn new(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            anchor_min: [0.0, 0.0],
            anchor_max: [1.0, 1.0],
            offsets: [0.0, 0.0, 0.0, 0.0],
            pivot: [0.0, 0.0],
            properties: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    /// Pin this element at a position and size against its anchors.
    #[must_use]
    pub fn pinned(mut self, anchor: [f32; 2], position: Vec2, size: Vec2) -> Self {
        self.anchor_min = anchor;
        self.anchor_max = anchor;
        self.offsets = [position.x, position.y, size.x, size.y];
        self
    }

    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: Value) -> Self {
        self.properties.insert(key.into(), value);
        self
    }

    #[must_use]
    pub fn child(mut self, child: UiElement) -> Self {
        self.children.push(child);
        self
    }

    /// The runtime placement this element describes.
    ///
    /// The bridge to MORROWIND-E: a document says nothing about layout that the
    /// runtime anchor model cannot already express, which is what keeps a
    /// `.somui` from becoming a second placement system.
    #[must_use]
    pub fn anchoring(&self) -> Anchoring {
        Anchoring {
            anchors: Anchors {
                min: Vec2::new(self.anchor_min[0], self.anchor_min[1]),
                max: Vec2::new(self.anchor_max[0], self.anchor_max[1]),
            },
            offsets: Offsets {
                left: self.offsets[0],
                top: self.offsets[1],
                right: self.offsets[2],
                bottom: self.offsets[3],
            },
            pivot: Vec2::new(self.pivot[0], self.pivot[1]),
        }
    }

    fn walk<'a>(&'a self, out: &mut Vec<&'a UiElement>) {
        out.push(self);
        for child in &self.children {
            child.walk(out);
        }
    }
}

/// A `.somui` document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiDocument {
    pub version: u32,
    /// The resolution this document was authored against.
    ///
    /// Not a constraint — anchors are fractions and scale on their own. It is
    /// what the canvas-mode preview opens at, so a document reopens looking the
    /// way its author left it.
    pub reference: [f32; 2],
    pub root: UiElement,
}

/// Why a document could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentError {
    /// The file is not a `.somui` JSON document at all.
    Parse(String),
    /// Written by a newer build.
    FromTheFuture { found: u32, current: u32 },
    /// An element names a kind this build does not have.
    UnknownKind { element: String, kind: String },
    /// Two elements answer to one name.
    DuplicateName(String),
    /// An element has no name, so nothing can address it.
    UnnamedElement { kind: String },
    /// The preview resolution cannot describe a canvas.
    InvalidReference,
    /// An element's anchors, offsets, or pivot contain an impossible value.
    InvalidPlacement { element: String },
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(reason) => write!(f, "invalid .somui document: {reason}"),
            Self::FromTheFuture { found, current } => write!(
                f,
                "document version {found} is newer than this build's {current}"
            ),
            Self::UnknownKind { element, kind } => {
                write!(f, "element `{element}` has unknown widget kind `{kind}`")
            }
            Self::DuplicateName(name) => write!(f, "two elements are named `{name}`"),
            Self::UnnamedElement { kind } => write!(f, "an element of kind `{kind}` has no name"),
            Self::InvalidReference => write!(f, "reference resolution must be finite and positive"),
            Self::InvalidPlacement { element } => {
                write!(
                    f,
                    "element `{element}` has invalid anchors, offsets, or pivot"
                )
            }
        }
    }
}

impl UiDocument {
    /// An empty document with one full-bleed root panel.
    #[must_use]
    pub fn new(reference: Vec2) -> Self {
        Self {
            version: CURRENT_VERSION,
            reference: [reference.x, reference.y],
            root: UiElement::new(kinds::PANEL, "Root"),
        }
    }

    /// Every element, root first, depth-first.
    #[must_use]
    pub fn elements(&self) -> Vec<&UiElement> {
        let mut out = Vec::new();
        self.root.walk(&mut out);
        out
    }

    /// The element with this name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&UiElement> {
        self.elements().into_iter().find(|e| e.name == name)
    }

    /// Check a document before anything acts on it.
    ///
    /// Returns **every** problem rather than the first. A document with four
    /// unknown kinds should say so once, not four times across four loads —
    /// that is the difference between a message somebody can fix from and a
    /// game of whack-a-mole.
    pub fn validate(&self) -> Result<(), Vec<DocumentError>> {
        let mut errors = Vec::new();
        if self.version > CURRENT_VERSION {
            errors.push(DocumentError::FromTheFuture {
                found: self.version,
                current: CURRENT_VERSION,
            });
        }
        if self.reference.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            errors.push(DocumentError::InvalidReference);
        }
        let mut seen: Vec<&str> = Vec::new();
        for element in self.elements() {
            if element.name.is_empty() {
                errors.push(DocumentError::UnnamedElement {
                    kind: element.kind.clone(),
                });
            } else if seen.contains(&element.name.as_str()) {
                errors.push(DocumentError::DuplicateName(element.name.clone()));
            } else {
                seen.push(&element.name);
            }
            if kind(&element.kind).is_none() {
                errors.push(DocumentError::UnknownKind {
                    element: element.name.clone(),
                    kind: element.kind.clone(),
                });
            }
            let anchors_are_valid = element
                .anchor_min
                .iter()
                .chain(element.anchor_max.iter())
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                && element.anchor_min[0] <= element.anchor_max[0]
                && element.anchor_min[1] <= element.anchor_max[1];
            let offsets_are_valid = element.offsets.iter().all(|v| v.is_finite());
            let pivot_is_valid = element
                .pivot
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v));
            if !(anchors_are_valid && offsets_are_valid && pivot_is_valid) {
                errors.push(DocumentError::InvalidPlacement {
                    element: element.name.clone(),
                });
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Bring an older document up to [`CURRENT_VERSION`].
    ///
    /// Nothing to do yet, and that is the point of having it now: the first
    /// format change should be a match arm, not an argument about whether old
    /// files are worth reading.
    pub fn migrate(&mut self) {
        if self.version < CURRENT_VERSION {
            self.version = CURRENT_VERSION;
        }
    }

    /// Parse a `.somui` document, migrating and validating it.
    pub fn from_json(text: &str) -> Result<Self, Vec<DocumentError>> {
        let mut document: Self = serde_json::from_str(text)
            .map_err(|error| vec![DocumentError::Parse(error.to_string())])?;
        document.migrate();
        document.validate()?;
        Ok(document)
    }

    /// Serialise, pretty, for a file people will read in a diff.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Build this document into the same retained tree a hand-written game UI
    /// uses. Validation happens before the first node is added, so a failed
    /// load cannot leave half a HUD attached to the canvas.
    pub fn instantiate(&self, canvas: &mut UiCanvas) -> Result<UiInstance, Vec<DocumentError>> {
        self.validate()?;
        let mut instance = UiInstance::default();
        let root = canvas.ui().root();
        let viewport = canvas.ui().screen_size;
        // `UserInterface`'s root deliberately stretches each top-level child
        // to the window for editor-shell grids. A document needs an absolute
        // positioning host beneath that root so its authored root can itself
        // be pinned or inset.
        let host = canvas.ui_mut().add_node(
            crate::widgets::canvas::CanvasBuilder::new(WidgetBuilder::new()).build(),
            root,
        );
        instantiate_element(&self.root, None, host, canvas, &mut instance);
        instance.apply_layout(self, canvas, viewport);
        Ok(instance)
    }
}

/// One live retained-tree instance of a `.somui` document.
///
/// Names are the public interface. Node handles remain implementation details:
/// a script or game asks for `HealthBar`, not for a pool index that changes on
/// every load.
#[derive(Debug, Default)]
pub struct UiInstance {
    handles: BTreeMap<String, NodeHandle>,
    placements: Vec<InstancePlacement>,
}

#[derive(Clone, Debug)]
struct InstancePlacement {
    name: String,
    parent: Option<String>,
    handle: NodeHandle,
}

impl UiInstance {
    /// Find a live widget by its authored name.
    #[must_use]
    pub fn handle(&self, name: &str) -> Option<NodeHandle> {
        self.handles.get(name).copied()
    }

    /// Write one property to one live element, by name.
    ///
    /// The runtime half of MORROWIND-M2's *"load and instantiate from Rust and
    /// from Luau"*: a script says `Score`, `text`, `"1200"` and this is what
    /// makes that a widget change. Names all the way through — a handle is an
    /// implementation detail that changes on every load.
    ///
    /// `visible` is handled here rather than per kind because it is a property
    /// of the widget every kind already has, and six identical arms in the
    /// registry would be six chances to spell it differently.
    pub fn set_property(
        &self,
        document: &UiDocument,
        element: &str,
        property: &str,
        value: &Value,
        canvas: &mut UiCanvas,
    ) -> Result<(), PropertyError> {
        let Some(handle) = self.handle(element) else {
            return Err(PropertyError::UnknownElement(element.to_owned()));
        };
        if property == "visible" {
            let visible = value.as_bool().ok_or(PropertyError::WrongType {
                property: property.to_owned(),
                expected: "boolean",
            })?;
            canvas.ui_mut().set_visibility(handle, visible);
            return Ok(());
        }
        let Some(authored) = document.find(element) else {
            return Err(PropertyError::UnknownElement(element.to_owned()));
        };
        let Some(registered) = kind(&authored.kind) else {
            return Err(PropertyError::UnknownElement(element.to_owned()));
        };
        (registered.apply)(handle, property, value, canvas).map_err(|error| match error {
            // `no_kind_properties` cannot know which kind it was called for;
            // fill that in here, where it is known, so the message names the
            // element's actual kind rather than "this".
            PropertyError::UnknownProperty { property, .. } => PropertyError::UnknownProperty {
                kind: authored.kind.clone(),
                property,
            },
            other => other,
        })
    }

    /// Re-resolve every anchor after the viewport, aspect, or safe area changes.
    /// Parent rectangles are resolved before their children, matching document
    /// order, so nested anchors never accidentally resolve against the screen.
    pub fn apply_layout(&self, document: &UiDocument, canvas: &mut UiCanvas, viewport: Vec2) {
        let layout = canvas.apply_canvas(viewport);
        let mut rects: BTreeMap<&str, Rect> = BTreeMap::new();
        for placement in &self.placements {
            let Some(element) = document.find(&placement.name) else {
                continue;
            };
            let parent_rect = placement
                .parent
                .as_deref()
                .and_then(|name| rects.get(name).copied())
                .unwrap_or(layout.safe_rect);
            let rect = element.anchoring().resolve(parent_rect);
            // Retained child positions are local to their parent. Keep the
            // absolute rectangle for resolving grandchildren, but apply a
            // parent-relative rectangle to the live widget.
            let local = if placement.parent.is_some() {
                Rect::new(
                    rect.x - parent_rect.x,
                    rect.y - parent_rect.y,
                    rect.w,
                    rect.h,
                )
            } else {
                rect
            };
            canvas.place_node(placement.handle, local);
            rects.insert(&placement.name, rect);
        }
    }
}

fn instantiate_element(
    element: &UiElement,
    parent_name: Option<&str>,
    parent_handle: NodeHandle,
    canvas: &mut UiCanvas,
    instance: &mut UiInstance,
) {
    // Safe after document validation: every kind was found before any node was
    // attached, and the function pointer is the registry's construction seam.
    let registered = kind(&element.kind).expect("validated widget kind disappeared");
    let node = (registered.spawn)(element, WidgetBuilder::new().with_name(&element.name));
    let handle = canvas.ui_mut().add_node(node, parent_handle);
    instance.handles.insert(element.name.clone(), handle);
    instance.placements.push(InstancePlacement {
        name: element.name.clone(),
        parent: parent_name.map(str::to_owned),
        handle,
    });
    for child in &element.children {
        instantiate_element(child, Some(&element.name), handle, canvas, instance);
    }
}

/// The ids [`KINDS`] registers, so call sites do not spell them by hand.
pub mod kinds {
    pub const PANEL: &str = "panel";
    pub const TEXT: &str = "text";
    pub const BUTTON: &str = "button";
    pub const STACK: &str = "stack";
    pub const IMAGE: &str = "image";
    pub const CHECK_BOX: &str = "checkBox";
}

/// Why a live property write did not land.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyError {
    /// No element of that name is in this instance.
    UnknownElement(String),
    /// The element's kind does not understand that property.
    ///
    /// Named rather than ignored: a script setting `txt` instead of `text` on a
    /// HUD gets silence otherwise, and silence during play is the hardest thing
    /// there is to trace back to a typo.
    UnknownProperty { kind: String, property: String },
    /// The property exists but not with that type.
    WrongType {
        property: String,
        expected: &'static str,
    },
}

impl std::fmt::Display for PropertyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownElement(name) => write!(f, "no element named `{name}`"),
            Self::UnknownProperty { kind, property } => {
                write!(f, "a `{kind}` element has no property `{property}`")
            }
            Self::WrongType { property, expected } => {
                write!(f, "`{property}` takes a {expected}")
            }
        }
    }
}

/// One registered widget kind.
pub struct WidgetKind {
    /// The id a document writes.
    pub id: &'static str,
    /// What the palette shows.
    pub label: &'static str,
    /// The palette group it appears under.
    pub category: &'static str,
    /// Build one. `builder` already carries the element's placement.
    pub spawn: fn(&UiElement, WidgetBuilder) -> crate::node::UiNode,
    /// Write one property to a live widget of this kind.
    ///
    /// On the kind rather than in a `match` somewhere else, for the same reason
    /// `spawn` is: [`KINDS`] stays the one list, and a widget added to the
    /// authoring surface says how it is built and how it is driven in the same
    /// entry. Returns `Err` naming the property when the kind does not have
    /// one, so a typo in a script is a diagnostic and not silence.
    pub apply: fn(NodeHandle, &str, &Value, &mut UiCanvas) -> Result<(), PropertyError>,
}

/// Every widget kind a `.somui` document may name.
///
/// **One list.** The palette enumerates it, the loader looks up in it, and
/// [`UiDocument::validate`] rejects anything absent from it. Adding a widget to
/// the authoring surface is one entry here.
///
/// This is a starter set rather than all thirty-one builders: the point of step
/// 1 is that the seam exists and is exercised end to end, and a kind whose
/// properties nobody has designed yet would be a palette entry that produces a
/// widget nobody can configure.
pub static KINDS: &[WidgetKind] = &[
    WidgetKind {
        id: kinds::PANEL,
        label: "Panel",
        category: "Layout",
        // A document panel is an absolute-positioning parent. Using Border
        // here made its children all fill the same inner rect, silently
        // overriding the anchors the document had just resolved.
        spawn: |_element, builder| crate::widgets::canvas::CanvasBuilder::new(builder).build(),
        apply: no_kind_properties,
    },
    WidgetKind {
        id: kinds::TEXT,
        label: "Text",
        category: "Content",
        spawn: |element, builder| {
            let text = element
                .properties
                .get("text")
                .and_then(Value::as_text)
                .unwrap_or("Text");
            crate::widgets::text::TextBuilder::new(builder)
                .with_text(text)
                .build()
        },
        apply: |handle, property, value, canvas| match property {
            "text" => {
                let text = value.as_text().ok_or(PropertyError::WrongType {
                    property: property.to_owned(),
                    expected: "string",
                })?;
                canvas.ui_mut().send(crate::message::TextMessage::set_text(
                    handle,
                    text.to_owned(),
                ));
                Ok(())
            }
            _ => Err(PropertyError::UnknownProperty {
                kind: kinds::TEXT.to_owned(),
                property: property.to_owned(),
            }),
        },
    },
    WidgetKind {
        id: kinds::BUTTON,
        label: "Button",
        category: "Input",
        spawn: |_element, builder| crate::widgets::button::ButtonBuilder::new(builder).build(),
        apply: no_kind_properties,
    },
    WidgetKind {
        id: kinds::STACK,
        label: "Stack Panel",
        category: "Layout",
        spawn: |_element, builder| {
            crate::widgets::stack_panel::StackPanelBuilder::new(builder).build()
        },
        apply: no_kind_properties,
    },
    WidgetKind {
        id: kinds::IMAGE,
        label: "Image",
        category: "Content",
        spawn: |_element, builder| crate::widgets::image::ImageBuilder::new(builder).build(),
        apply: no_kind_properties,
    },
    WidgetKind {
        id: kinds::CHECK_BOX,
        label: "Check Box",
        category: "Input",
        spawn: |_element, builder| crate::widgets::check_box::CheckBoxBuilder::new(builder).build(),
        apply: no_kind_properties,
    },
];

/// A kind with no properties of its own.
///
/// Not a silent `Ok`: reaching here means the generic properties did not claim
/// it either, so the property does not exist anywhere and saying so is the
/// whole point.
fn no_kind_properties(
    _handle: NodeHandle,
    property: &str,
    _value: &Value,
    _canvas: &mut UiCanvas,
) -> Result<(), PropertyError> {
    Err(PropertyError::UnknownProperty {
        kind: "this".to_owned(),
        property: property.to_owned(),
    })
}

/// Look a kind up by id.
#[must_use]
pub fn kind(id: &str) -> Option<&'static WidgetKind> {
    KINDS.iter().find(|k| k.id == id)
}

/// The palette, grouped by category, in registration order.
///
/// Generated, never written down twice. A widget added to [`KINDS`] appears
/// here without anyone remembering to add it.
#[must_use]
pub fn palette() -> Vec<(&'static str, Vec<&'static WidgetKind>)> {
    let mut groups: Vec<(&'static str, Vec<&'static WidgetKind>)> = Vec::new();
    for entry in KINDS {
        match groups.iter_mut().find(|(name, _)| *name == entry.category) {
            Some((_, list)) => list.push(entry),
            None => groups.push((entry.category, vec![entry])),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hud() -> UiDocument {
        let mut document = UiDocument::new(Vec2::new(1920.0, 1080.0));
        document.root = UiElement::new(kinds::PANEL, "Root")
            .child(
                UiElement::new(kinds::TEXT, "Score")
                    .pinned([1.0, 0.0], Vec2::new(-220.0, 24.0), Vec2::new(200.0, 32.0))
                    .with("text", Value::Text("0".into())),
            )
            .child(UiElement::new(kinds::IMAGE, "HealthBar").pinned(
                [0.0, 1.0],
                Vec2::new(24.0, -48.0),
                Vec2::new(320.0, 24.0),
            ));
        document
    }

    // ── The registry is the list, and there is only one ────────────────────

    #[test]
    fn the_palette_is_generated_from_the_registry() {
        let palette = palette();
        let listed: usize = palette.iter().map(|(_, kinds)| kinds.len()).sum();
        assert_eq!(
            listed,
            KINDS.len(),
            "a registered kind is missing from the palette"
        );
        assert!(
            palette.iter().any(|(name, _)| *name == "Layout"),
            "categories should group: {:?}",
            palette.iter().map(|(n, _)| *n).collect::<Vec<_>>()
        );
    }

    #[test]
    fn every_registered_kind_is_reachable_by_its_id() {
        for entry in KINDS {
            assert!(kind(entry.id).is_some(), "{} is not findable", entry.id);
        }
        assert!(kind("no-such-widget").is_none());
    }

    #[test]
    fn kind_ids_are_unique() {
        // Two entries sharing an id makes `kind()` return whichever came first
        // and the second unreachable — a palette entry that builds the wrong
        // widget.
        let mut ids: Vec<&str> = KINDS.iter().map(|k| k.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate kind id");
    }

    // ── Documents ──────────────────────────────────────────────────────────

    #[test]
    fn a_document_round_trips_through_json() {
        let document = hud();
        let text = document.to_json();
        let back = UiDocument::from_json(&text).expect("a valid document must load");
        assert_eq!(back, document);
    }

    #[test]
    fn an_element_describes_a_placement_the_runtime_already_understands() {
        // The bridge to MORROWIND-E. A document that needed its own layout
        // rules would be a second placement system, which is the thing M2 is
        // explicitly not building.
        let document = hud();
        let score = document.find("Score").expect("Score is in the document");
        let anchoring = score.anchoring();
        assert_eq!(anchoring.anchors.min, Vec2::new(1.0, 0.0));
        assert_eq!(anchoring.anchors.max, Vec2::new(1.0, 0.0));
        assert_eq!(anchoring.offsets.left, -220.0);
    }

    #[test]
    fn a_document_naming_an_unknown_kind_is_refused_by_name() {
        // The failure that matters: a build without some widget must say which
        // element and which kind, not drop it and render a hole.
        let mut document = hud();
        document.root.children[0].kind = "hologram".into();
        let errors = document.validate().expect_err("this must not validate");
        assert!(
            errors.contains(&DocumentError::UnknownKind {
                element: "Score".into(),
                kind: "hologram".into(),
            }),
            "{errors:?}"
        );
    }

    #[test]
    fn duplicate_and_missing_names_are_both_errors() {
        let mut document = hud();
        document.root.children[1].name = "Score".into();
        let errors = document
            .validate()
            .expect_err("a duplicate must not validate");
        assert!(
            errors.contains(&DocumentError::DuplicateName("Score".into())),
            "{errors:?}"
        );

        let mut document = hud();
        document.root.children[0].name = String::new();
        let errors = document
            .validate()
            .expect_err("an unnamed element must not validate");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, DocumentError::UnnamedElement { .. })),
            "{errors:?}"
        );
    }

    #[test]
    fn validation_reports_every_problem_rather_than_the_first() {
        let mut document = hud();
        document.root.children[0].kind = "hologram".into();
        document.root.children[1].kind = "sparkles".into();
        let errors = document.validate().expect_err("two bad kinds");
        assert_eq!(errors.len(), 2, "{errors:?}");
    }

    #[test]
    fn a_document_from_a_newer_build_is_refused_rather_than_guessed_at() {
        let mut document = hud();
        document.version = CURRENT_VERSION + 1;
        let errors = document.validate().expect_err("the future must not load");
        assert!(
            errors.contains(&DocumentError::FromTheFuture {
                found: CURRENT_VERSION + 1,
                current: CURRENT_VERSION,
            }),
            "{errors:?}"
        );
    }

    #[test]
    fn an_older_document_is_migrated_rather_than_rejected() {
        let mut document = hud();
        document.version = 0;
        let text = serde_json::to_string(&document).unwrap();
        let loaded = UiDocument::from_json(&text).expect("an old document must still load");
        assert_eq!(loaded.version, CURRENT_VERSION);
    }

    #[test]
    fn malformed_json_reports_a_parse_error_instead_of_a_fake_widget_error() {
        let errors = UiDocument::from_json("not json").expect_err("this is not a document");
        assert!(matches!(errors.as_slice(), [DocumentError::Parse(_)]));
    }

    #[test]
    fn invalid_canvas_and_anchor_numbers_are_refused_before_instantiation() {
        let mut document = hud();
        document.reference[0] = f32::NAN;
        document.root.children[0].anchor_min[0] = 2.0;
        let errors = document
            .validate()
            .expect_err("invalid geometry must be refused");
        assert!(
            errors.contains(&DocumentError::InvalidReference),
            "{errors:?}"
        );
        assert!(
            errors.contains(&DocumentError::InvalidPlacement {
                element: "Score".into()
            }),
            "{errors:?}"
        );
    }

    #[test]
    fn a_document_instantiates_real_widgets_addressable_by_authored_name() {
        let document = hud();
        let mut canvas = UiCanvas::new(1920.0, 1080.0);
        let instance = document
            .instantiate(&mut canvas)
            .expect("the HUD is a valid runtime document");
        let score = instance.handle("Score").expect("authored name resolves");
        let health = instance
            .handle("HealthBar")
            .expect("second authored name resolves");
        assert!(score.is_some() && health.is_some());
        canvas.ui_mut().perform_layout();
        let score_bounds = canvas.ui().screen_bounds(score);
        assert!(
            (score_bounds.x - 1700.0).abs() < 0.5
                && (score_bounds.y - 24.0).abs() < 0.5
                && (score_bounds.w - 200.0).abs() < 0.5,
            "{score_bounds:?}"
        );
    }

    #[test]
    fn child_anchors_resolve_against_the_parent_not_the_screen() {
        let mut document = UiDocument::new(Vec2::new(800.0, 600.0));
        document.root = UiElement::new(kinds::PANEL, "Root")
            .pinned([0.0, 0.0], Vec2::new(100.0, 50.0), Vec2::new(400.0, 300.0))
            .child(UiElement::new(kinds::TEXT, "Corner").pinned(
                [1.0, 1.0],
                Vec2::new(-30.0, -20.0),
                Vec2::new(20.0, 10.0),
            ));
        let mut canvas = UiCanvas::new(800.0, 600.0);
        let instance = document.instantiate(&mut canvas).unwrap();
        canvas.ui_mut().perform_layout();
        let corner = canvas
            .ui()
            .screen_bounds(instance.handle("Corner").unwrap());
        assert_eq!(corner, Rect::new(470.0, 330.0, 20.0, 10.0));
    }

    // ── Driving a live document by name (MORROWIND-M2, item 4) ────────────

    #[test]
    fn a_property_written_by_name_reaches_the_live_widget() {
        let document = hud();
        let mut canvas = UiCanvas::new(1920.0, 1080.0);
        let instance = document.instantiate(&mut canvas).unwrap();

        instance
            .set_property(
                &document,
                "Score",
                "text",
                &Value::Text("1200".into()),
                &mut canvas,
            )
            .expect("text is a property of a text element");
        // Writes are messages, like every other write in this UI.
        canvas.ui_mut().update();

        // Read back through the accessibility probe rather than a new
        // inspection API: a `Text` control already reports its string as its
        // a11y name, and that is the same string a screen reader speaks.
        let handle = instance.handle("Score").unwrap();
        let probe = canvas.ui().a11y_probe(handle).expect("Score is live");
        assert_eq!(probe.name, "1200");
    }

    #[test]
    fn a_property_write_survives_the_relayout_that_follows_it() {
        // The order a game actually runs them in: a script writes, then the
        // frame re-resolves anchors. A relayout that rebuilt widgets would
        // silently undo the write.
        let document = hud();
        let mut canvas = UiCanvas::new(1920.0, 1080.0);
        let instance = document.instantiate(&mut canvas).unwrap();
        instance
            .set_property(
                &document,
                "Score",
                "text",
                &Value::Text("42".into()),
                &mut canvas,
            )
            .unwrap();
        canvas.ui_mut().update();
        instance.apply_layout(&document, &mut canvas, Vec2::new(2560.0, 1440.0));
        canvas.ui_mut().perform_layout();

        let handle = instance.handle("Score").unwrap();
        assert_eq!(canvas.ui().a11y_probe(handle).unwrap().name, "42");
    }

    #[test]
    fn visible_is_a_property_of_every_kind() {
        // Handled once, before kind dispatch. Six identical registry arms
        // would be six chances to spell it differently.
        let document = hud();
        let mut canvas = UiCanvas::new(1920.0, 1080.0);
        let instance = document.instantiate(&mut canvas).unwrap();
        for element in ["Root", "Score", "HealthBar"] {
            instance
                .set_property(
                    &document,
                    element,
                    "visible",
                    &Value::Bool(false),
                    &mut canvas,
                )
                .unwrap_or_else(|error| panic!("{element}: {error}"));
        }
    }

    #[test]
    fn a_misspelled_property_is_named_rather_than_ignored() {
        // The failure this exists for: a script setting `txt` instead of
        // `text` during play. Silence is the hardest thing there is to trace
        // back to a typo, so the error carries both the kind and the property.
        let document = hud();
        let mut canvas = UiCanvas::new(1920.0, 1080.0);
        let instance = document.instantiate(&mut canvas).unwrap();

        let error = instance
            .set_property(
                &document,
                "Score",
                "txt",
                &Value::Text("x".into()),
                &mut canvas,
            )
            .expect_err("a misspelling must not pass silently");
        assert_eq!(
            error,
            PropertyError::UnknownProperty {
                kind: kinds::TEXT.to_owned(),
                property: "txt".to_owned(),
            }
        );

        let error = instance
            .set_property(
                &document,
                "Nope",
                "text",
                &Value::Text("x".into()),
                &mut canvas,
            )
            .expect_err("an unknown element must not pass silently");
        assert_eq!(error, PropertyError::UnknownElement("Nope".to_owned()));

        let error = instance
            .set_property(&document, "Score", "text", &Value::Number(1.0), &mut canvas)
            .expect_err("a number is not a string");
        assert!(
            matches!(error, PropertyError::WrongType { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn every_registered_kind_names_its_own_kind_when_it_refuses() {
        // `no_kind_properties` cannot know which kind it was called for, so the
        // fill-in happens in `set_property`. If that ever regresses, every kind
        // starts reporting "this" and the message stops being actionable.
        let mut document = UiDocument::new(Vec2::new(1920.0, 1080.0));
        let mut root = UiElement::new(kinds::PANEL, "Root");
        for entry in KINDS {
            if entry.id == kinds::PANEL {
                continue;
            }
            root = root.child(UiElement::new(entry.id, format!("E_{}", entry.id)));
        }
        document.root = root;
        let mut canvas = UiCanvas::new(1920.0, 1080.0);
        let instance = document.instantiate(&mut canvas).unwrap();

        for entry in KINDS {
            if entry.id == kinds::PANEL {
                continue;
            }
            let name = format!("E_{}", entry.id);
            let error = instance
                .set_property(
                    &document,
                    &name,
                    "nonesuch",
                    &Value::Bool(true),
                    &mut canvas,
                )
                .expect_err("nonesuch is not a property of anything");
            assert_eq!(
                error,
                PropertyError::UnknownProperty {
                    kind: entry.id.to_owned(),
                    property: "nonesuch".to_owned(),
                },
                "{} reported the wrong kind",
                entry.id
            );
        }
    }

    #[test]
    fn relayout_keeps_authored_anchors_when_the_viewport_changes() {
        let document = hud();
        let mut canvas = UiCanvas::new(1920.0, 1080.0);
        let instance = document.instantiate(&mut canvas).unwrap();
        instance.apply_layout(&document, &mut canvas, Vec2::new(2560.0, 1440.0));
        canvas.ui_mut().perform_layout();
        let score = canvas.ui().screen_bounds(instance.handle("Score").unwrap());
        assert!((score.x - 2340.0).abs() < 0.5, "{score:?}");
    }
}
