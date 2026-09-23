use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, PointerEventButton, WindowAdapter, WindowEvent,
    },
    ComponentHandle, Image, LogicalPosition, Rgb8Pixel, SharedPixelBuffer,
};
use std::rc::Rc;

slint::slint! {
    export { DesignTokens } from "../ui/echo-tokens.slint";
    export { LucideIconImages } from "../ui/lucide-icon-images.slint";
    import { SpaceSideCard } from "../ui/software-cards.slint";
    import { EntryRow, EntryView } from "../ui/entry-row.slint";
    export component SideLayoutFixture inherits Window {
        width: 500px; height: 300px;
        in property <length> frame-width: 300px;
        callback chosen();
        SpaceSideCard {
            x: 0px; y: 0px;
            width: root.frame-width; height: 280px;
            layout-width: 300px; layout-height: 280px;
            space: {key: "favorites", title: "A long title that wraps into multiple lines", count: "2", system: true, icon-key: "Star"};
            preview: {rows: [{key: "preview", title: "Example", body: "Stable line wrapping must survive a changing card frame. More text makes the second line observable.", title-rich: @markdown("Example"), body-rich: @markdown("Stable line wrapping must survive a changing card frame. More text makes the second line observable."), title-matches: [], body-matches: []}], favorites: true};
            chosen => { root.chosen(); }
        }
    }
    export component SideParityFixture inherits Window {
        width: 650px; height: 360px;
        in property <bool> favorites: true;
        in property <image> sample-thumbnail;
        private property <EntryRow> sample: {
            key: "parity", title: "Title", body: "Body text that uses the shared row layout",
            title-rich: @markdown("Title"), body-rich: @markdown("Body text that uses the shared row layout"),
            icon-key: "lucide-star", time-label: "12:34", pinned: true, tags: "tag",
            has-thumbnail: root.sample-thumbnail.width > 0, thumbnail: root.sample-thumbnail
        };
        SpaceSideCard {
            x: 0px; y: 0px; width: 300px; height: 340px;
            layout-width: 300px; layout-height: 340px;
            space: {key: "side", title: "Side", count: "1", system: false, icon-key: ""};
            preview: {rows: [root.sample], favorites: root.favorites};
        }
        EntryView {
            x: 316px; y: 96px; width: 268px;
            entry: root.sample; favorites: root.favorites; capture-only: true;
            thumbnail-active: false; batch: false; busy: false; input-blocked: false;
            inline-mode: false; quick-insert: false; copy-only: false; reorder-enabled: false;
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

#[test]
fn side_entry_remains_read_only_and_routes_click_to_the_space_card() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = SideLayoutFixture::new().unwrap();
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    let chosen = Rc::new(std::cell::Cell::new(0));
    let chosen_callback = chosen.clone();
    ui.on_chosen(move || chosen_callback.set(chosen_callback.get() + 1));
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(500, 300));
    let position = LogicalPosition::new(80., 130.);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(
        chosen.get(),
        1,
        "side-card click should choose the space once"
    );
    ui.hide().unwrap();
}

#[test]
fn side_entry_pixels_match_the_same_read_only_main_entry() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = SideParityFixture::new().unwrap();
    ui.global::<LucideIconImages>()
        .on_image_for(|key| crate::lucide_icons::image(key.as_str()));
    crate::style::apply_style!(
        ui.global::<DesignTokens>(),
        &crate::style::StyleSnapshot::default(),
        false
    );
    let mut thumbnail = SharedPixelBuffer::new(12, 8);
    thumbnail.make_mut_bytes().fill(255);
    ui.set_sample_thumbnail(Image::from_rgba8(thumbnail));
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(650, 360));
    for favorites in [true, false] {
        ui.set_favorites(favorites);
        let mut pixels = vec![Rgb8Pixel::default(); 650 * 360];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 650);
        });
        for y in 108..168 {
            for x in 28..272 {
                assert_eq!(
                    pixels[y * 650 + x],
                    pixels[y * 650 + x + 300],
                    "side/main row pixel differs at ({x},{y}), favorites={favorites}"
                );
            }
        }
    }
    ui.hide().unwrap();
}
