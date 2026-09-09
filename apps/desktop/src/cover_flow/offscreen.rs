//! One persistent Slint scene renders directly to a GPU texture on the shared device.
//! No native window, focus changes, UI Automation mirror, CPU pixels, map, poll, or readback.
use crate::{CardSnapshot, EntryRow};
use slint::platform::{
    femtovg_renderer::FemtoVGWGPURenderer, Renderer, WindowAdapter, WindowEvent,
};
use slint::wgpu_29::wgpu;
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel, Window, WindowSize};
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
    rows: Rc<VecModel<EntryRow>>,
    geometry: Cell<Option<(u32, u32, u32)>>,
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
        let rows = Rc::new(VecModel::default());
        component.set_rows(ModelRc::from(rows.clone()));
        Ok(Self {
            component,
            adapter,
            rows,
            geometry: Cell::new(None),
        })
    }
    pub fn clear(&self) {
        self.rows.set_vec(Vec::new());
    }
    pub fn render(
        &self,
        snapshot: &super::snapshot::PanelSnapshot,
        texture: &wgpu::Texture,
    ) -> Result<(), String> {
        let c = &self.component;
        let geometry = (
            snapshot.width.to_bits(),
            snapshot.height.to_bits(),
            snapshot.dpi.to_bits(),
        );
        if self.geometry.get().map(|old| old.2) != Some(geometry.2) {
            self.adapter
                .window
                .dispatch_event(WindowEvent::ScaleFactorChanged {
                    scale_factor: snapshot.dpi,
                });
        }
        if self.geometry.replace(Some(geometry)) != Some(geometry) {
            self.adapter
                .window
                .set_size(slint::LogicalSize::new(snapshot.width, snapshot.height));
        }
        {
            let _timing = crate::popup_timing::span("offscreen_model_update");
            snapshot.apply(c);
            crate::native_model::reconcile_keyed_by(
                self.rows.as_ref(),
                snapshot.rows.clone(),
                |row: &EntryRow| row.key.clone(),
                crate::native_model::entry_row_equal,
            );
        }
        let _timing = crate::popup_timing::span("offscreen_render");
        let result = self
            .adapter
            .renderer
            .render_to_texture(texture)
            .map_err(|e| e.to_string());
        // Keep the same model identity until explicit hidden-memory reclamation.
        result
    }
}
