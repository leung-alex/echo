//! One persistent Slint scene renders directly to a GPU texture on the shared device.
//! No native window, focus changes, UI Automation mirror, CPU pixels, map, poll, or readback.
use crate::{AppWindow, CardSnapshot};
use slint::platform::{
    femtovg_renderer::FemtoVGWGPURenderer, Renderer, WindowAdapter, WindowEvent,
};
use slint::wgpu_29::wgpu;
use slint::{ComponentHandle, PhysicalSize, Window, WindowSize};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};
struct Adapter {
    window: Window,
    renderer: FemtoVGWGPURenderer,
    size: Cell<PhysicalSize>,
}
impl WindowAdapter for Adapter {
    fn window(&self) -> &Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
    fn set_size(&self, size: WindowSize) {
        self.size.set(size.to_physical(self.window.scale_factor()));
        self.window.dispatch_event(WindowEvent::Resized {
            size: size.to_logical(self.window.scale_factor()),
        });
    }
}
pub struct PanelRenderer {
    component: CardSnapshot,
    adapter: Rc<Adapter>,
}
impl PanelRenderer {
    pub fn new(
        instance: wgpu::Instance,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, String> {
        let renderer =
            FemtoVGWGPURenderer::new(instance, device, queue).map_err(|e| e.to_string())?;
        let adapter = Rc::new_cyclic(|weak: &Weak<Adapter>| Adapter {
            window: Window::new(weak.clone()),
            renderer,
            size: Cell::new(PhysicalSize::new(1, 1)),
        });
        let component =
            i_slint_backend_winit::echo_offscreen::with_adapter(adapter.clone(), CardSnapshot::new)
                .map_err(|e| e.to_string())?;
        component
            .global::<crate::FavoriteIconImages>()
            .on_index_for(|key| crate::favorite_icons::index(key.as_str()));
        Ok(Self { component, adapter })
    }
    pub fn clear(&self) {
        self.component.set_rows(slint::ModelRc::default());
    }
    pub fn render(
        &self,
        source: &AppWindow,
        texture: &wgpu::Texture,
        dpi: f32,
    ) -> Result<(), String> {
        let c = &self.component;
        self.adapter
            .window
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: dpi });
        self.adapter.window.set_size(slint::LogicalSize::new(
            source.get_panel_width(),
            source.get_panel_height(),
        ));
        c.set_dark(source.get_dark());
        c.set_panel_title(source.get_capture_title());
        c.set_subtitle(source.get_capture_subtitle());
        c.set_icon_key(source.get_capture_icon());
        c.set_accent(source.get_capture_accent());
        c.set_favorites(source.get_capture_favorites());
        c.set_loading(source.get_capture_loading());
        c.set_titles_only(source.get_capture_titles_only());
        c.set_compact(source.get_density().as_str() == "compact");
        c.set_rows(source.get_capture_rows());
        c.set_scroll_y(source.get_capture_scroll());
        c.set_query(if source.get_capture_titles_only() {
            "".into()
        } else {
            source.get_capture_query()
        });
        c.set_navigation_label(source.get_capture_navigation_label());
        c.set_navigation_hint(source.get_capture_navigation_hint());
        c.set_search_focused(source.get_capture_search_focused());
        c.set_previous_enabled(source.get_capture_previous_enabled());
        c.set_next_enabled(source.get_capture_next_enabled());
        c.set_has_more(source.get_capture_has_more());
        c.set_has_previous(source.get_capture_has_previous());
        c.set_batch(source.get_capture_batch());
        c.set_selected_count(source.get_capture_selected_count());
        c.set_quick_insert(source.get_capture_quick_insert());
        c.set_inline_mode(source.get_inline_mode());
        let result = self
            .adapter
            .renderer
            .render_to_texture(texture)
            .map_err(|e| e.to_string());
        // Reuse the bounded last model. Clearing it on every capture rebuilt the entire item tree.
        result
    }
}
