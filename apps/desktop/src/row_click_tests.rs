use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, PointerEventButton, WindowAdapter, WindowEvent,
    },
    ComponentHandle, LogicalPosition,
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    export { DesignTokens } from "../ui/echo-tokens.slint";
    import { EntryView } from "../ui/entry-row.slint";
    export component RowFixture inherits Window {
        width: 500px; height: 180px;
        in property <bool> saved;
        in property <bool> blocked;
        in property <bool> copy-only;
        callback action(string);
        pure callback drag-data() -> data-transfer;
        EntryView {
            width: 500px; height: 180px;
            entry: { key: "opaque-row", body: "Synthetic content" };
            favorites: root.saved; reorder-enabled: true; quick-insert: true;
            input-blocked: root.blocked; copy-only: root.copy-only;
            action(command, key) => { root.action(command); }
            drag-data(key) => { return root.drag-data(); }
        }
    }
}

struct TestPlatform;
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

#[test]
fn left_click_inserts_history_and_saved_space_rows() {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    for saved in [false, true] {
        let ui = RowFixture::new().unwrap();
        crate::style::apply_style!(
            ui.global::<DesignTokens>(),
            &crate::style::StyleSnapshot::default(),
            false
        );
        ui.set_saved(saved);
        ui.on_drag_data(|| {
            let mut data = slint::DataTransfer::default();
            data.set_user_data(Rc::new("synthetic-saved-item"));
            data
        });
        let actions = Rc::new(RefCell::new(Vec::new()));
        let recorded = actions.clone();
        ui.on_action(move |action| recorded.borrow_mut().push(action.to_string()));
        ui.show().unwrap();
        let position = LogicalPosition::new(80., 70.);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        assert_eq!(*actions.borrow(), vec!["insert"], "saved={saved}");
        actions.borrow_mut().clear();
        ui.set_copy_only(true);
        click(&ui, position);
        assert_eq!(*actions.borrow(), vec!["copy"], "copy-only saved={saved}");
        actions.borrow_mut().clear();
        ui.set_blocked(true);
        click(&ui, position);
        assert!(actions.borrow().is_empty(), "blocked saved={saved}");
        ui.set_blocked(false);
        ui.set_copy_only(false);
        // Moving within the click tolerance must still activate the row.
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        let nearby = LogicalPosition::new(81., 71.);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position: nearby });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: nearby,
            button: PointerEventButton::Left,
        });
        assert_eq!(
            *actions.borrow(),
            vec!["insert"],
            "small movement saved={saved}"
        );
        if saved {
            actions.borrow_mut().clear();
            // The Copy action stays a button click, not a row insert.
            let copy_button = LogicalPosition::new(414., 24.);
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: copy_button,
            });
            click(&ui, copy_button);
            assert_eq!(*actions.borrow(), vec!["copy"]);
        }
        ui.hide().unwrap();
    }
}

fn click(ui: &RowFixture, position: LogicalPosition) {
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
