//! Holding instantiated `.somui` documents so script can address them
//! (MORROWIND-M2).
//!
//! [`UiDocuments`] is the registry a game keeps: named documents, each with its
//! parsed [`UiDocument`], its live [`UiInstance`], and the [`UiCanvas`] it was
//! instantiated into. It implements
//! [`UiDocumentSink`](crate::script_host::UiDocumentSink), which is the whole
//! reason it exists — `ctx:setUiProperty("hud", "Score", "text", "1200")` has to
//! land somewhere, and that somewhere belongs to the game.
//!
//! # Why the engine ships one rather than requiring each game to write it
//!
//! The sink is a trait so a game with an unusual arrangement can implement it.
//! Almost none will: the ordinary case is *"I have some documents, they have
//! names"*, and making every game write the same thirty lines is how a seam
//! that was meant to be flexible becomes a tax. A game with one HUD registers
//! it and is done.
//!
//! # Names, all the way through
//!
//! A document is registered under a name; an element is addressed by the name
//! its author gave it; a property by the key the widget kind understands.
//! Nothing in this path is an index. A `.somui` reordered in the editor, or a
//! widget pool that hands out different handles on the next load, leaves every
//! script that drove it still correct.

use somnium_script::command::UiValue;
use somnium_ui::somui::{UiDocument, UiInstance, Value};

use crate::UiCanvas;
use crate::script_host::UiDocumentSink;

/// One registered document and the widgets it produced.
pub struct LoadedDocument {
    /// The parsed document. Kept because a live write needs to know the
    /// element's *kind* to dispatch, and the instance holds handles, not kinds.
    pub document: UiDocument,
    /// Handles by authored name.
    pub instance: UiInstance,
    /// The canvas it lives in. One per document: two documents sharing a canvas
    /// would fight over the root's children on reload.
    pub canvas: UiCanvas,
}

impl LoadedDocument {
    /// Re-resolve anchors for a viewport, then hand the canvas to the frame.
    ///
    /// The order matters and is easy to get backwards: layout first, draw
    /// second. A frame drawn before the relayout shows the previous viewport's
    /// arrangement for one frame, which reads as a flicker on every resize.
    pub fn relayout(&mut self, viewport: glam::Vec2) {
        self.instance
            .apply_layout(&self.document, &mut self.canvas, viewport);
    }
}

/// A game's authored UI documents, addressed by name.
#[derive(Default)]
pub struct UiDocuments {
    entries: Vec<(String, LoadedDocument)>,
}

impl UiDocuments {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse, instantiate and register a document under `name`.
    ///
    /// Returns the validation errors rather than panicking: a `.somui` is an
    /// asset, and a game shipping a broken one should be able to say so and
    /// keep running rather than fail to start.
    pub fn load(
        &mut self,
        name: impl Into<String>,
        json: &str,
    ) -> Result<(), Vec<somnium_ui::somui::DocumentError>> {
        let document = UiDocument::from_json(json)?;
        let mut canvas = UiCanvas::new(document.reference[0], document.reference[1]);
        let instance = document.instantiate(&mut canvas)?;
        let name = name.into();
        let loaded = LoadedDocument {
            document,
            instance,
            canvas,
        };
        // Replace rather than append: reloading a document under a name that is
        // already taken is a hot reload, and keeping both would leave the old
        // widgets drawn underneath the new ones.
        match self.entries.iter_mut().find(|(key, _)| *key == name) {
            Some(entry) => entry.1 = loaded,
            None => self.entries.push((name, loaded)),
        }
        Ok(())
    }

    /// A registered document, by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&LoadedDocument> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// A registered document, mutably.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut LoadedDocument> {
        self.entries
            .iter_mut()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// Every registered document, for a frame that draws all of them.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut LoadedDocument> {
        self.entries.iter_mut().map(|(_, value)| value)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl UiDocumentSink for UiDocuments {
    fn set_property(
        &mut self,
        document: &str,
        element: &str,
        property: &str,
        value: &UiValue,
    ) -> Result<(), String> {
        let Some(loaded) = self.get_mut(document) else {
            return Err(format!("no document is registered as `{document}`"));
        };
        // The two vocabularies meet here and only here. `somnium_script` must
        // not learn `somnium_ui`'s types, so the narrow script value is widened
        // into the document value at the one point that knows both.
        let value = match value {
            UiValue::Bool(value) => Value::Bool(*value),
            UiValue::Number(number) => Value::Number(*number),
            UiValue::Text(text) => Value::Text(text.clone()),
        };
        loaded
            .instance
            .set_property(
                &loaded.document,
                element,
                property,
                &value,
                &mut loaded.canvas,
            )
            .map_err(|error| format!("`{document}`: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUD: &str = r#"{
        "version": 1,
        "reference": [1920.0, 1080.0],
        "root": {
            "kind": "panel", "name": "Root",
            "anchor_min": [0.0, 0.0], "anchor_max": [1.0, 1.0],
            "offsets": [0.0, 0.0, 0.0, 0.0], "pivot": [0.0, 0.0],
            "children": [{
                "kind": "text", "name": "Score",
                "anchor_min": [1.0, 0.0], "anchor_max": [1.0, 0.0],
                "offsets": [-232.0, 28.0, 204.0, 32.0], "pivot": [0.0, 0.0],
                "properties": { "text": { "Text": "0" } }
            }]
        }
    }"#;

    fn loaded() -> UiDocuments {
        let mut documents = UiDocuments::new();
        documents.load("hud", HUD).expect("the fixture is valid");
        documents
    }

    #[test]
    fn a_script_write_reaches_the_widget_it_names() {
        // The full MORROWIND-M2 chain below the Luau boundary: the value a
        // script produced, through the sink, into a live retained widget.
        let mut documents = loaded();
        documents
            .set_property("hud", "Score", "text", &UiValue::Text("1200".into()))
            .expect("Score has a text property");

        let hud = documents.get_mut("hud").unwrap();
        hud.canvas.ui_mut().update();
        let handle = hud.instance.handle("Score").unwrap();
        assert_eq!(hud.canvas.ui().a11y_probe(handle).unwrap().name, "1200");
    }

    #[test]
    fn every_wrong_address_says_which_part_was_wrong() {
        // Three ways to miss, three different messages. A single "failed" would
        // leave a script author guessing which of the three names was the typo.
        let mut documents = loaded();
        let missing_document = documents
            .set_property("hud2", "Score", "text", &UiValue::Text("x".into()))
            .expect_err("hud2 is not registered");
        assert!(missing_document.contains("hud2"), "{missing_document}");

        let missing_element = documents
            .set_property("hud", "Nope", "text", &UiValue::Text("x".into()))
            .expect_err("Nope is not an element");
        assert!(missing_element.contains("Nope"), "{missing_element}");

        let missing_property = documents
            .set_property("hud", "Score", "txt", &UiValue::Text("x".into()))
            .expect_err("txt is not a property");
        assert!(missing_property.contains("txt"), "{missing_property}");
    }

    #[test]
    fn reloading_a_name_replaces_rather_than_stacks() {
        // A hot reload must not leave the previous widgets drawn underneath.
        let mut documents = loaded();
        documents.load("hud", HUD).expect("reload");
        assert!(documents.get("hud").is_some());
        assert_eq!(documents.iter_mut().count(), 1);
    }

    #[test]
    fn a_broken_document_is_reported_rather_than_panicking() {
        let mut documents = UiDocuments::new();
        let errors = documents
            .load(
                "hud",
                r#"{ "version": 1, "reference": [1.0, 1.0], "root": {
                "kind": "hologram", "name": "Root",
                "anchor_min": [0.0,0.0], "anchor_max": [1.0,1.0],
                "offsets": [0.0,0.0,0.0,0.0], "pivot": [0.0,0.0] } }"#,
            )
            .expect_err("an unknown kind must not load");
        assert!(!errors.is_empty());
        assert!(documents.is_empty(), "a failed load must register nothing");
    }
}
