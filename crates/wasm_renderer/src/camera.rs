//! Camera module — projection + view matrices from position/direction.
//!
//! Receives SetCamera commands from the main thread.
//! Produces the `CameraUniforms` struct consumed by all render/compute stages.

use glam::{Mat4, Vec3};

/// Camera state. Updated by SetCamera commands, consumed by render_frame.
pub struct Camera {
    position: Vec3,
    direction: Vec3,
    up: Vec3,
    fov_y: f32,
    aspect: f32,
    near: f32,
    far: f32,
}

impl Camera {
    pub fn new(width: f32, height: f32) -> Self {
        // Default: orbit view of a 64³ chunk at the origin.
        // Position ~120 units back along the diagonal, looking at chunk center (32,32,32).
        let target = Vec3::new(32.0, 32.0, 32.0);
        let position = Vec3::new(100.0, 80.0, 100.0);
        let direction = (target - position).normalize();
        Self {
            position,
            direction,
            up: Vec3::Y,
            fov_y: std::f32::consts::FRAC_PI_4, // 45 degrees
            aspect: width / height.max(1.0),
            near: 0.1,
            far: 2000.0,
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.aspect = width / height.max(1.0);
    }

    pub fn set_fov(&mut self, fov_degrees: f32) {
        self.fov_y = (fov_degrees.clamp(10.0, 120.0)).to_radians();
    }

    pub fn set_look(&mut self, position: Vec3, direction: Vec3) {
        self.position = position;
        let len = direction.length();
        self.direction = if len > 1e-6 {
            direction / len
        } else {
            Vec3::NEG_Z
        };
    }

    pub fn view(&self) -> Mat4 {
        let target = self.position + self.direction;
        Mat4::look_at_rh(self.position, target, self.up)
    }

    pub fn proj(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far)
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj() * self.view()
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }

    pub fn direction(&self) -> Vec3 {
        self.direction
    }

    pub fn near(&self) -> f32 {
        self.near
    }

    pub fn far(&self) -> f32 {
        self.far
    }

    pub fn fov_y(&self) -> f32 {
        self.fov_y
    }

    /// Reposition camera to frame a model with the given world-space AABB center and extent.
    /// Places camera along a diagonal, far enough back to see the whole model.
    pub fn frame_model(&mut self, center: Vec3, extent: f32) {
        let distance = extent * 1.8; // back off ~1.8x the model size
        let offset = Vec3::new(0.6, 0.4, 0.6).normalize() * distance;
        self.position = center + offset;
        self.direction = (center - self.position).normalize();
    }

    pub fn aspect(&self) -> f32 {
        self.aspect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    #[test]
    fn default_camera_valid() {
        let cam = Camera::new(1920.0, 1080.0);
        assert!(cam.near > 0.0, "Near must be positive");
        assert!(cam.near < cam.far, "Near must be less than far");
        assert!(cam.aspect > 0.0, "Aspect must be positive");
        let dir_len = cam.direction.length();
        assert!((dir_len - 1.0).abs() < EPSILON, "Direction must be normalized: len={dir_len}");
    }

    #[test]
    fn direction_normalized_after_set_look() {
        let mut cam = Camera::new(800.0, 600.0);
        cam.set_look(Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0));
        let len = cam.direction().length();
        assert!((len - 1.0).abs() < EPSILON, "Direction not normalized: len={len}");
    }

    #[test]
    fn zero_direction_falls_back() {
        let mut cam = Camera::new(800.0, 600.0);
        cam.set_look(Vec3::ZERO, Vec3::ZERO);
        assert_eq!(cam.direction(), Vec3::NEG_Z, "Zero direction should fall back to -Z");
    }

    #[test]
    fn view_proj_composition() {
        let cam = Camera::new(1920.0, 1080.0);
        let vp = cam.view_proj();
        let expected = cam.proj() * cam.view();
        for i in 0..16 {
            let a = vp.to_cols_array()[i];
            let b = expected.to_cols_array()[i];
            assert!((a - b).abs() < EPSILON, "view_proj mismatch at [{i}]: {a} vs {b}");
        }
    }

    #[test]
    fn view_matrix_invertible() {
        let cam = Camera::new(800.0, 600.0);
        let view = cam.view();
        let inv = view.inverse();
        let identity = view * inv;
        for i in 0..4 {
            for j in 0..4 {
                let expected = if i == j { 1.0 } else { 0.0 };
                let actual = identity.col(j)[i];
                assert!(
                    (actual - expected).abs() < EPSILON,
                    "View * View^-1 not identity at [{i},{j}]: {actual}"
                );
            }
        }
    }

    #[test]
    fn proj_matrix_finite() {
        let cam = Camera::new(800.0, 600.0);
        let proj = cam.proj();
        for val in proj.to_cols_array() {
            assert!(val.is_finite(), "Projection matrix has non-finite value: {val}");
        }
    }

    #[test]
    fn resize_updates_aspect() {
        let mut cam = Camera::new(800.0, 600.0);
        let old_aspect = cam.aspect;
        cam.resize(1920.0, 1080.0);
        assert!((cam.aspect - 1920.0 / 1080.0).abs() < EPSILON);
        assert_ne!(cam.aspect, old_aspect);
    }

    #[test]
    fn resize_zero_height_safe() {
        let mut cam = Camera::new(800.0, 600.0);
        cam.resize(800.0, 0.0);
        assert!(cam.aspect.is_finite(), "Aspect must be finite even with zero height");
    }

    #[test]
    fn point_at_center_projects_to_clip_center() {
        let cam = Camera::new(800.0, 600.0);
        // A point directly in front of the camera
        let point = cam.position() + cam.direction();
        let clip = cam.view_proj() * glam::Vec4::new(point.x, point.y, point.z, 1.0);
        // In clip space, x and y should be near 0 (center of screen)
        let ndc_x = clip.x / clip.w;
        let ndc_y = clip.y / clip.w;
        assert!(ndc_x.abs() < 0.01, "Center point NDC x should be ~0: {ndc_x}");
        assert!(ndc_y.abs() < 0.01, "Center point NDC y should be ~0: {ndc_y}");
    }

    #[test]
    fn point_in_front_has_valid_depth() {
        let cam = Camera::new(800.0, 600.0);
        let point = cam.position() + cam.direction() * 5.0;
        let clip = cam.view_proj() * glam::Vec4::new(point.x, point.y, point.z, 1.0);
        let ndc_z = clip.z / clip.w;
        // In right-handed perspective, depth should be in [0, 1] for points between near and far
        assert!(ndc_z >= 0.0 && ndc_z <= 1.0, "Depth should be in [0,1]: {ndc_z}");
    }
}
