use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, WindowAdapter,
    },
    ComponentHandle, Rgb8Pixel,
};
use std::rc::Rc;

slint::slint! {
    export { DesignTokens } from "../ui/echo-tokens.slint";
    import { SearchText, MatchRange } from "../ui/search-text.slint";
    import { EntryView } from "../ui/entry-row.slint";
    export component SearchFixture inherits Window {
        width: 360px; height: 150px; background: white;
        in property <bool> marked: true;
        private property <[MatchRange]> marked-ranges: [{start:37,end:40}, {start:47,end:53}];
        SearchText {
            x: 16px; y: 16px; width: 240px; height: 110px;
            text: "https://www.bilibili.com/video/\ncard.all.click 中文匹配";
            foreground: #626a70; font-size: 16px;
            matches: root.marked ? root.marked-ranges : [];
        }
    }
    export component SearchRowFixture inherits Window {
        width: 500px; height: 200px; background: #f6f7f8;
        in property <bool> selected: true;
        out property <length> row-height: row.height;
        row := EntryView {
            x: 8px; y: 8px; width: 480px;
            entry: { key: "preview", body: "https://www.bilibili.com/video/\ncard.all.click 中文匹配",
                body-matches: [{start:37,end:40}, {start:47,end:53}], match-count: 2,
                selected: root.selected, time-label: "21:46" };
        }
    }
}
struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}
#[test]
fn search_marks_paint_background_without_moving_neighboring_text() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = SearchFixture::new().unwrap();
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(360, 150));
    let capture = || {
        let mut pixels = vec![Rgb8Pixel::default(); 360 * 150];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 360);
        });
        pixels
    };
    let marked = capture();
    assert!(
        marked
            .iter()
            .filter(|p| (p.r, p.g, p.b) == (255, 229, 154))
            .count()
            > 100,
        "yellow background is missing"
    );
    ui.set_marked(false);
    let plain = capture();
    // The first line has no matches, and must not move/reflow when marks change.
    assert_eq!(&marked[..360 * 35], &plain[..360 * 35]);
    assert!(!plain.iter().any(|p| (p.r, p.g, p.b) == (255, 229, 154)));
    if let Ok(path) = std::env::var("ECHO_SEARCH_PREVIEW") {
        let bytes = marked
            .iter()
            .flat_map(|p| [p.r, p.g, p.b])
            .collect::<Vec<_>>();
        image::save_buffer(path, &bytes, 360, 150, image::ColorType::Rgb8).unwrap();
    }
    ui.hide().unwrap();
    let row = SearchRowFixture::new().unwrap();
    crate::style::apply_style!(
        row.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    row.show().unwrap();
    window.set_size(slint::PhysicalSize::new(500, 200));
    for selected in [true, false] {
        row.set_selected(selected);
        let mut pixels = vec![Rgb8Pixel::default(); 500 * 200];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 500);
        });
        assert!(
            row.get_row_height() >= 50. && row.get_row_height() < 150.,
            "unexpected row height {}",
            row.get_row_height()
        );
        assert!(
            pixels
                .iter()
                .filter(|p| (p.r, p.g, p.b) == (255, 229, 154))
                .count()
                > 100,
            "selected={selected}"
        );
        if selected {
            if let Ok(path) = std::env::var("ECHO_SEARCH_PREVIEW") {
                let path = std::path::Path::new(&path).with_file_name("search-row-preview.png");
                let bytes = pixels
                    .iter()
                    .flat_map(|p| [p.r, p.g, p.b])
                    .collect::<Vec<_>>();
                image::save_buffer(path, &bytes, 500, 200, image::ColorType::Rgb8).unwrap();
            }
        }
    }
}
