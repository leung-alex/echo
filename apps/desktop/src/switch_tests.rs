use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, PointerEventButton, WindowAdapter, WindowEvent,
    },
    ComponentHandle, LogicalPosition,
};
use std::rc::Rc;

slint::slint! {
    export { DesignTokens } from "../ui/echo-tokens.slint";
    import { EchoSwitch } from "../ui/echo-switch.slint";
    import { LineEdit } from "../ui/echo-lineedit.slint";
    import { TextEdit } from "../ui/echo-textedit.slint";
    export { EchoTheme } from "../ui/theme.slint";
    import { Palette } from "std-widgets.slint";
    export component InputAccentFixture inherits Window {
        width: 280px; height: 160px;
        in property <bool> dark;
        changed dark => { Palette.color-scheme = root.dark ? ColorScheme.dark : ColorScheme.light; }
        public function select-input(multiline: bool) {
            if multiline { multi.focus(); multi.select-all(); }
            else { single.focus(); single.select-all(); }
        }
        single := LineEdit { x: 10px; y: 10px; width: 260px; height: 36px; text: "Selected text"; }
        multi := TextEdit { x: 10px; y: 60px; width: 260px; height: 90px; text: "Selected text"; }
    }
    import { FavoriteIconPicker, FavoriteIconChoice } from "../ui/favorite-icon-picker.slint";
    export { FavoriteIconChoice } from "../ui/favorite-icon-picker.slint";
    export { FavoriteIconImages } from "../ui/favorite-icon-images.slint";
    export component IconGridFixture inherits Window {
        width: 500px; height: 540px;
        in property <[FavoriteIconChoice]> choices;
        out property <string> chosen-key;
        out property <bool> dismissed;
        FavoriteIconPicker {
            choices: root.choices;
            chosen(key) => { root.chosen-key = key; }
            dismissed => { root.dismissed = true; }
        }
    }
    export component SwitchFixture inherits Window {
        width: 240px; height: 60px;
        in-out property <bool> checked;
        in property <bool> enabled: true;
        EchoSwitch {
            x: 10px; y: 10px; width: 220px; height: 36px;
            text: "Switch label";
            checked <=> root.checked; enabled: root.enabled;
        }
    }
}

struct TestPlatform(
    Rc<MinimalSoftwareWindow>,
    Rc<std::cell::Cell<std::time::Duration>>,
);

#[test]
fn input_badge_keeps_size_black_white_style_and_transparent_corners() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let now = Rc::new(std::cell::Cell::new(std::time::Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), now))).unwrap();
    let ui = crate::InputIndicatorWindow::new().unwrap();
    ui.set_animations(false);
    ui.set_reveal(true);
    ui.show().unwrap();
    for scale in [1., 1.25, 1.5, 2.] {
        ui.window().dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        window.set_size(slint::PhysicalSize::new(
            (48. * scale) as u32,
            (36. * scale) as u32,
        ));
        for mode in ["中", "EN"] {
            ui.set_mode(mode.into());
            let image = ui.window().take_snapshot().unwrap();
            assert_eq!(
                (image.width(), image.height()),
                ((48. * scale) as u32, (36. * scale) as u32)
            );
            assert_eq!(image.as_slice()[0].a, 0);
            assert!(image
                .as_slice()
                .iter()
                .any(|p| p.a == 255 && p.r == 0 && p.g == 0 && p.b == 0));
            assert!(image
                .as_slice()
                .iter()
                .any(|p| p.a == 255 && p.r == 255 && p.g == 255 && p.b == 255));
            {
                let margin = (4. * scale) as usize;
                let width = image.width() as usize;
                let height = image.height() as usize;
                let glyph: Vec<_> = image
                    .as_slice()
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.a == 255 && p.r > 32)
                    .map(|(i, _)| (i % width, i / width))
                    .collect();
                assert!(!glyph.is_empty(), "missing glyph: {mode} at {scale}");
                assert!(
                    glyph.iter().all(|&(x, y)| {
                        x >= margin && x < width - margin && y >= margin && y < height - margin
                    }),
                    "clipped glyph: {mode} at {scale}"
                );
            }
        }
    }
}
#[test]
fn input_badge_flip_is_centered_serial_and_settles_to_latest_mode() {
    use crate::IndicatorFlipPhase::{FlippingIn, FlippingOut, Idle};
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let now = Rc::new(std::cell::Cell::new(std::time::Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), now.clone()))).unwrap();
    let ui = crate::InputIndicatorWindow::new().unwrap();
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(48, 36));
    let frame_index = std::cell::Cell::new(0);
    let tick = |ms| {
        now.set(now.get() + std::time::Duration::from_millis(ms));
        slint::platform::update_timers_and_animations();
        let frame = ui.window().take_snapshot().unwrap();
        if let Some(directory) = std::env::var_os("ECHO_FLIP_TEST_FRAMES") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            image::save_buffer(
                directory.join(format!("{:03}.png", frame_index.get())),
                frame.as_bytes(),
                frame.width(),
                frame.height(),
                image::ColorType::Rgba8,
            )
            .unwrap();
            frame_index.set(frame_index.get() + 1);
        }
        assert_eq!((frame.width(), frame.height()), (48, 36));
        assert_eq!(ui.window().size(), slint::PhysicalSize::new(48, 36));
        assert!((ui.get_flip_x() * 2. + ui.get_flip_width() - 48.).abs() < 0.01);
    };
    ui.set_mode("中".into());
    ui.set_reveal(true);
    tick(0);
    assert_eq!(ui.get_displayed_mode(), "中");
    assert_eq!(ui.get_flip_phase(), Idle);
    tick(120);
    for (old, new) in [("中", "EN"), ("EN", "中")] {
        ui.set_mode(new.into());
        tick(0);
        assert_eq!(ui.get_displayed_mode(), old);
        assert_eq!(ui.get_flip_phase(), FlippingOut);
        tick(40);
        assert!(ui.get_flip_width() > 3. && ui.get_flip_width() < 48.);
        assert_eq!(ui.get_displayed_mode(), old);
        tick(40);
        assert_eq!(ui.get_flip_phase(), FlippingIn);
        assert_eq!(ui.get_displayed_mode(), new);
        assert!(
            (ui.get_flip_width() - 3.).abs() < 0.01,
            "midpoint width {}",
            ui.get_flip_width()
        );
        tick(40);
        assert!(ui.get_flip_width() > 3. && ui.get_flip_width() < 48.);
        tick(40);
        assert_eq!(ui.get_flip_phase(), Idle);
        assert_eq!(ui.get_flip_width(), 48.);
    }
    // Updates during both halves coalesce; only the latest target survives.
    for (initial, other) in [("中", "EN"), ("EN", "中")] {
        ui.set_animations(false);
        ui.set_mode(initial.into());
        tick(0);
        ui.set_animations(true);
        ui.set_mode(other.into());
        tick(0);
        tick(30);
        ui.set_mode(initial.into());
        tick(0);
        tick(50);
        assert_eq!(ui.get_displayed_mode(), initial);
        tick(80);
        assert_eq!(ui.get_flip_phase(), Idle);
        ui.set_mode(other.into());
        tick(0);
        tick(80);
        assert_eq!(ui.get_displayed_mode(), other);
        ui.set_mode(initial.into());
        tick(0);
        tick(80);
        tick(1);
        tick(80);
        tick(80);
        assert_eq!(ui.get_flip_phase(), Idle);
        assert_eq!(ui.get_displayed_mode(), ui.get_target_mode());
        assert_eq!(ui.get_displayed_mode(), initial);
    }
    ui.set_mode("中".into());
    ui.set_animations(false);
    tick(0);
    assert_eq!(ui.get_flip_phase(), Idle);
    assert_eq!(ui.get_displayed_mode(), "中");
    assert_eq!(ui.get_flip_width(), 48.);
    ui.set_animations(true);
    ui.set_mode("EN".into());
    tick(0);
    tick(30);
    ui.set_reveal(false);
    ui.set_mode("中".into());
    tick(0);
    ui.set_reveal(true);
    tick(0);
    assert_eq!(ui.get_displayed_mode(), "中");
    assert_eq!(ui.get_flip_phase(), Idle);
    assert_eq!(ui.get_flip_width(), 48.);
    ui.hide().unwrap();
}

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> std::time::Duration {
        self.1.get()
    }
}

#[test]
fn switch_label_track_keyboard_and_disabled_state() {
    slint::platform::set_platform(Box::new(TestPlatform(
        MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer),
        Rc::default(),
    )))
    .unwrap();
    let ui = SwitchFixture::new().unwrap();
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.show().unwrap();
    let click = |x| {
        let position = LogicalPosition::new(x, 28.);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    };
    let space = || {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    };
    click(40.);
    assert!(ui.get_checked(), "label click must toggle and focus");
    space();
    assert!(!ui.get_checked(), "focused switch must support Space");
    click(215.);
    assert!(ui.get_checked(), "track click must toggle once");
    ui.set_enabled(false);
    click(40.);
    click(215.);
    space();
    assert!(ui.get_checked(), "disabled switch must reject all input");
    ui.set_enabled(true);
    for expected in [false, true, false, true] {
        click(215.);
        assert_eq!(ui.get_checked(), expected, "rapid toggles must not be lost");
    }
    ui.hide().unwrap();
}

#[test]
fn settings_render_in_both_languages_themes_and_narrow_widths() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let now = Rc::new(std::cell::Cell::new(std::time::Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), now.clone()))).unwrap();
    let ui = crate::AppWindow::new().unwrap();
    ui.global::<crate::SelectEnvironment>()
        .on_filter(crate::select::filter);
    ui.set_route("settings".into());
    ui.set_global_hotkey("Ctrl+Alt+J".into());
    ui.set_global_hotkey_enabled(true);
    ui.set_settings_valid(true);
    crate::style::apply_style!(
        ui.global::<crate::DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(960, 720));
    let position = LogicalPosition::new(110., 220.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
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
    };
    let saves = Rc::new(std::cell::Cell::new(0));
    let recorded = saves.clone();
    ui.on_save_settings(move || recorded.set(recorded.get() + 1));
    ui.set_global_hotkey_enabled(false);
    click(840., 211.);
    assert_eq!(ui.get_global_hotkey(), "Alt+V");
    assert!(ui.get_global_hotkey_enabled());
    assert_eq!(saves.get(), 0, "Reset must edit the draft without saving");
    ui.set_global_hotkey("Ctrl+Alt+J".into());
    ui.set_busy(true);
    click(840., 211.);
    assert_eq!(ui.get_global_hotkey(), "Ctrl+Alt+J");
    ui.set_busy(false);
    let retries = Rc::new(std::cell::Cell::new(0));
    let recorded = retries.clone();
    ui.on_settings_action(move |action| {
        if action == "retry-hotkey" {
            recorded.set(recorded.get() + 1);
        }
    });
    click(350., 262.);
    assert_eq!(retries.get(), 0, "healthy binding must not expose Retry");
    ui.set_hotkey_registration_failed(true);
    click(350., 262.);
    assert_eq!(retries.get(), 1, "failed saved binding must expose Retry");
    ui.set_settings_dirty(true);
    click(350., 262.);
    assert_eq!(retries.get(), 1, "Retry must not apply an unsaved draft");
    ui.set_settings_dirty(false);
    ui.set_busy(true);
    click(350., 262.);
    assert_eq!(retries.get(), 1, "busy settings must reject Retry");
    ui.set_busy(false);
    ui.set_hotkey_registration_failed(false);
    let dismissals = Rc::new(std::cell::Cell::new(0));
    let recorded_dismissals = dismissals.clone();
    ui.on_dismiss(move || recorded_dismissals.set(recorded_dismissals.get() + 1));
    for language in ["en", "zh-CN"] {
        slint::select_bundled_translation(language).unwrap();
        for dark in [false, true] {
            ui.set_dark(dark);
            crate::style::apply_style!(
                ui.global::<crate::DesignTokens>(),
                &crate::style::StyleSnapshot::default(),
                dark
            );
            for width in [960, 320] {
                for scale in [1., 1.5, 2.] {
                    ui.window().dispatch_event(WindowEvent::ScaleFactorChanged {
                        scale_factor: scale,
                    });
                    let physical_width = (width as f32 * scale) as u32;
                    let physical_height = (720. * scale) as u32;
                    window.set_size(slint::PhysicalSize::new(physical_width, physical_height));
                    let mut pixels = vec![
                        slint::Rgb8Pixel::default();
                        (physical_width * physical_height) as usize
                    ];
                    window.request_redraw();
                    window.draw_if_needed(|renderer| {
                        renderer.render(&mut pixels, physical_width as usize);
                    });
                    now.set(now.get() + std::time::Duration::from_millis(300));
                    slint::platform::update_timers_and_animations();
                    window.request_redraw();
                    window.draw_if_needed(|renderer| {
                        renderer.render(&mut pixels, physical_width as usize);
                    });
                    assert!(pixels.windows(2).any(|p| p[0] != p[1]));
                    let row_y = if width == 320 { 251. } else { 211. };
                    ui.set_global_hotkey("Ctrl+Alt+J".into());
                    ui.set_global_hotkey_enabled(false);
                    click(width as f32 - 120., row_y);
                    assert_eq!(ui.get_global_hotkey(), "Alt+V", "Reset at {width}/{scale}");
                    assert!(ui.get_global_hotkey_enabled());
                    click(width as f32 - 74., row_y);
                    assert!(!ui.get_global_hotkey_enabled(), "Switch at {width}/{scale}");
                    assert_eq!(saves.get(), 0);
                    ui.set_global_hotkey_enabled(true);
                    let before = dismissals.get();
                    click(width as f32 - 68., 55.);
                    assert_eq!(
                        dismissals.get(),
                        before + 1,
                        "Close must request hide at {width}/{scale}"
                    );
                    assert_eq!(
                        ui.get_route(),
                        "settings",
                        "Close must not navigate to History"
                    );
                    if let Some(directory) = std::env::var_os("ECHO_SETTINGS_RENDER_EVIDENCE") {
                        let directory = std::path::PathBuf::from(directory);
                        std::fs::create_dir_all(&directory).unwrap();
                        let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
                        image::save_buffer(
                            directory
                                .join(format!("settings-{language}-{dark}-{width}-{scale}.png")),
                            &bytes,
                            physical_width,
                            physical_height,
                            image::ColorType::Rgb8,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
    ui.hide().unwrap();
}

#[test]
fn input_focus_and_selection_follow_monochrome_and_high_contrast_theme() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), Rc::default()))).unwrap();
    let ui = InputAccentFixture::new().unwrap();
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(280, 160));
    for (dark, high_contrast, expected) in [
        (false, false, [32, 32, 32]),
        (true, false, [238, 238, 238]),
        (true, true, [255, 255, 0]),
        (false, false, [32, 32, 32]),
    ] {
        ui.set_dark(dark);
        crate::style::apply_style!(
            ui.global::<DesignTokens>(),
            &crate::style::StyleSnapshot::default(),
            dark
        );
        ui.global::<EchoTheme>().set_high_contrast(high_contrast);
        for multiline in [false, true] {
            ui.invoke_select_input(multiline);
            let mut pixels = vec![slint::Rgb8Pixel::default(); 280 * 160];
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 280);
            });
            let (start, end) = if multiline { (60, 150) } else { (10, 46) };
            let count = pixels[start * 280..end * 280]
                .iter()
                .filter(|p| [p.r, p.g, p.b] == expected)
                .count();
            assert!(count > 500, "selected text and focus underline must use theme accent: {dark}/{high_contrast}/{multiline}: {count}");
        }
    }
    ui.hide().unwrap();
}

#[test]
fn icon_grid_click_keyboard_scroll_and_close() {
    use slint::Model;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone(), Rc::default()))).unwrap();
    let ui = IconGridFixture::new().unwrap();
    ui.global::<FavoriteIconImages>()
        .on_index_for(|key| crate::favorite_icons::index(key.as_str()));
    let source = crate::favorite_icons::choices();
    let last = source.row_data(source.row_count() - 1).unwrap().key;
    ui.set_choices(
        Rc::new(slint::VecModel::from(
            source
                .iter()
                .map(|choice| FavoriteIconChoice {
                    key: choice.key,
                    label: choice.label,
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(500, 540));
    let render = || {
        let mut pixels = vec![slint::Rgb8Pixel::default(); 500 * 540];
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut pixels, 500);
        });
        pixels
    };
    let pixels = render();
    if let Some(path) = std::env::var_os("ECHO_ICON_GRID_RENDER") {
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        image::save_buffer(path, &bytes, 500, 540, image::ColorType::Rgb8).unwrap();
    }
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
    };
    click(104., 112.);
    assert_eq!(ui.get_chosen_key(), "Mail");
    let key = |text: slint::SharedString| {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    };
    for _ in 0..100 {
        key(slint::platform::Key::DownArrow.into());
    }
    render();
    key(slint::platform::Key::Return.into());
    assert_eq!(
        ui.get_chosen_key(),
        last,
        "keyboard must reach the end of the full catalog"
    );
    click(446., 64.);
    assert!(ui.get_dismissed(), "top-right close icon must dismiss");
    ui.hide().unwrap();
}
