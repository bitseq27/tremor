//! Planet pipeline: textured sphere + GPU-side seismic wave-fronts.
//! The fragment shader evaluates every pixel against the active quake
//! coordinate set (storage buffer) and applies the exponential decay
//! equation for the expanding rings.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::{assets, mesh, model};

pub const MAX_WAVES: usize = 96;
pub const MAX_EPICS: usize = 128;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GlobeUniforms {
    pub view_proj: [[f32; 4]; 4],
    /// xyz camera position
    pub cam_pos: [f32; 4],
    /// xyz light direction
    pub light_dir: [f32; 4],
    /// x time, y graticule on, z waves on, w wave count
    pub misc: [f32; 4],
    /// x epicentre mark count
    pub misc2: [f32; 4],
    /// xyz sun direction (subsolar point), w pad
    pub sun_dir: [f32; 4],
}

/// Persistent surface-attached epicentre mark (X + pip + select ring).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EpicGpu {
    /// xyz unit epicentre, w magnitude
    pub center: [f32; 4],
    /// x age, y ttl, z selected, w pad
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct WaveGpu {
    /// xyz unit epicentre, w start time
    pub center: [f32; 4],
    /// x magnitude, y ttl, z angular speed, w ring width
    pub params: [f32; 4],
    /// x kind (0 = event wave, 1 = new-event fast flash), y..w pad
    pub extra: [f32; 4],
}

impl WaveGpu {
    /// Timeline-derived wave front: starts at the event ORIGIN time so
    /// scrubbing the playhead replays every front in the window.
    pub fn from_quake(q: &model::Quake, now: f64) -> Self {
        let ttl = model::wave_ttl(q.mag) as f32;
        Self {
            center: [q.unit_pos()[0], q.unit_pos()[1], q.unit_pos()[2], (q.time_s() - now) as f32],
            params: [q.mag, ttl, 0.16 + q.mag * 0.022, 0.035 + q.mag * 0.006],
            extra: [0.0; 4],
        }
    }

    /// Fast bright flash ring for a freshly ingested event.
    pub fn flash(center: [f32; 3], mag: f32) -> Self {
        Self {
            center: [center[0], center[1], center[2], 0.0],
            params: [mag, 2.6, 1.15, 0.020],
            extra: [1.0, 0.0, 0.0, 0.0],
        }
    }

    /// `now` keeps the start time relative to the frame: absolute epoch
    /// seconds do not survive f32 precision (ulp ~128 s at 1.7e9).
    pub fn from_wave(w: &model::Wave, now: f64) -> Self {
        let ttl = model::wave_ttl(w.mag) as f32;
        Self {
            center: [w.center[0], w.center[1], w.center[2], (w.start - now) as f32],
            // bigger events propagate a little faster and wider
            params: [w.mag, ttl, 0.16 + w.mag * 0.022, 0.035 + w.mag * 0.006],
            extra: [0.0; 4],
        }
    }
}

pub struct GlobePipe {
    pipeline: wgpu::RenderPipeline,
    pub uniforms: wgpu::Buffer,
    pub uniform_bind: wgpu::BindGroup,
    waves: wgpu::Buffer,
    wave_bind: wgpu::BindGroup,
    epics: wgpu::Buffer,
    epic_bind: wgpu::BindGroup,
    tex_bind: wgpu::BindGroup,
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    index_count: u32,
}

impl GlobePipe {
    pub fn new(gpu: &super::Gpu) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/globe.wgsl"));

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe-u"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<GlobeUniforms>() as u64,
                    ),
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe-t"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let wave_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe-w"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        (std::mem::size_of::<WaveGpu>() * MAX_WAVES) as u64,
                    ),
                },
                count: None,
            }],
        });

        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe-uniforms"),
            contents: bytemuck::cast_slice(&[GlobeUniforms {
                view_proj: [[0.0; 4]; 4],
                cam_pos: [0.0; 4],
                light_dir: [0.0, 0.0, 1.0, 0.0],
                misc: [0.0; 4],
                misc2: [0.0; 4],
                sun_dir: [0.0, 0.0, 1.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe-u-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        let px = assets::build_earth_texture(4096, 2048);
        let tex = super::upload_rgba8(
            device,
            &gpu.queue,
            "earth",
            &px,
            4096,
            2048,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::empty(),
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let smp = super::linear_sampler(device, "earth-smp");
        let (lpx, lw, lh) = assets::build_citylights_rgba();
        let ltex = super::upload_rgba8(
            device,
            &gpu.queue,
            "citylights",
            &lpx,
            lw,
            lh,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::empty(),
        );
        let lview = ltex.create_view(&wgpu::TextureViewDescriptor::default());
        let tex_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe-t-bg"),
            layout: &tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&smp),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lview),
                },
            ],
        });

        let epic_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe-e"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        (std::mem::size_of::<EpicGpu>() * MAX_EPICS) as u64,
                    ),
                },
                count: None,
            }],
        });
        let epics = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("epics"),
            size: (std::mem::size_of::<EpicGpu>() * MAX_EPICS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let epic_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe-e-bg"),
            layout: &epic_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: epics.as_entire_binding(),
            }],
        });

        let waves = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("waves"),
            size: (std::mem::size_of::<WaveGpu>() * MAX_WAVES) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wave_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globe-w-bg"),
            layout: &wave_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: waves.as_entire_binding(),
            }],
        });

        let mesh = mesh::uv_sphere(64, 128, 1.0);
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sphere-vb"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sphere-ib"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let index_count = mesh.indices.len() as u32;

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globe-pl"),
            bind_group_layouts: &[&uniform_layout, &tex_layout, &wave_layout, &epic_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("globe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<mesh::GlobeVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            uniforms,
            uniform_bind,
            waves,
            wave_bind,
            epics,
            epic_bind,
            tex_bind,
            vb,
            ib,
            index_count,
        }
    }

    pub fn write_waves(&self, queue: &wgpu::Queue, wave_data: &[WaveGpu]) {
        let n = wave_data.len().min(MAX_WAVES);
        let mut buf = [WaveGpu {
            center: [0.0; 4],
            params: [0.0; 4],
            extra: [0.0; 4],
        }; MAX_WAVES];
        buf[..n].copy_from_slice(&wave_data[..n]);
        queue.write_buffer(&self.waves, 0, bytemuck::cast_slice(&buf));
    }

    pub fn write_epics(&self, queue: &wgpu::Queue, data: &[EpicGpu]) {
        let n = data.len().min(MAX_EPICS);
        let mut buf = [EpicGpu {
            center: [0.0; 4],
            params: [0.0; 4],
        }; MAX_EPICS];
        buf[..n].copy_from_slice(&data[..n]);
        queue.write_buffer(&self.epics, 0, bytemuck::cast_slice(&buf));
    }

    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind, &[]);
        pass.set_bind_group(1, &self.tex_bind, &[]);
        pass.set_bind_group(2, &self.wave_bind, &[]);
        pass.set_bind_group(3, &self.epic_bind, &[]);
        pass.set_vertex_buffer(0, self.vb.slice(..));
        pass.set_index_buffer(self.ib.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
