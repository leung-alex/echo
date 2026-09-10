//! Bounded perspective-correct textured quads, sharing Slint's device.
use echo_presentation::echo_tokens as t;
use slint::wgpu_29::wgpu;
use std::collections::HashMap;

pub const HARD_BUDGET: u64 = t::BUDGET_TEXTURE_HARD_MIB as u64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelDraw {
    pub id: i64,
    pub shadow_opacity: f32,
    /// Screen-space camera translation, applied after perspective division.
    pub origin_x: f32,
    pub origin_y: f32,
    pub width: f32,
    pub height: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub scale: f32,
    pub opacity: f32,
    pub shade: f32,
}
struct Panel {
    _texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    binding: wgpu::BindGroup,
    bytes: u64,
}
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    panels: HashMap<i64, Panel>,
    output: Option<wgpu::Texture>,
    output_view: Option<wgpu::TextureView>,
    output_image: Option<slint::Image>,
    output_size: (u32, u32),
    pub frames: u64,
    pub uploads: u64,
    pub reflections: bool,
}
impl Compositor {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("echo-cover-flow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cover_flow.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("echo-perspective-panels"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("echo-panel-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            device,
            queue,
            pipeline,
            sampler,
            panels: HashMap::new(),
            output: None,
            output_view: None,
            output_image: None,
            output_size: (0, 0),
            frames: 0,
            uploads: 0,
            reflections: false,
        }
    }
    pub fn bytes(&self) -> u64 {
        self.panels.values().map(|p| p.bytes).sum::<u64>()
            + u64::from(self.output_size.0) * u64::from(self.output_size.1) * 4
    }
    pub fn max_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }
    #[cfg(feature = "native-test")]
    pub fn panel_dimensions(&self) -> Vec<[u32; 2]> {
        let mut sizes = self
            .panels
            .values()
            .map(|p| [p._texture.width(), p._texture.height()])
            .collect::<Vec<_>>();
        sizes.sort();
        sizes
    }
    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }
    pub fn contains(&self, id: i64) -> bool {
        self.panels.contains_key(&id)
    }
    pub fn retain(&mut self, ids: &[i64]) {
        self.panels.retain(|id, _| ids.contains(id));
    }
    pub fn clear(&mut self) {
        self.panels.clear();
        self.output_image = None;
        self.output_view = None;
        self.output = None;
        self.output_size = (0, 0);
    }
    /// Allocate a renderable cached panel, reusing dimensions. No CPU pixel allocation.
    pub fn panel_target(
        &mut self,
        id: i64,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, String> {
        if width == 0
            || height == 0
            || width > self.device.limits().max_texture_dimension_2d
            || height > self.device.limits().max_texture_dimension_2d
        {
            return Err("Invalid panel dimensions".into());
        }
        if let Some(p) = self.panels.get(&id) {
            if p._texture.width() == width && p._texture.height() == height {
                return Ok(p._texture.clone());
            }
        }
        let bytes = u64::from(width) * u64::from(height) * 4;
        let old = self.panels.get(&id).map_or(0, |p| p.bytes);
        if self.bytes().saturating_sub(old) + bytes > HARD_BUDGET
            || (!self.panels.contains_key(&id) && self.panels.len() >= 4)
        {
            return Err("Cover Flow texture budget exceeded".into());
        }
        let _timing = crate::popup_timing::span("panel_allocation");
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("echo-panel-render-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("echo-panel-pose"),
            size: 112,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("echo-panel-binding"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.panels.insert(
            id,
            Panel {
                _texture: texture.clone(),
                uniform,
                binding,
                bytes,
            },
        );
        Ok(texture)
    }
    /// Test/probe pixel upload; production panels use panel_target and GPU-only rendering.
    pub fn upload(&mut self, id: i64, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
        if u64::from(width) * u64::from(height) * 4 != rgba.len() as u64 {
            return Err("Invalid pixel buffer".into());
        }
        let texture = self.panel_target(id, width, height)?;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            texture.size(),
        );
        self.uploads += 1;
        Ok(())
    }
    pub fn resize(&mut self, width: u32, height: u32) -> Result<slint::Image, String> {
        let bytes = u64::from(width) * u64::from(height) * 4;
        let previous = u64::from(self.output_size.0) * u64::from(self.output_size.1) * 4;
        if width == 0
            || height == 0
            || width > self.device.limits().max_texture_dimension_2d
            || height > self.device.limits().max_texture_dimension_2d
            || self.bytes().saturating_sub(previous) + bytes > HARD_BUDGET
        {
            return Err("Cover Flow output exceeds texture budget".into());
        }
        if self.output_size != (width, height) {
            let _timing = crate::popup_timing::span("output_allocation");
            self.output = Some(self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("echo-cover-flow-output"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }));
            self.output_view = Some(
                self.output
                    .as_ref()
                    .unwrap()
                    .create_view(&Default::default()),
            );
            self.output_image = Some(
                slint::Image::try_from(self.output.as_ref().unwrap().clone())
                    .map_err(|e| e.to_string())?,
            );
            self.output_size = (width, height);
        }
        self.image()
    }
    pub fn image(&self) -> Result<slint::Image, String> {
        self.output_image
            .clone()
            .ok_or_else(|| "Output is not ready".into())
    }
    pub fn draw(
        &mut self,
        width: f32,
        height: f32,
        dpi: f32,
        panels: &[PanelDraw],
    ) -> Result<(), String> {
        if panels.len() > 4 || width <= 0.0 || height <= 0.0 || dpi <= 0.0 {
            return Err("Invalid Cover Flow frame".into());
        }
        let view = self.output_view.as_ref().ok_or("Output is not ready")?;
        let mut ordered = panels.to_vec();
        ordered.sort_by(|a, b| a.z.total_cmp(&b.z).then(a.id.cmp(&b.id)));
        for pose in &ordered {
            if let Some(panel) = self.panels.get(&pose.id) {
                let params = [
                    width,
                    height,
                    pose.width * t::FLOW_PERSPECTIVE_RATIO,
                    t::PANEL_RADIUS,
                    pose.width,
                    pose.height,
                    pose.scale,
                    pose.opacity,
                    pose.x,
                    pose.y,
                    pose.z,
                    pose.yaw,
                    pose.shade,
                    1.0 / dpi,
                    if self.reflections {
                        t::FLOW_REFLECTION_OPACITY
                    } else {
                        0.0
                    },
                    t::FLOW_REFLECTION_HEIGHT_RATIO,
                    t::FLOW_SHADOW_SOFTNESS,
                    t::FLOW_SHADOW_MARGIN,
                    t::FLOW_SHADOW_OFFSET_Y,
                    t::FLOW_SHADOW_OPACITY * pose.shadow_opacity,
                    t::FLOW_REFLECTION_GAP,
                    t::FLOW_EDGE_PADDING_PIXELS,
                    t::FLOW_EDGE_AA_PIXELS,
                    pose.origin_x,
                    pose.origin_y,
                    0.0,
                    0.0,
                    0.0,
                ];
                let mut bytes = [0u8; 112];
                for (slot, value) in bytes.chunks_exact_mut(4).zip(params) {
                    slot.copy_from_slice(&value.to_le_bytes());
                }
                self.queue.write_buffer(&panel.uniform, 0, &bytes);
            }
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("echo-cover-flow-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("echo-perspective-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            for pose in &ordered {
                if let Some(panel) = self.panels.get(&pose.id) {
                    pass.set_bind_group(0, &panel.binding, &[]);
                    pass.draw(0..6, 1..2);
                    if self.reflections {
                        pass.draw(0..6, 2..3);
                    }
                    pass.draw(0..6, 0..1);
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        self.frames += 1;
        Ok(())
    }
}
