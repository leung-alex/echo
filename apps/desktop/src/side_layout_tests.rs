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
    import { SpaceSideCard } from "../ui/software-cards.slint";
    export component SideLayoutFixture inherits Window {
        width: 500px; height: 300px;
        in property <length> frame-width: 300px;
        SpaceSideCard {
            x: 0px; y: 0px;
            width: root.frame-width; height: 280px;
            layout-width: 300px; layout-height: 280px;
            space: {title: "A long title that wraps into multiple lines", count: "2"};
            preview: {rows: [{title: "Example", body: "Stable line wrapping must survive a changing card frame. More text makes the second line observable."}]};
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
fn side_text_pixels_stay_stable_while_only_the_frame_resizes() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = SideLayoutFixture::new().unwrap();
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(500, 300));
    let capture = || {
        let mut pixels = vec![Rgb8Pixel::default(); 500 * 300];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 500);
        });
        // Stay inside the narrowest frame, away from its border and rounded corners.
        (25..220)
            .flat_map(|y| (30..130).map(move |x| y * 500 + x))
            .map(|i| (pixels[i].r, pixels[i].g, pixels[i].b))
            .collect::<Vec<_>>()
    };
    let settled = capture();
    assert!(
        settled.windows(2).any(|pair| pair[0] != pair[1]),
        "fixture must contain visible text"
    );
    for width in [160., 220., 400., 300.] {
        ui.set_frame_width(width);
        assert!(capture() == settled, "text reflowed at frame width {width}");
    }
}
