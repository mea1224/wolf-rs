use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

pub struct Display {
    instance: wgpu::Instance,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wolf_render::TilePipeline,
    camera: wgpu::BindGroup,
    atlas: wgpu::BindGroup,
    texture: wgpu::Texture,
    instances: wgpu::Buffer,
    suspended: bool,
    logical_size: (u32, u32),
}
impl Display {
    pub async fn new(window: Arc<Window>, logical_size: (u32, u32)) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        eprintln!("GPU: {:?}", adapter.get_info());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let limit = device.limits().max_texture_dimension_2d;
        if logical_size.0 == 0
            || logical_size.1 == 0
            || logical_size.0 > limit
            || logical_size.1 > limit
        {
            return Err(format!(
                "logical resolution {}x{} exceeds GPU texture dimension limit {limit}",
                logical_size.0, logical_size.1
            ));
        }
        if size.width > limit || size.height > limit {
            return Err(format!(
                "window surface {}x{} exceeds GPU texture dimension limit {limit}",
                size.width, size.height
            ));
        }
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("no supported surface configuration")?;
        let caps = surface.get_capabilities(&adapter);
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        config.format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(config.format);
        config.present_mode = wgpu::PresentMode::Fifo;
        // Keep only one queued frame so input is not buried behind a deeper
        // presentation queue. FIFO continues to avoid tearing.
        config.desired_maximum_frame_latency = 1;
        surface.configure(&device, &config);
        let pipeline = wolf_render::create_pipeline(&device, config.format);
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("preview camera"),
            contents: bytemuck::bytes_of(&wolf_render::Camera {
                origin: [0., 0.],
                viewport: [logical_size.0 as f32, logical_size.1 as f32],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let (texture, view, sampler) =
            wolf_render::create_atlas(&device, logical_size.0, logical_size.1, 1);
        let atlas = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let instances = wolf_render::create_instances(
            &device,
            &[wolf_render::TileInstance {
                origin: [0., 0.],
                size: [logical_size.0 as f32, logical_size.1 as f32],
                uv_min: [0., 0.],
                uv_max: [1., 1.],
                tint: [1.; 4],
                page: 0,
                flags: 0,
            }],
        )
        .unwrap();
        Ok(Self {
            instance,
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            camera,
            atlas,
            texture,
            instances,
            suspended: size.width == 0 || size.height == 0,
            logical_size,
        })
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.suspended = width == 0 || height == 0;
        if self.suspended {
            return;
        }
        let limit = self.device.limits().max_texture_dimension_2d;
        self.config.width = width.min(limit);
        self.config.height = height.min(limit);
        self.surface.configure(&self.device, &self.config);
    }
    pub fn render(
        &mut self,
        image: &wolf_media::Image,
        capture: Option<&std::path::Path>,
    ) -> Result<bool, String> {
        if image.dimensions() != self.logical_size {
            return Err("render image differs from configured logical resolution".into());
        }
        if self.suspended {
            return Ok(false);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                drop(frame);
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .map_err(|e| e.to_string())?;
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false)
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface validation failed".into())
            }
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.logical_size.0 * 4),
                rows_per_image: Some(self.logical_size.1),
            },
            wgpu::Extent3d {
                width: self.logical_size.0,
                height: self.logical_size.1,
                depth_or_array_layers: 1,
            },
        );
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let scale = (self.config.width as f32 / self.logical_size.0 as f32)
                .min(self.config.height as f32 / self.logical_size.1 as f32);
            let width = self.logical_size.0 as f32 * scale;
            let height = self.logical_size.1 as f32 * scale;
            pass.set_viewport(
                (self.config.width as f32 - width) / 2.,
                (self.config.height as f32 - height) / 2.,
                width,
                height,
                0.,
                1.,
            );
            wolf_render::draw(
                &mut pass,
                &self.pipeline,
                &self.camera,
                &self.atlas,
                &self.instances,
                1,
            );
        }
        let row = (self.config.width * 4).div_ceil(256) * 256;
        let readback = if capture.is_some() {
            if !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC)
                || !matches!(
                    self.config.format,
                    wgpu::TextureFormat::Bgra8UnormSrgb
                        | wgpu::TextureFormat::Bgra8Unorm
                        | wgpu::TextureFormat::Rgba8UnormSrgb
                        | wgpu::TextureFormat::Rgba8Unorm
                )
            {
                return Err("surface does not support RGBA/BGRA readback".into());
            }
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("capture"),
                size: u64::from(row) * u64::from(self.config.height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &frame.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
            );
            Some(buffer)
        } else {
            None
        };
        self.queue.submit([encoder.finish()]);
        if let (Some(buffer), Some(path)) = (readback, capture) {
            let (send, recv) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = send.send(result);
                });
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
                .map_err(|e| e.to_string())?;
            recv.recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            let mapped = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|e| e.to_string())?;
            let mut rgba =
                Vec::with_capacity(self.config.width as usize * self.config.height as usize * 4);
            for line in mapped.chunks_exact(row as usize) {
                rgba.extend_from_slice(&line[..self.config.width as usize * 4]);
            }
            if matches!(
                self.config.format,
                wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
            ) {
                for p in rgba.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
            }
            drop(mapped);
            buffer.unmap();
            wolf_media::Image::new(
                self.config.width as usize,
                self.config.height as usize,
                rgba,
            )?
            .write_png(std::fs::File::create(path).map_err(|e| e.to_string())?)?;
            eprintln!("GPU surface captured: {}", path.display());
        }
        self.queue.present(frame);
        Ok(true)
    }
}
