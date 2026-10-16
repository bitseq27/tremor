//! 2D HUD pipeline: one batched vertex stream draws panel fills, 1px frames,
//! bars and bitmap-font text in physical pixel space. This is what carries
//! the orrery HMI look (black cells, #6E6E6E rules, VT323 glyphs at the
//! orrery UI sizes 10/13/16 px).

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::assets::{self, Face, FACE_MD, FACE_SM, FACE_XS};
use crate::palette::Rgba;

/// Cell metrics of the three faces, for HUD box math.
pub const XS_W: f32 = FACE_XS.cw as f32;
pub const XS_H: f32 = FACE_XS.ch as f32;
pub const SM_W: f32 = FACE_SM.cw as f32;
pub const SM_H: f32 = FACE_SM.ch as f32;
pub const MD_W: f32 = FACE_MD.cw as f32;
pub const MD_H: f32 = FACE_MD.ch as f32;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// 0 = solid fill, 1 = font atlas sample
    pub mode: f32,
    pub _pad: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct UiUniforms {
    /// x width, y height
    pub screen: [f32; 4],
}

#[derive(Debug, Default, Clone)]
pub struct UiBatch {
    pub verts: Vec<UiVertex>,
}

impl UiBatch {
    pub fn clear(&mut self) {
        self.verts.clear();
    }

    /// Free-form quad from four pixel-space corners.
    pub fn quad_pts(&mut self, p: [[f32; 2]; 4], color: Rgba) {
        let c = |pos: [f32; 2]| UiVertex {
            pos,
            uv: [0.0, 0.0],
            color,
            mode: 0.0,
            _pad: 0.0,
        };
        let (a, b, d, e) = (c(p[0]), c(p[1]), c(p[2]), c(p[3]));
        self.verts.extend_from_slice(&[a, b, d, b, e, d]);
    }

    /// 1px (or `w` px) thick line segment between two points.
    pub fn line_seg(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, color: Rgba) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-4 {
            return;
        }
        let nx = -dy / len * w * 0.5;
        let ny = dx / len * w * 0.5;
        self.quad_pts(
            [
                [x0 + nx, y0 + ny],
                [x1 + nx, y1 + ny],
                [x0 - nx, y0 - ny],
                [x1 - nx, y1 - ny],
            ],
            color,
        );
    }

    /// Polyline through pixel-space points.
    pub fn polyline(&mut self, pts: &[(f32, f32)], w: f32, color: Rgba) {
        for pair in pts.windows(2) {
            self.line_seg(pair[0].0, pair[0].1, pair[1].0, pair[1].1, w, color);
        }
    }

    fn push_quad(&mut self, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4], color: Rgba, mode: f32) {
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
        let c = |px: f32, py: f32, u: f32, v: f32| UiVertex {
            pos: [px, py],
            uv: [u, v],
            color,
            mode,
            _pad: 0.0,
        };
        let a = c(x, y, u0, v0);
        let b = c(x + w, y, u1, v0);
        let d = c(x, y + h, u0, v1);
        let e = c(x + w, y + h, u1, v1);
        self.verts.extend_from_slice(&[a, b, d, b, e, d]);
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        self.push_quad(x, y, w, h, [0.0, 0.0, 0.0, 0.0], color, 0.0);
    }

    /// 1px frame, reference-panel style.
    pub fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        self.rect(x, y, w, 1.0, color);
        self.rect(x, y + h - 1.0, w, 1.0, color);
        self.rect(x, y, 1.0, h, color);
        self.rect(x + w - 1.0, y, 1.0, h, color);
    }

    pub fn hline(&mut self, x: f32, y: f32, w: f32, color: Rgba) {
        self.rect(x, y, w, 1.0, color);
    }

    pub fn vline(&mut self, x: f32, y: f32, h: f32, color: Rgba) {
        self.rect(x, y, 1.0, h, color);
    }

    /// Text in an explicit face; returns the x just past the last cell.
    pub fn text_face(&mut self, f: &Face, x: f32, y: f32, s: &str, color: Rgba) -> f32 {
        let (aw, ah) = (assets::ATLAS_W as f32, assets::ATLAS_H as f32);
        let (cw, ch) = (f.cw as f32, f.ch as f32);
        let mut cx = x;
        for ch_ in s.chars() {
            let idx = assets::glyph_idx(ch_);
            let gx = (idx % assets::FONT_ATLAS_COLS) as f32 * cw;
            let gy = f.yoff as f32 + (idx / assets::FONT_ATLAS_COLS) as f32 * ch;
            let u0 = gx / aw;
            let v0 = gy / ah;
            let u1 = (gx + cw) / aw;
            let v1 = (gy + ch) / ah;
            self.push_quad(cx, y, cw, ch, [u0, v0, u1, v1], color, 1.0);
            cx += cw;
        }
        cx
    }

    /// orrery PX (16 px): banners, body text
    pub fn text(&mut self, x: f32, y: f32, s: &str, color: Rgba) -> f32 {
        self.text_face(&FACE_MD, x, y, s, color)
    }
    /// orrery PX_SM (13 px): labels, readouts, keys, tabs
    pub fn text_sm(&mut self, x: f32, y: f32, s: &str, color: Rgba) -> f32 {
        self.text_face(&FACE_SM, x, y, s, color)
    }
    /// 10 px: plot ticks, annotations, chips
    pub fn text_xs(&mut self, x: f32, y: f32, s: &str, color: Rgba) -> f32 {
        self.text_face(&FACE_XS, x, y, s, color)
    }

    pub fn text_width_face(f: &Face, s: &str) -> f32 {
        s.chars().count() as f32 * f.cw as f32
    }
    pub fn text_width(s: &str) -> f32 {
        Self::text_width_face(&FACE_MD, s)
    }
    pub fn text_width_sm(s: &str) -> f32 {
        Self::text_width_face(&FACE_SM, s)
    }
    pub fn text_width_xs(s: &str) -> f32 {
        Self::text_width_face(&FACE_XS, s)
    }

    /// Right-aligned text ending at `x_end`.
    pub fn text_right(&mut self, x_end: f32, y: f32, s: &str, color: Rgba) {
        self.text(x_end - Self::text_width(s), y, s, color);
    }
    pub fn text_right_sm(&mut self, x_end: f32, y: f32, s: &str, color: Rgba) {
        self.text_sm(x_end - Self::text_width_sm(s), y, s, color);
    }
    pub fn text_right_xs(&mut self, x_end: f32, y: f32, s: &str, color: Rgba) {
        self.text_xs(x_end - Self::text_width_xs(s), y, s, color);
    }
}

pub struct UiPipe {
    pipeline: wgpu::RenderPipeline,
    pub uniforms: wgpu::Buffer,
    uniform_bind: wgpu::BindGroup,
    font_bind: wgpu::BindGroup,
    vb: wgpu::Buffer,
    vb_cap: u64,
}

impl UiPipe {
    pub fn new(gpu: &super::Gpu) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::include_wgsl!("../shaders/ui.wgsl"));

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-u"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<UiUniforms>() as u64,
                    ),
                },
                count: None,
            }],
        });
        let font_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-f"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(
                        wgpu::SamplerBindingType::NonFiltering,
                    ),
                    count: None,
                },
            ],
        });

        let (atlas, aw, ah) = assets::build_font_atlas();
        let tex = super::upload_rgba8(
            device,
            &gpu.queue,
            "font-atlas",
            &atlas,
            aw,
            ah,
            wgpu::TextureFormat::R8Unorm,
            wgpu::TextureUsages::empty(),
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let smp = super::nearest_sampler(device, "font-smp");
        let font_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-f-bg"),
            layout: &font_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&smp),
                },
            ],
        });

        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui-uniforms"),
            contents: bytemuck::cast_slice(&[UiUniforms {
                screen: [1.0, 1.0, 0.0, 0.0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-u-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        let vb_cap = (std::mem::size_of::<UiVertex>() * 6 * 65536) as u64;
        let vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-vb"),
            size: vb_cap,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui-pl"),
            bind_group_layouts: &[&uniform_layout, &font_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 32,
                            shader_location: 3,
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
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
            font_bind,
            vb,
            vb_cap,
        }
    }

    pub fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        queue: &wgpu::Queue,
        batch: &UiBatch,
        screen: [f32; 2],
    ) {
        if batch.verts.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::cast_slice(&[UiUniforms {
                screen: [screen[0], screen[1], 0.0, 0.0],
            }]),
        );
        let bytes = bytemuck::cast_slice(&batch.verts);
        if bytes.len() as u64 <= self.vb_cap {
            queue.write_buffer(&self.vb, 0, bytes);
        } else {
            let fit = (self.vb_cap as usize / std::mem::size_of::<UiVertex>()) as usize;
            queue.write_buffer(&self.vb, 0, bytemuck::cast_slice(&batch.verts[..fit]));
        }
        let n = batch.verts.len().min((self.vb_cap as usize / std::mem::size_of::<UiVertex>()) as usize);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind, &[]);
        pass.set_bind_group(1, &self.font_bind, &[]);
        pass.set_vertex_buffer(0, self.vb.slice(..));
        pass.draw(0..n as u32, 0..1);
    }
}
