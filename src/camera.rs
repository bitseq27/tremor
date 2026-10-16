//! Orbit camera with inertial damping: mouse drag rotates, wheel zooms.

use cgmath::{perspective, point3, vec3, Deg, InnerSpace, Matrix4, Vector3, Vector4};

#[derive(Debug, Clone)]
pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub yaw_v: f32,
    pub pitch_v: f32,
    pub target_dist: f32,
    pub auto_spin: bool,
    pub dragging: bool,
    /// when set, the camera slews to (yaw, pitch) and holds - used to put a
    /// fresh epicentre centre-screen for a fixed dwell time
    pub focus: Option<(f32, f32)>,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: if std::env::var("TREMOR_FRONT").is_ok() { 0.0 } else { 0.35 },
            pitch: if std::env::var("TREMOR_FRONT").is_ok() { 0.0 } else { 0.28 },
            dist: 3.6,
            yaw_v: 0.0,
            pitch_v: 0.0,
            target_dist: 3.6,
            auto_spin: false,
            dragging: false,
            focus: None,
        }
    }
}

impl OrbitCamera {
    pub const DIST_MIN: f32 = 1.45;
    pub const DIST_MAX: f32 = 9.0;

    pub fn drag(&mut self, dx: f32, dy: f32) {
        self.focus = None; // manual input takes over from auto-track
        self.yaw_v = dx * 0.0052;
        self.pitch_v = dy * 0.0052;
        self.yaw += self.yaw_v;
        self.pitch = (self.pitch + self.pitch_v).clamp(-1.45, 1.45);
    }

    pub fn zoom(&mut self, delta: f32) {
        self.target_dist = (self.target_dist * (1.0 - delta * 0.0011)).clamp(Self::DIST_MIN, Self::DIST_MAX);
    }

    pub fn update(&mut self, dt: f32) {
        if let Some((ty, tp)) = self.focus {
            let k = 1.0 - 0.001f32.powf(dt.max(1e-4));
            let dy = (ty - self.yaw + std::f32::consts::TAU * 1.5)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            self.yaw += dy * k;
            self.pitch += (tp - self.pitch) * k;
            self.yaw_v = 0.0;
            self.pitch_v = 0.0;
        } else if !self.dragging {
            // inertia decay
            self.yaw += self.yaw_v;
            self.pitch = (self.pitch + self.pitch_v).clamp(-1.45, 1.45);
            self.yaw_v *= 0.90;
            self.pitch_v *= 0.90;
            if self.auto_spin && self.yaw_v.abs() < 0.0004 {
                self.yaw += dt * 0.020;
            }
        }
        self.dist += (self.target_dist - self.dist) * (1.0 - 0.85f32.powf(dt * 60.0));
    }

    pub fn eye(&self) -> Vector3<f32> {
        let cp = self.pitch.cos();
        vec3(
            self.dist * cp * self.yaw.sin(),
            self.dist * self.pitch.sin(),
            self.dist * cp * self.yaw.cos(),
        )
    }

    pub fn view(&self) -> Matrix4<f32> {
        let eye = self.eye();
        Matrix4::look_at_rh(point3(eye.x, eye.y, eye.z), point3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0))
    }

    pub fn projection(&self, aspect: f32) -> Matrix4<f32> {
        perspective(Deg(42.0), aspect, 0.05, 60.0)
    }

    pub fn view_proj(&self, aspect: f32) -> Matrix4<f32> {
        self.projection(aspect) * self.view()
    }

    /// Camera right / up in world space (for billboards).
    pub fn basis(&self) -> (Vector3<f32>, Vector3<f32>) {
        let v = self.view();
        let right = vec3(v.x.x, v.y.x, v.z.x);
        let up = vec3(v.x.y, v.y.y, v.z.y);
        (right, up)
    }

    /// Project a world point to physical pixel coords; None when behind eye.
    /// Returns (x, y, facing) with facing > 0 when the point is on the
    /// camera-facing hemisphere (used for marker occlusion).
    pub fn project(
        &self,
        p: Vector3<f32>,
        view_proj: Matrix4<f32>,
        vp: (u32, u32),
    ) -> Option<(f32, f32, f32)> {
        let clip = view_proj * Vector4::new(p.x, p.y, p.z, 1.0);
        if clip.w <= 1e-5 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        let x = (ndc.x * 0.5 + 0.5) * vp.0 as f32;
        let y = (1.0 - (ndc.y * 0.5 + 0.5)) * vp.1 as f32;
        let eye = self.eye();
        let to_eye = (eye - p).normalize();
        let facing = to_eye.dot(p.normalize());
        Some((x, y, facing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_clamps() {
        let mut c = OrbitCamera::default();
        for _ in 0..200 {
            c.zoom(50.0);
        }
        assert!((c.target_dist - OrbitCamera::DIST_MIN).abs() < 1e-4);
        for _ in 0..400 {
            c.zoom(-50.0);
        }
        assert!((c.target_dist - OrbitCamera::DIST_MAX).abs() < 1e-4);
    }

    #[test]
    fn center_projects_to_screen_center() {
        let c = OrbitCamera::default();
        let eye = c.eye();
        // a point on the visible limb facing the camera
        let p = eye.normalize();
        let vp = c.view_proj(16.0 / 9.0);
        let (x, y, facing) = c.project(p, vp, (1600, 900)).unwrap();
        assert!((x - 800.0).abs() < 1.5, "{x}");
        assert!((y - 450.0).abs() < 1.5, "{y}");
        assert!(facing > 0.9);
    }

    #[test]
    fn back_side_is_occluded() {
        let c = OrbitCamera::default();
        let eye = c.eye();
        let p = -eye.normalize();
        let vp = c.view_proj(16.0 / 9.0);
        let (_, _, facing) = c.project(p, vp, (1600, 900)).unwrap();
        assert!(facing < -0.9);
    }
}
