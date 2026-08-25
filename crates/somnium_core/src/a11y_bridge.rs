//! MORROWIND-I — the platform screen-reader adapter.
//!
//! `somnium_ui::a11y` builds the accessibility tree and converts it to an
//! [`accesskit::TreeUpdate`]. This module is the other half: it hands those
//! updates to the platform — UI Automation on Windows, NSAccessibility on
//! macOS, AT-SPI on Linux — through `accesskit_winit`.
//!
//! # The threading problem, and why there is a mutex here
//!
//! AccessKit's three handler traits are called **on a platform-dependent
//! thread**. A screen reader can ask for the tree at any moment, including
//! while the render thread is halfway through building the next one, and
//! including before the first frame has ever been drawn.
//!
//! Somnium's widget tree is not `Sync` and never will be — it is a pool of
//! boxed `dyn Control` owned by the main loop. So the handlers cannot walk it.
//! What they get instead is the **last published update**, behind a mutex: the
//! main loop publishes a full [`accesskit::TreeUpdate`] whenever the tree
//! changes, and a handler on any thread hands back whatever the most recent one
//! was.
//!
//! The cost is one frame of staleness in the worst case. The alternative —
//! making the widget tree shareable — is a change to every widget in the crate
//! for a consumer that reads it a few times a second.
//!
//! # What is verified and what is not
//!
//! The tree, the roles, the names, the conversion and the publish/handoff are
//! covered by tests. **Whether a real screen reader reads this well has not
//! been measured**, and `phase_MORROWIND.md` §14.5 already says this sub-phase
//! delivers no conformance claim. What is claimed: a correct, well-formed
//! AccessKit tree reaches the platform adapter, which is the necessary
//! condition everything else is built on.

use std::sync::{Arc, Mutex};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};

/// The last tree the main loop published, readable from a platform thread.
#[derive(Default)]
struct Shared {
    latest: Option<TreeUpdate>,
    /// Set when a platform adapter has asked for a tree at least once.
    ///
    /// Used to answer "is anything actually listening" without asking the
    /// platform, which matters because building the tree every frame for a user
    /// who has no screen reader running is pure cost.
    requested: bool,
    /// Actions a reader asked for, drained by the main loop.
    pending: Vec<ActionRequest>,
}

/// Handles for the platform's threads. Cloneable; all three traits share one.
#[derive(Clone, Default)]
struct Handlers {
    shared: Arc<Mutex<Shared>>,
}

impl ActivationHandler for Handlers {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let mut shared = self.shared.lock().ok()?;
        shared.requested = true;
        // `None` is correct when nothing has been published: AccessKit's
        // contract says the adapter supplies a placeholder and waits for the
        // real update, and it explicitly says **not** to return a placeholder
        // of our own. The first frame will publish one.
        shared.latest.clone()
    }
}

impl ActionHandler for Handlers {
    fn do_action(&mut self, request: ActionRequest) {
        // Queued rather than handled: this is a platform thread and the widget
        // tree belongs to the main loop. AccessKit's own documentation prefers
        // queueing to blocking here.
        if let Ok(mut shared) = self.shared.lock() {
            shared.pending.push(request);
        }
    }
}

impl DeactivationHandler for Handlers {
    fn deactivate_accessibility(&mut self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.requested = false;
            shared.latest = None;
        }
    }
}

/// The engine's accessibility bridge.
///
/// One per window. Created before the window is shown — `accesskit_winit`
/// **panics** if the window is already visible, which is why
/// `Engine::resumed` now builds the window invisible and shows it after
/// everything is initialised. That is a better startup anyway: no flash of an
/// unpainted window.
pub struct A11yBridge {
    adapter: accesskit_winit::Adapter,
    handlers: Handlers,
    /// The last tree published, to skip republishing an unchanged one.
    last: Option<somnium_ui::A11yTree>,
}

impl A11yBridge {
    /// Attach to a window that has not been shown yet.
    pub fn new(
        event_loop: &winit::event_loop::ActiveEventLoop,
        window: &winit::window::Window,
    ) -> Self {
        let handlers = Handlers::default();
        let adapter = accesskit_winit::Adapter::with_direct_handlers(
            event_loop,
            window,
            handlers.clone(),
            handlers.clone(),
            handlers.clone(),
        );
        Self {
            adapter,
            handlers,
            last: None,
        }
    }

    /// Route a window event to the platform adapter.
    ///
    /// Must be called for every event, not only the ones that look relevant:
    /// the adapter tracks window focus and geometry, and a reader whose idea of
    /// where the window is has gone stale points at the wrong place on screen.
    pub fn process_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) {
        self.adapter.process_event(window, event);
    }

    /// Whether a screen reader has asked for the tree.
    ///
    /// The gate on doing any of this work at all. For the overwhelming majority
    /// of runs this is `false` for the whole session and the accessibility path
    /// costs one atomic load per frame.
    pub fn is_active(&self) -> bool {
        self.handlers
            .shared
            .lock()
            .map(|s| s.requested)
            .unwrap_or(false)
    }

    /// Publish a tree, if it differs from the last one published.
    ///
    /// The comparison is on Somnium's own `A11yTree` rather than on the
    /// AccessKit update, because `TreeUpdate` is not `PartialEq` and because
    /// the Somnium tree is the thing that actually changed or did not.
    pub fn publish(&mut self, tree: somnium_ui::A11yTree) {
        if self.last.as_ref() == Some(&tree) {
            return;
        }
        let update = tree.to_accesskit();
        if let Ok(mut shared) = self.handlers.shared.lock() {
            shared.latest = Some(update.clone());
        }
        self.adapter.update_if_active(|| update);
        self.last = Some(tree);
    }

    /// Take the actions a screen reader has requested since the last call.
    ///
    /// Returns `(node id, action)` pairs. The caller maps the id back to a
    /// widget — id is the widget handle's index plus one, per
    /// `somnium_ui::a11y`.
    pub fn take_actions(&mut self) -> Vec<ActionRequest> {
        self.handlers
            .shared
            .lock()
            .map(|mut s| std::mem::take(&mut s.pending))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{Action, NodeId};

    fn a_tree() -> somnium_ui::A11yTree {
        let mut ui = somnium_ui::ui::UserInterface::new(800.0, 600.0);
        let root = ui.root();
        ui.add_node(
            somnium_ui::widgets::button::ButtonBuilder::new(
                somnium_ui::widget::WidgetBuilder::new().with_tooltip("Save"),
            )
            .build(),
            root,
        );
        ui.perform_layout();
        ui.a11y_tree()
    }

    /// The contract AccessKit states explicitly: return `None` rather than a
    /// placeholder when nothing has been published yet.
    #[test]
    fn an_activation_before_the_first_frame_returns_none_not_a_placeholder() {
        let mut handlers = Handlers::default();
        assert!(handlers.request_initial_tree().is_none());
        // ...and it recorded that somebody asked, which is the gate on doing
        // any of this work at all.
        assert!(handlers.shared.lock().unwrap().requested);
    }

    #[test]
    fn a_published_tree_is_what_a_platform_thread_gets() {
        let handlers = Handlers::default();
        let update = a_tree().to_accesskit();
        handlers.shared.lock().unwrap().latest = Some(update);

        let mut other_thread = handlers.clone();
        let got = std::thread::spawn(move || other_thread.request_initial_tree())
            .join()
            .expect("the handler panicked on a platform thread");
        let got = got.expect("nothing published to a thread that asked");
        assert!(
            got.nodes
                .iter()
                .any(|(_, n)| n.label().as_deref() == Some("Save")),
            "the published tree lost its content crossing threads"
        );
    }

    #[test]
    fn actions_queue_rather_than_blocking_and_drain_once() {
        let mut handlers = Handlers::default();
        handlers.do_action(ActionRequest {
            action: Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: NodeId(7),
            data: None,
        });
        handlers.do_action(ActionRequest {
            action: Action::Focus,
            target_tree: accesskit::TreeId::ROOT,
            target_node: NodeId(9),
            data: None,
        });

        let drained = std::mem::take(&mut handlers.shared.lock().unwrap().pending);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].target_node, NodeId(7));
        assert_eq!(drained[1].action, Action::Focus);
        // Draining twice must not replay: a click delivered twice is a click
        // the user did not make.
        assert!(handlers.shared.lock().unwrap().pending.is_empty());
    }

    #[test]
    fn deactivation_drops_the_tree_and_the_gate() {
        let mut handlers = Handlers::default();
        handlers.shared.lock().unwrap().latest = Some(a_tree().to_accesskit());
        let _ = handlers.request_initial_tree();
        assert!(handlers.shared.lock().unwrap().requested);

        handlers.deactivate_accessibility();
        assert!(!handlers.shared.lock().unwrap().requested);
        assert!(
            handlers.shared.lock().unwrap().latest.is_none(),
            "a deactivated adapter kept a tree it will never be asked for"
        );
    }

    /// A11yTree is compared, not TreeUpdate, and the comparison is what stops
    /// an idle shell publishing sixty identical trees a second.
    #[test]
    fn an_unchanged_tree_compares_equal() {
        assert_eq!(a_tree(), a_tree());
        let mut ui = somnium_ui::ui::UserInterface::new(800.0, 600.0);
        let root = ui.root();
        ui.add_node(
            somnium_ui::widgets::button::ButtonBuilder::new(
                somnium_ui::widget::WidgetBuilder::new().with_tooltip("Open"),
            )
            .build(),
            root,
        );
        ui.perform_layout();
        assert_ne!(a_tree(), ui.a11y_tree());
    }
}
