//! Headless authoring model for the `.somui` layout editor (MORROWIND-M2).
//!
//! The retained shell projects this model; it does not own the edit rules.
//! That keeps drag, anchor, aspect, safe-area, and undo behaviour testable
//! without a window or GPU and gives a future floating editor window the same
//! interface as the docked one.

use crate::runtime::{Anchors, SafeArea};
use crate::somui::{UiDocument, UiElement, kind};
use crate::types::Rect;
use glam::Vec2;

/// The preview surface surrounding an authored document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preview {
    pub resolution: Vec2,
    pub safe_area: SafeArea,
}

impl Preview {
    #[must_use]
    pub fn new(resolution: Vec2) -> Self {
        Self {
            resolution: resolution.max(Vec2::ONE),
            safe_area: SafeArea::NONE,
        }
    }

    /// Letterbox the authored resolution into the available editor viewport.
    #[must_use]
    pub fn artboard(self, available: Rect) -> Rect {
        let scale = (available.w / self.resolution.x)
            .min(available.h / self.resolution.y)
            .max(0.0);
        let size = self.resolution * scale;
        Rect::new(
            available.x + (available.w - size.x) * 0.5,
            available.y + (available.h - size.y) * 0.5,
            size.x,
            size.y,
        )
    }

    /// The safe-area overlay in authored document coordinates.
    #[must_use]
    pub fn safe_rect(self) -> Rect {
        self.safe_area
            .apply(Rect::new(0.0, 0.0, self.resolution.x, self.resolution.y))
    }
}

/// Four authoring handles for one element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorHandles {
    pub min: Vec2,
    pub max: Vec2,
    pub pivot: Vec2,
    pub rect: Rect,
}

/// One undoable `.somui` editing session.
#[derive(Clone, Debug)]
pub struct AuthoringSession {
    document: UiDocument,
    selected: Option<String>,
    preview: Preview,
    undo: Vec<UiDocument>,
    redo: Vec<UiDocument>,
}

impl AuthoringSession {
    #[must_use]
    pub fn new(document: UiDocument) -> Self {
        let preview = Preview::new(Vec2::from_array(document.reference));
        Self {
            document,
            selected: None,
            preview,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    #[must_use]
    pub fn document(&self) -> &UiDocument {
        &self.document
    }

    #[must_use]
    pub fn preview(&self) -> Preview {
        self.preview
    }

    pub fn set_preview(&mut self, preview: Preview) {
        self.preview = Preview::new(preview.resolution);
        self.preview.safe_area = preview.safe_area;
    }

    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Select by durable authored name, never by a transient tree index.
    pub fn select(&mut self, name: impl Into<String>) -> bool {
        let name = name.into();
        if self.document.find(&name).is_none() {
            return false;
        }
        self.selected = Some(name);
        true
    }

    /// Drop a registered palette kind into `parent` at a document-space point.
    /// The new element is pinned because a drag-to-place gesture names a
    /// position and size, not four stretch insets.
    pub fn place(
        &mut self,
        parent: &str,
        kind_id: &str,
        position: Vec2,
        size: Vec2,
    ) -> Option<String> {
        let registered = kind(kind_id)?;
        let name = self.unique_name(registered.label);
        self.checkpoint();
        let parent_element = find_mut(&mut self.document.root, parent)?;
        parent_element
            .children
            .push(UiElement::new(kind_id, &name).pinned([0.0, 0.0], position, size.max(Vec2::ONE)));
        self.selected = Some(name.clone());
        Some(name)
    }

    /// Translate the selected element in its parent's local space.
    pub fn move_selected(&mut self, delta: Vec2) -> bool {
        if !delta.is_finite() || delta == Vec2::ZERO {
            return false;
        }
        let Some(name) = self.selected.clone() else {
            return false;
        };
        if self.document.find(&name).is_none() {
            return false;
        }
        self.checkpoint();
        let element = find_mut(&mut self.document.root, &name).expect("selection was checked");
        element.offsets[0] += delta.x;
        element.offsets[1] += delta.y;
        if element.anchor_min[0] != element.anchor_max[0] {
            element.offsets[2] -= delta.x;
        }
        if element.anchor_min[1] != element.anchor_max[1] {
            element.offsets[3] -= delta.y;
        }
        true
    }

    /// Change anchors without moving the element on screen.
    pub fn set_selected_anchors(&mut self, anchors: Anchors) -> bool {
        if anchors.min.x > anchors.max.x
            || anchors.min.y > anchors.max.y
            || anchors.min.cmplt(Vec2::ZERO).any()
            || anchors.max.cmpgt(Vec2::ONE).any()
        {
            return false;
        }
        let Some(name) = self.selected.clone() else {
            return false;
        };
        let Some((rect, parent)) = self.rect_and_parent(&name) else {
            return false;
        };
        self.checkpoint();
        let element = find_mut(&mut self.document.root, &name).expect("selection was checked");
        let anchor_min = Vec2::new(
            parent.x + parent.w * anchors.min.x,
            parent.y + parent.h * anchors.min.y,
        );
        let anchor_max = Vec2::new(
            parent.x + parent.w * anchors.max.x,
            parent.y + parent.h * anchors.max.y,
        );
        element.anchor_min = anchors.min.to_array();
        element.anchor_max = anchors.max.to_array();
        element.offsets = [
            rect.x - anchor_min.x,
            rect.y - anchor_min.y,
            if anchors.min.x == anchors.max.x {
                rect.w
            } else {
                anchor_max.x - (rect.x + rect.w)
            },
            if anchors.min.y == anchors.max.y {
                rect.h
            } else {
                anchor_max.y - (rect.y + rect.h)
            },
        ];
        true
    }

    #[must_use]
    pub fn selected_handles(&self) -> Option<AnchorHandles> {
        let name = self.selected.as_deref()?;
        let element = self.document.find(name)?;
        let (rect, parent) = self.rect_and_parent(name)?;
        let min = Vec2::new(
            parent.x + parent.w * element.anchor_min[0],
            parent.y + parent.h * element.anchor_min[1],
        );
        let max = Vec2::new(
            parent.x + parent.w * element.anchor_max[0],
            parent.y + parent.h * element.anchor_max[1],
        );
        let pivot = Vec2::new(
            rect.x + rect.w * element.pivot[0],
            rect.y + rect.h * element.pivot[1],
        );
        Some(AnchorHandles {
            min,
            max,
            pivot,
            rect,
        })
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.document, previous));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.document, next));
        true
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.document.clone());
        self.redo.clear();
    }

    fn unique_name(&self, base: &str) -> String {
        if self.document.find(base).is_none() {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base}{n}"))
            .find(|name| self.document.find(name).is_none())
            .expect("the integer name space is finite only after memory is exhausted")
    }

    fn rect_and_parent(&self, name: &str) -> Option<(Rect, Rect)> {
        let root_parent = Rect::new(
            0.0,
            0.0,
            self.document.reference[0],
            self.document.reference[1],
        );
        rect_and_parent(&self.document.root, name, root_parent)
    }
}

fn find_mut<'a>(element: &'a mut UiElement, name: &str) -> Option<&'a mut UiElement> {
    if element.name == name {
        return Some(element);
    }
    element
        .children
        .iter_mut()
        .find_map(|child| find_mut(child, name))
}

fn rect_and_parent(element: &UiElement, name: &str, parent: Rect) -> Option<(Rect, Rect)> {
    let rect = element.anchoring().resolve(parent);
    if element.name == name {
        return Some((rect, parent));
    }
    element
        .children
        .iter()
        .find_map(|child| rect_and_parent(child, name, rect))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::somui::{UiDocument, kinds};

    fn session() -> AuthoringSession {
        AuthoringSession::new(UiDocument::new(Vec2::new(1920.0, 1080.0)))
    }

    #[test]
    fn the_artboard_preserves_aspect_and_centres() {
        let art =
            Preview::new(Vec2::new(1920.0, 1080.0)).artboard(Rect::new(0.0, 0.0, 1000.0, 1000.0));
        assert!((art.w - 1000.0).abs() < 0.01);
        assert!((art.h - 562.5).abs() < 0.01);
        assert!((art.y - 218.75).abs() < 0.01);
    }

    #[test]
    fn safe_area_is_a_visible_document_space_overlay() {
        let mut preview = Preview::new(Vec2::new(1000.0, 500.0));
        preview.safe_area = SafeArea {
            left: 40.0,
            top: 20.0,
            right: 10.0,
            bottom: 30.0,
        };
        assert_eq!(preview.safe_rect(), Rect::new(40.0, 20.0, 950.0, 450.0));
    }

    #[test]
    fn palette_drop_places_a_registered_widget_and_selects_it() {
        let mut editor = session();
        let name = editor
            .place(
                "Root",
                kinds::BUTTON,
                Vec2::new(80.0, 60.0),
                Vec2::new(160.0, 36.0),
            )
            .unwrap();
        assert_eq!(name, "Button");
        assert_eq!(editor.selected(), Some("Button"));
        assert!(editor.document().find("Button").is_some());
        assert!(
            editor
                .place("Root", "not-registered", Vec2::ZERO, Vec2::ONE)
                .is_none()
        );
    }

    #[test]
    fn repeated_palette_kinds_receive_durable_unique_names() {
        let mut editor = session();
        assert_eq!(
            editor.place("Root", kinds::TEXT, Vec2::ZERO, Vec2::ONE),
            Some("Text".into())
        );
        assert_eq!(
            editor.place("Root", kinds::TEXT, Vec2::ZERO, Vec2::ONE),
            Some("Text2".into())
        );
    }

    #[test]
    fn dragging_moves_in_parent_local_space_and_undo_is_one_operation() {
        let mut editor = session();
        editor.place(
            "Root",
            kinds::TEXT,
            Vec2::new(10.0, 20.0),
            Vec2::new(80.0, 20.0),
        );
        editor.move_selected(Vec2::new(7.0, -3.0));
        let moved = editor.document().find("Text").unwrap();
        assert_eq!(&moved.offsets[..2], &[17.0, 17.0]);
        assert!(editor.undo());
        let restored = editor.document().find("Text").unwrap();
        assert_eq!(&restored.offsets[..2], &[10.0, 20.0]);
        assert!(editor.redo());
        assert_eq!(
            &editor.document().find("Text").unwrap().offsets[..2],
            &[17.0, 17.0]
        );
    }

    #[test]
    fn changing_anchors_preserves_the_visible_rectangle() {
        let mut editor = session();
        editor.place(
            "Root",
            kinds::BUTTON,
            Vec2::new(100.0, 80.0),
            Vec2::new(200.0, 40.0),
        );
        let before = editor.selected_handles().unwrap().rect;
        assert!(editor.set_selected_anchors(Anchors::CENTRE));
        let after = editor.selected_handles().unwrap().rect;
        assert_eq!(before, after);
    }

    #[test]
    fn handles_are_derived_from_the_same_anchor_model_runtime_uses() {
        let mut editor = session();
        editor.place(
            "Root",
            kinds::TEXT,
            Vec2::new(20.0, 30.0),
            Vec2::new(100.0, 20.0),
        );
        let handles = editor.selected_handles().unwrap();
        assert_eq!(handles.min, Vec2::ZERO);
        assert_eq!(handles.max, Vec2::ZERO);
        assert_eq!(handles.rect, Rect::new(20.0, 30.0, 100.0, 20.0));
    }
}
