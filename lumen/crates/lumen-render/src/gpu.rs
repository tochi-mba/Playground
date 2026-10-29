//! wgpu renderer.
//!
//! Renders a [`Scene`] into any texture view: an output window's swapchain in the app, or an
//! offscreen texture that is read back for the CLI and the tests. The pipeline is the same
//! two passes as the software renderer: composite every surface with premultiplied alpha, then
//! post-process (colour, edge blend, master, blackout).

use crate::scene::{Post, Scene, SurfaceDraw};
use image::RgbaImage;
use lumen_core::blend::BlendCurve;
use lumen_core::project::BlendMode;
use lumen_sources::Frame;
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Errors from GPU setup and readback.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter satisfies the request (no GPU and no software fallback installed).
    #[error("no compatible GPU adapter: {0}")]
    NoAdapter(String),
    /// The adapter refused to create a device.
    #[error("cannot create GPU device: {0}")]
    Device(String),
    /// Reading the rendered image back failed.
    #[error("GPU readback failed: {0}")]
    Readback(String),
}

/// Texture format of the composite buffer.
pub const COMPOSITE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Instance, adapter, device and queue.
pub struct GpuContext {
    /// wgpu instance; the app uses it to create window surfaces.
    pub instance: wgpu::Instance,
    /// Chosen adapter.
    pub adapter: wgpu::Adapter,
    /// Logical device.
    pub device: wgpu::Device,
    /// Command queue.
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// Creates a context with no surface, suitable for headless rendering. Honours the
    /// `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` environment variables like wgpu's own examples.
    pub fn headless() -> Result<Self, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        Self::with_instance(instance, None)
    }

    /// Creates a context on an existing instance, optionally compatible with a surface.
    pub fn with_instance(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, GpuError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|e| GpuError::NoAdapter(e.to_string()))?;
        log::info!("GPU adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lumen"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uvw: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfaceUniforms {
    crop: [f32; 4],
    opacity: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniforms {
    widths: [f32; 4],
    curves: [u32; 4],
    params: [f32; 4],
    params2: [f32; 4],
}

impl PostUniforms {
    fn from_post(post: &Post) -> Self {
        let b = post.blend.sanitised();
        let curve = |c: BlendCurve| match c {
            BlendCurve::Linear => 0,
            BlendCurve::Smooth => 1,
            BlendCurve::Cosine => 2,
        };
        Self {
            widths: [
                b.left.width as f32,
                b.right.width as f32,
                b.top.width as f32,
                b.bottom.width as f32,
            ],
            curves: [
                curve(b.left.curve),
                curve(b.right.curve),
                curve(b.top.curve),
                curve(b.bottom.curve),
            ],
            params: [
                b.gamma as f32,
                post.color.brightness as f32,
                post.color.contrast as f32,
                post.color.gamma as f32,
            ],
            params2: [
                post.master as f32,
                if post.blackout { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ],
        }
    }
}

struct CachedTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    frame: Arc<Frame>,
}

struct Composite {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    width: u32,
    height: u32,
}

/// Renders scenes with wgpu. One renderer serves every output window.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    sampler: wgpu::Sampler,
    surface_layout: wgpu::BindGroupLayout,
    surface_pipelines: HashMap<BlendMode, wgpu::RenderPipeline>,
    post_layout: wgpu::BindGroupLayout,
    post_module: wgpu::ShaderModule,
    post_pipeline_layout: wgpu::PipelineLayout,
    post_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    textures: HashMap<String, CachedTexture>,
    masks: HashMap<String, (wgpu::TextureView, Vec<u8>)>,
    composites: HashMap<String, Composite>,
}

impl GpuRenderer {
    /// Builds pipelines on a device.
    #[allow(clippy::too_many_lines)]
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lumen linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lumen surface"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                sampler_entry(2),
                texture_entry(3),
            ],
        });
        let surface_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lumen surface"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/surface.wgsl").into()),
        });
        let surface_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lumen surface"),
                bind_group_layouts: &[Some(&surface_layout)],
                ..Default::default()
            });
        let mut surface_pipelines = HashMap::new();
        for mode in [
            BlendMode::Normal,
            BlendMode::Add,
            BlendMode::Multiply,
            BlendMode::Screen,
        ] {
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lumen surface"),
                layout: Some(&surface_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &surface_module,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &surface_module,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COMPOSITE_FORMAT,
                        blend: Some(blend_state(mode)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            surface_pipelines.insert(mode, pipeline);
        }
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lumen post"),
            entries: &[uniform_entry(0), texture_entry(1), sampler_entry(2)],
        });
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lumen post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let post_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lumen post"),
            bind_group_layouts: &[Some(&post_layout)],
            ..Default::default()
        });
        Self {
            device,
            queue,
            sampler,
            surface_layout,
            surface_pipelines,
            post_layout,
            post_module,
            post_pipeline_layout,
            post_pipelines: HashMap::new(),
            textures: HashMap::new(),
            masks: HashMap::new(),
            composites: HashMap::new(),
        }
    }

    /// The device this renderer draws with.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Records the composite pass for `scene` into `encoder` and readies the post pass for a
    /// target in `format`. `key` names the composite buffer (one per output window or preview)
    /// so several targets can be in flight without re-allocating.
    ///
    /// Call [`GpuRenderer::paint`] with the same `key` inside a render pass on the target.
    pub fn prepare(
        &mut self,
        key: &str,
        scene: &Scene,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let (width, height) = (width.max(1), height.max(1));
        self.ensure_composite(key, width, height);
        self.ensure_post_pipeline(format);
        let draws: Vec<(wgpu::Buffer, wgpu::Buffer, u32, wgpu::BindGroup, BlendMode)> = scene
            .draws
            .iter()
            .filter_map(|d| self.prepare_draw(d))
            .collect();
        let Some(composite) = self.composites.get(key) else {
            return;
        };
        self.queue.write_buffer(
            &composite.uniforms,
            0,
            bytemuck::bytes_of(&PostUniforms::from_post(&scene.post)),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lumen composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &composite.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for (vb, ib, count, bind_group, mode) in &draws {
            pass.set_pipeline(&self.surface_pipelines[mode]);
            pass.set_bind_group(0, bind_group, &[]);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..*count, 0, 0..1);
        }
    }

    /// Draws the post-processed composite named `key` into `pass`, whose colour target must be
    /// in `format`. Does nothing if `key` was never prepared.
    pub fn paint(&self, key: &str, format: wgpu::TextureFormat, pass: &mut wgpu::RenderPass<'_>) {
        let (Some(composite), Some(pipeline)) =
            (self.composites.get(key), self.post_pipelines.get(&format))
        else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &composite.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Renders `scene` into `target`, a view of a `width × height` texture in `format` that is
    /// usable as a render attachment.
    pub fn render_to_view(
        &mut self,
        scene: &Scene,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lumen frame"),
            });
        let key = format!("view:{}", scene.output_id);
        self.prepare(&key, scene, width, height, format, &mut encoder);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lumen post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.paint(&key, format, &mut pass);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// Renders `scene` at `width × height` and reads the pixels back.
    pub fn render_to_image(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, GpuError> {
        let (width, height) = (width.max(1), height.max(1));
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lumen readback target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COMPOSITE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to_view(scene, &view, COMPOSITE_FORMAT, width, height);

        let bytes_per_row = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lumen readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lumen readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let data = slice
            .get_mapped_range()
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            pixels.extend_from_slice(&data[start..start + (width * 4) as usize]);
        }
        drop(data);
        buffer.unmap();
        RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| GpuError::Readback("size mismatch".into()))
    }

    /// Drops cached textures for sources no longer in use.
    pub fn retain_sources(&mut self, keep: impl Fn(&str) -> bool) {
        self.textures.retain(|id, _| keep(id));
    }

    /// Drops composite buffers and mask textures whose keys are no longer in use.
    pub fn retain_targets(&mut self, keep: impl Fn(&str) -> bool) {
        self.composites.retain(|k, _| keep(k));
    }

    fn ensure_composite(&mut self, key: &str, width: u32, height: u32) {
        if let Some(c) = self.composites.get(key) {
            if c.width == width && c.height == height {
                return;
            }
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lumen composite"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COMPOSITE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lumen post uniforms"),
            size: size_of::<PostUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lumen post"),
            layout: &self.post_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
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
        self.composites.insert(
            key.to_string(),
            Composite {
                view,
                bind_group,
                uniforms,
                width,
                height,
            },
        );
    }

    fn ensure_post_pipeline(&mut self, format: wgpu::TextureFormat) {
        if self.post_pipelines.contains_key(&format) {
            return;
        }
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lumen post"),
                layout: Some(&self.post_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.post_module,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.post_module,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.post_pipelines.insert(format, pipeline);
    }

    fn upload_rgba(
        &self,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        bytes_per_pixel: u32,
        data: &[u8],
    ) -> wgpu::Texture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * bytes_per_pixel),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    fn source_view(&mut self, draw: &SurfaceDraw) -> wgpu::TextureView {
        let fresh = self
            .textures
            .get(&draw.source_id)
            .is_some_and(|c| Arc::ptr_eq(&c.frame, &draw.frame));
        if !fresh {
            let f = &draw.frame;
            let texture = self.upload_rgba(
                "lumen source",
                f.width,
                f.height,
                wgpu::TextureFormat::Rgba8Unorm,
                4,
                &f.rgba,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.textures.insert(
                draw.source_id.clone(),
                CachedTexture {
                    texture,
                    view,
                    frame: Arc::clone(f),
                },
            );
        }
        let cached = &self.textures[&draw.source_id];
        let _ = &cached.texture;
        cached.view.clone()
    }

    fn mask_view(&mut self, draw: &SurfaceDraw) -> wgpu::TextureView {
        let fresh = self
            .masks
            .get(&draw.id)
            .is_some_and(|(_, data)| *data == draw.mask.data);
        if !fresh {
            let m = &draw.mask;
            let texture = self.upload_rgba(
                "lumen mask",
                m.size,
                m.size,
                wgpu::TextureFormat::R8Unorm,
                1,
                &m.data,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.masks.insert(draw.id.clone(), (view, m.data.clone()));
        }
        self.masks[&draw.id].0.clone()
    }

    fn prepare_draw(
        &mut self,
        draw: &SurfaceDraw,
    ) -> Option<(wgpu::Buffer, wgpu::Buffer, u32, wgpu::BindGroup, BlendMode)> {
        if draw.mesh.indices.is_empty() || !draw.mesh.is_well_formed() {
            return None;
        }
        let vertices: Vec<Vertex> = draw
            .mesh
            .positions
            .iter()
            .zip(&draw.mesh.texcoords)
            .map(|(p, t)| Vertex {
                pos: [p.x as f32, p.y as f32],
                uvw: [t[0] as f32, t[1] as f32, t[2] as f32],
            })
            .collect();
        let vb = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lumen vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let ib = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lumen indices"),
                contents: bytemuck::cast_slice(&draw.mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let uniforms = SurfaceUniforms {
            crop: [
                draw.crop.x as f32,
                draw.crop.y as f32,
                draw.crop.w as f32,
                draw.crop.h as f32,
            ],
            opacity: draw.opacity as f32,
            _pad: [0.0; 3],
        };
        let ub = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lumen surface uniforms"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let src_view = self.source_view(draw);
        let mask_view = self.mask_view(draw);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lumen surface"),
            layout: &self.surface_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ub.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&mask_view),
                },
            ],
        });
        Some((
            vb,
            ib,
            draw.mesh.indices.len() as u32,
            bind_group,
            draw.blend_mode,
        ))
    }
}

/// Blend state that reproduces [`crate::software::blend_pixel`] for premultiplied sources.
fn blend_state(mode: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent, BlendFactor, BlendOperation};
    let comp = |src, dst| BlendComponent {
        src_factor: src,
        dst_factor: dst,
        operation: BlendOperation::Add,
    };
    let alpha = comp(BlendFactor::One, BlendFactor::OneMinusSrcAlpha);
    match mode {
        BlendMode::Normal => wgpu::BlendState {
            color: comp(BlendFactor::One, BlendFactor::OneMinusSrcAlpha),
            alpha,
        },
        BlendMode::Add => wgpu::BlendState {
            color: comp(BlendFactor::One, BlendFactor::One),
            alpha,
        },
        BlendMode::Multiply => wgpu::BlendState {
            color: comp(BlendFactor::Dst, BlendFactor::OneMinusSrcAlpha),
            alpha,
        },
        BlendMode::Screen => wgpu::BlendState {
            color: comp(BlendFactor::One, BlendFactor::OneMinusSrc),
            alpha,
        },
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn post_uniforms_mirror_post() {
        let mut post = Post {
            blend: lumen_core::blend::EdgeBlend::none(),
            color: lumen_core::project::ColorAdjust {
                brightness: 1.5,
                contrast: 0.5,
                gamma: 2.0,
            },
            master: 0.7,
            blackout: true,
        };
        post.blend.right.width = 0.3;
        post.blend.right.curve = BlendCurve::Cosine;
        post.blend.top.curve = BlendCurve::Linear;
        post.blend.gamma = 99.0;
        let u = PostUniforms::from_post(&post);
        assert_eq!(u.widths, [0.0, 0.3, 0.0, 0.0]);
        assert_eq!(u.curves, [1, 2, 0, 1]);
        assert_eq!(
            u.params,
            [4.0, 1.5, 0.5, 2.0],
            "gamma sanitised to the 4.0 ceiling"
        );
        assert_eq!(u.params2, [0.7, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn blend_states_are_distinct() {
        let states: Vec<_> = [
            BlendMode::Normal,
            BlendMode::Add,
            BlendMode::Multiply,
            BlendMode::Screen,
        ]
        .into_iter()
        .map(blend_state)
        .collect();
        for i in 0..4 {
            for j in (i + 1)..4 {
                assert_ne!(states[i], states[j]);
            }
        }
    }
}
