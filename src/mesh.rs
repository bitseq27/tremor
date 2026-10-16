//! Geometric mathematics: indexed UV-sphere model + bytemuck-safe vertex
//! layouts matching the GPU memory alignment rules.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GlobeVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

pub struct SphereMesh {
    pub vertices: Vec<GlobeVertex>,
    pub indices: Vec<u32>,
}

/// Latitude/longitude tessellated sphere. `rings` stacks from pole to pole,
/// `segments` slices around the equator. Vertices carry unit normals; the
/// fragment shader derives equirectangular uv from the normal itself so the
/// land texture stays glued to the surface under any rotation.
pub fn uv_sphere(rings: u32, segments: u32, radius: f32) -> SphereMesh {
    assert!(rings >= 3 && segments >= 3);
    let mut vertices = Vec::with_capacity(((rings + 1) * (segments + 1)) as usize);
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let phi = v * std::f32::consts::PI; // 0..=pi from +Y
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            let theta = u * std::f32::consts::TAU;
            let n = [
                phi.sin() * theta.sin(),
                phi.cos(),
                phi.sin() * theta.cos(),
            ];
            vertices.push(GlobeVertex {
                position: [n[0] * radius, n[1] * radius, n[2] * radius],
                normal: n,
            });
        }
    }
    let mut indices = Vec::with_capacity((rings * segments * 6) as usize);
    for r in 0..rings {
        for s in 0..segments {
            let a = r * (segments + 1) + s;
            let b = a + segments + 1;
            indices.extend_from_slice(&[a, b, a + 1, b, b + 1, a + 1]);
        }
    }
    SphereMesh { vertices, indices }
}

/// Screen-space quad corner used by billboard + UI pipelines (vertex_index
/// driven, no vertex buffer needed).
pub const QUAD_CORNERS: [[f32; 2]; 4] = [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_topology() {
        let m = uv_sphere(48, 96, 1.0);
        assert_eq!(m.vertices.len(), 49 * 97);
        assert_eq!(m.indices.len(), 48 * 96 * 6);
        for v in &m.vertices {
            let len = (v.position[0].powi(2) + v.position[1].powi(2) + v.position[2].powi(2)).sqrt();
            assert!((len - 1.0).abs() < 1e-5);
            assert!(v.position.iter().all(|c| c.is_finite()));
        }
        let vmax = m.vertices.len() as u32;
        assert!(m.indices.iter().all(|i| *i < vmax));
    }

    #[test]
    fn poles_present() {
        let m = uv_sphere(8, 8, 1.0);
        assert!((m.vertices[0].position[1] - 1.0).abs() < 1e-6);
    }
}
