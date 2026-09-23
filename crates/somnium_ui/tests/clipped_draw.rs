use glam::Vec2;
use somnium_ui::{
    draw::DrawingContext,
    node::{Control, UiNode},
    ui::UserInterface,
    widget::{Widget, WidgetBuilder},
    widgets::{scroll_viewer::ScrollViewerBuilder, stack_panel::StackPanelBuilder},
};
use std::sync::{Arc, Mutex};

struct Row(usize, Arc<Mutex<Vec<usize>>>);
impl Control for Row {
    fn draw(&self, _: &Widget, _: &mut DrawingContext) {
        self.1.lock().unwrap().push(self.0);
    }
}

#[test]
fn scrolled_out_rows_do_not_shape_or_draw_their_contents() {
    let mut ui = UserInterface::new(800.0, 600.0);
    let root = ui.root();
    let scroll = ui.add_node(ScrollViewerBuilder::new(WidgetBuilder::new()).build(), root);
    let stack = ui.add_node(StackPanelBuilder::new(WidgetBuilder::new()).build(), scroll);
    let drawn = Arc::new(Mutex::new(Vec::new()));
    for id in 0..2_000 {
        ui.add_node(
            UiNode::new(
                WidgetBuilder::new().with_height(24.0).build(),
                Box::new(Row(id, drawn.clone())),
            ),
            stack,
        );
    }
    ui.perform_layout();
    ui.draw();
    assert_eq!(*drawn.lock().unwrap(), (0..25).collect::<Vec<_>>());
    // A floating panel uses its own clip, not the main window's bounds.
    ui.detach(scroll, Vec2::new(320.0, 240.0));
    ui.perform_layout();
    drawn.lock().unwrap().clear();
    assert!(ui.draw_detached(scroll));
    assert_eq!(*drawn.lock().unwrap(), (0..10).collect::<Vec<_>>());
}
