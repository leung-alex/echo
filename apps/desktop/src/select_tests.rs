use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, PointerEventButton, WindowAdapter, WindowEvent,
    },
    ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel,
};
use std::{cell::Cell, rc::Rc, time::Duration};

slint::slint! {
    export { DesignTokens } from "../ui/echo-tokens.slint";
    export { SelectEnvironment } from "../ui/echo-select.slint";
    import { EchoSelect } from "../ui/echo-select.slint";
    import { EchoTheme } from "../ui/theme.slint";
    export component SelectFixture inherits Window {
        width: 400px; height: 480px; background: EchoTheme.canvas;
        default-font-family: "Microsoft YaHei UI";
        in property <[string]> choices;
        in-out property <int> selected: 1;
        in property <bool> enabled: true;
        in property <length> field-y: 40px;
        out property <bool> expanded: choice.expanded;
        callback picked(int);
        choice := EchoSelect {
            x: 40px; y: root.field-y; width: 300px;
            model: root.choices; selected-index: root.selected;
            enabled: root.enabled; accessible-label: "Fixture select";
            selection-changed(index) => { root.selected = index; root.picked(index); }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

#[test]
fn select_text_geometry_is_stable_during_opening_animation() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let now = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), now.clone()))).unwrap();
    let ui = SelectFixture::new().unwrap();
    ui.global::<SelectEnvironment>()
        .on_filter(crate::select::filter);
    ui.global::<SelectEnvironment>().set_window_width(400.);
    ui.global::<SelectEnvironment>().set_window_height(480.);
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.set_choices(ModelRc::new(VecModel::from(vec![
        "历史记录".into(),
        "收藏".into(),
        "上次使用的空间".into(),
    ])));
    ui.set_selected(2);
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(400, 480));
    let capture = |millis| {
        now.set(Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 400 * 480];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 400);
        });
        if let Some(directory) = std::env::var_os("ECHO_SELECT_RENDER_EVIDENCE") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            image::save_buffer(
                directory.join(format!("select-motion-{millis}.png")),
                &bytes,
                400,
                480,
                image::ColorType::Rgb8,
            )
            .unwrap();
        }
        (0..3)
            .map(|row| {
                // Normalize the ink threshold against each row's current fade level.
                let top = 88 + row * 36;
                let darkest = (top..top + 36)
                    .flat_map(|y| (74..320).map(move |x| y * 400 + x))
                    .map(|i| pixels[i].r)
                    .min()
                    .unwrap();
                let background = pixels[(top + 2) * 400 + 310].r;
                assert!(
                    background > darkest + 30,
                    "fixture must contain visible text at {millis}ms"
                );
                let threshold = darkest as u16 + (background as u16 - darkest as u16) / 2;
                let mut bounds = (400usize, 480usize, 0usize, 0usize);
                for y in top..top + 36 {
                    for x in 74..320 {
                        if (pixels[y * 400 + x].r as u16) < threshold {
                            bounds = (
                                bounds.0.min(x),
                                bounds.1.min(y),
                                bounds.2.max(x),
                                bounds.3.max(y),
                            );
                        }
                    }
                }
                bounds
            })
            .collect::<Vec<_>>()
    };
    // Layout before input; the first capture is intentionally outside the menu.
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut vec![slint::Rgb8Pixel::default(); 400 * 480], 400);
    });
    let position = LogicalPosition::new(100., 58.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    now.set(Duration::from_millis(2));
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut vec![slint::Rgb8Pixel::default(); 400 * 480], 400);
    });
    let early = capture(42);
    let middle = capture(152);
    let settled = capture(402);
    assert_eq!(
        early, settled,
        "text must not resize or move while the panel opens"
    );
    assert_eq!(
        middle, settled,
        "text must not snap at the end of the animation"
    );
    assert_eq!(
        capture(1102),
        settled,
        "no late font/layout change after opening"
    );
    ui.hide().unwrap();
}

#[test]
fn select_popup_commit_cancel_search_and_outside_click() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let now = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), now.clone()))).unwrap();
    let ui = SelectFixture::new().unwrap();
    ui.global::<SelectEnvironment>()
        .on_filter(crate::select::filter);
    ui.global::<SelectEnvironment>().set_window_width(400.);
    ui.global::<SelectEnvironment>().set_window_height(480.);
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.set_choices(ModelRc::new(VecModel::from(vec![
        "Alpha".into(),
        "Beta".into(),
        "Gamma".into(),
    ])));
    let calls = Rc::new(Cell::new(0));
    let recorded = calls.clone();
    ui.on_picked(move |_| recorded.set(recorded.get() + 1));
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(400, 480));
    let draw = || {
        let mut pixels = vec![slint::Rgb8Pixel::default(); 400 * 480];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 400);
        });
        pixels
    };
    let tick = |millis| {
        now.set(now.get() + Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
        draw();
    };
    let click = |x, y| {
        let position = LogicalPosition::new(x, y);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        draw();
    };
    let key = |text: SharedString| {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
        draw();
    };
    let open = || {
        click(100., ui.get_field_y() + 18.);
        assert!(ui.get_expanded());
        tick(2);
        tick(360);
    };
    draw();
    assert_eq!(calls.get(), 0, "initial value is not a user edit");
    open();
    if let Some(directory) = std::env::var_os("ECHO_SELECT_RENDER_EVIDENCE") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let bytes: Vec<u8> = draw().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        image::save_buffer(
            directory.join("select-open.png"),
            &bytes,
            400,
            480,
            image::ColorType::Rgb8,
        )
        .unwrap();
    }
    click(130., 178.);
    assert_eq!(ui.get_selected(), 1, "selection waits until popup exit");
    tick(210);
    assert!(!ui.get_expanded());
    assert_eq!(ui.get_selected(), 2);
    assert_eq!(calls.get(), 1);
    open();
    key(slint::platform::Key::UpArrow.into());
    key(slint::platform::Key::Escape.into());
    tick(210);
    assert_eq!(
        ui.get_selected(),
        2,
        "Escape must discard highlighted choice"
    );
    assert!(!ui.get_expanded());
    open();
    click(380., 450.);
    tick(210);
    assert!(!ui.get_expanded(), "outside click must dismiss");
    assert_eq!(calls.get(), 1);
    ui.set_choices(ModelRc::new(VecModel::from(
        (0..16)
            .map(|i| {
                if i == 14 {
                    "工作 Project".into()
                } else {
                    format!("Space {i}").into()
                }
            })
            .collect::<Vec<SharedString>>(),
    )));
    open();
    key("工作".into());
    if let Some(directory) = std::env::var_os("ECHO_SELECT_RENDER_EVIDENCE") {
        let bytes: Vec<u8> = draw().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        image::save_buffer(
            std::path::PathBuf::from(directory).join("select-search.png"),
            &bytes,
            400,
            480,
            image::ColorType::Rgb8,
        )
        .unwrap();
    }
    key(slint::platform::Key::Return.into());
    tick(210);
    assert_eq!(
        ui.get_selected(),
        14,
        "filtered choice must retain source index"
    );
    assert_eq!(calls.get(), 2);
    open();
    key("unmatched".into());
    key(slint::platform::Key::Return.into());
    assert_eq!(calls.get(), 2, "empty search cannot select");
    key(slint::platform::Key::Escape.into());
    tick(210);
    ui.set_enabled(false);
    click(100., 58.);
    assert!(!ui.get_expanded());
    ui.set_enabled(true);
    ui.set_field_y(420.);
    open();
    key("工作".into());
    key(slint::platform::Key::Return.into());
    tick(210);
    assert_eq!(
        ui.get_selected(),
        14,
        "upward popup must also select correctly"
    );
    ui.hide().unwrap();
}

#[test]
fn actual_settings_selects_update_drafts_and_survive_category_change() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let now = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), now.clone()))).unwrap();
    let ui = crate::AppWindow::new().unwrap();
    ui.global::<crate::SelectEnvironment>()
        .on_filter(crate::select::filter);
    crate::style::apply_style!(
        ui.global::<crate::DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.set_route("settings".into());
    ui.set_theme_mode("light".into());
    ui.set_language("en".into());
    ui.set_global_hotkey("Ctrl+Alt+J".into());
    ui.set_global_hotkey_enabled(true);
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(960, 720));
    let draw = || {
        let size = window.size();
        let mut pixels = vec![slint::Rgb8Pixel::default(); (size.width * size.height) as usize];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, size.width as usize);
        });
    };
    let tick = |millis| {
        now.set(now.get() + Duration::from_millis(millis));
        slint::platform::update_timers_and_animations();
        draw();
    };
    let click = |x, y| {
        let position = LogicalPosition::new(x, y);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        draw();
    };
    let key = |key: slint::platform::Key| {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
        draw();
    };
    draw();
    click(400., 185.);
    tick(2);
    tick(360);
    key(slint::platform::Key::DownArrow);
    key(slint::platform::Key::Return);
    tick(210);
    assert_eq!(ui.get_theme_mode(), "dark");
    click(400., 263.);
    tick(2);
    tick(360);
    key(slint::platform::Key::UpArrow);
    key(slint::platform::Key::Return);
    tick(210);
    assert_eq!(ui.get_language(), "zh-CN");
    window.set_size(slint::PhysicalSize::new(320, 720));
    draw();
    click(120., 104.);
    tick(2);
    tick(360);
    key(slint::platform::Key::DownArrow);
    key(slint::platform::Key::DownArrow);
    key(slint::platform::Key::Return);
    tick(210);
    click(200., 251.);
    assert_eq!(
        ui.get_global_hotkey(),
        "Alt+V",
        "category popup must yield to new page controls"
    );
    ui.hide().unwrap();
}
