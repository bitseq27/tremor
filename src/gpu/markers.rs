//! Epicentre markers: instanced camera-facing billboards, magnitude colour
//! coded, depth tested against the globe so the far side stays hidden.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const MAX_MARKERS: usize = 512;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MarkerGpu {
    /// xyz unit epicentre (scaled to surface)
    pub pos: [f32; 4],
    pub color: [f32; 4],
    /// x world size, y selected flag, z age seconds, w magnitude
    pub misc: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MarkerUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
    /// x time, y.. unused
    pub misc: [f32; 4],
}

pub struct MarkerPipe {
    pipeline: wgpu::RenderPipeline,
    pub uniforms: wgpu::Buffer,
    uniform_bind: wgpu::BindGroup,
    instances: wgpu::Buffer,
    instance_bind: wgpu::BindGroup,
}

impl MarkerPipe {
    pub fn new(gpu: &super::Gpu) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/markers.wgsl"));

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("marker-u"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<MarkerUniforms>() as u64,
                    ),
                },
                count: None,
            }],
        });
        let instance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("marker-i"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        (std::mem::size_of::<MarkerGpu>() * MAX_MARKERS) as u64,
                    ),
                },
                count: None,
            }],
        });

        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("marker-uniforms"),
            contents: bytemuck::cast_slice(&[MarkerUniforms {
                view_proj: [[0.0; 4]; 4],
                right: [1.0, 0.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0, 0.0],
                misc: [0.0; 4],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("marker-u-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("marker-instances"),
            size: (std::mem::size_of::<MarkerGpu>() * MAX_MARKERS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instance_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("marker-i-bg"),
            layout: &instance_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instances.as_entire_binding(),
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("marker-pl"),
            bind_group_layouts: &[&uniform_layout, &instance_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("markers"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
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
            instances,
            instance_bind,
        }
    }

    pub fn write(&self, queue: &wgpu::Queue, data: &[MarkerGpu]) {
        let n = data.len().min(MAX_MARKERS);
        let mut buf = vec![
            MarkerGpu {
                pos: [0.0; 4],
                color: [0.0; 4],
                misc: [0.0; 4],
            };
            n.max(1)
        ];
        buf[..n].copy_from_slice(&data[..n]);
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&buf));
    }

    /// `count` instances drawn this frame.
    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, count: u32) {
        if count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind, &[]);
        pass.set_bind_group(1, &self.instance_bind, &[]);
        pass.draw(0..6, 0..count);
    }
}
