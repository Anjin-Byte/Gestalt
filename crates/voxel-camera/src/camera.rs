//! The cameras: the hybrid orbit — ambient turntable spin until first grabbed,
//! then direct drag control with fling momentum and friction — and the
//! interactive free-fly. The deterministic [`orbit_camera`] turntable remains
//! for scripted, frame-rate-independent profiling runs.

use glam::Vec3;
use voxel_core::{GpuCamera, VoxelCoord};

use crate::input::Input;

/// Vertical field of view, in degrees.
const FOV_Y_DEG: f32 = 60.0;
/// Pitch is clamped just shy of straight up/down to keep the basis well-defined.
const MAX_PITCH: f32 = 1.553_343; // ≈ 89° in radians
/// Radians of look rotation per pixel of mouse motion.
const LOOK_SENSITIVITY: f32 = 0.003;
/// Multiplicative speed change per scroll-wheel notch.
const SCROLL_SPEED_FACTOR: f32 = 1.15;

/// Interactive-orbit elevation is clamped a little further from the poles than
/// the fly camera's pitch: at exactly ±90° the eye sits directly over the
/// pivot and its forward is ±Y, degenerating `forward × Y`.
const MAX_ELEVATION: f32 = 1.4; // ≈ 80°
/// Interactive-orbit radius bounds, as a fraction of the framed extent: close
/// enough to inspect a surface, far enough to frame the whole object.
const MIN_ORBIT_RADIUS_FRAC: f32 = 0.3;
const MAX_ORBIT_RADIUS_FRAC: f32 = 8.0;
/// Fraction of the viewport's binding axis a freshly loaded model fills (see
/// [`fit_orbit_radius`]). A little shy of full so nothing kisses the frame edge
/// as the turntable spins.
const LOAD_FILL: f32 = 0.99;
/// Radius scale per scroll-wheel notch (orbit zoom).
const ZOOM_PER_NOTCH: f32 = 1.15;
/// The hybrid orbit's ambient display spin (radians/second) — active only
/// while the control is pristine (never grabbed since construction).
const AMBIENT_RATE: f32 = 0.25;
/// Momentum friction: velocity decays as `exp(-FRICTION · dt)` after release
/// (half-life ≈ 0.31 s — a visible glide that settles in about a second).
const FRICTION: f32 = 2.2;
/// Angular speed (rad/s) below which a glide is considered settled.
const SPIN_EPSILON: f32 = 0.02;
/// Pivot pan: world units per pixel, as a fraction of the view distance.
const PAN_SENSITIVITY: f32 = 0.0015;
/// Hold-to-boost movement multiplier.
const BOOST_MULTIPLIER: f32 = 4.0;

/// Builds the orthonormal camera basis `(forward, right, up)` for a forward
/// direction, matching the renderer's convention (right-handed, world-up `+Y`).
fn basis(forward: Vec3) -> (Vec3, Vec3, Vec3) {
    let forward = forward.normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    (forward, right, up)
}

/// Packs an eye/forward pair into the GPU camera uniform for a `w×h` viewport
/// over an `n³` grid with `k` internal levels.
fn pack(eye: Vec3, forward: Vec3, w: u32, h: u32, n: f32, k: u32) -> GpuCamera {
    let (forward, right, up) = basis(forward);
    GpuCamera {
        eye: eye.to_array(),
        tan: (FOV_Y_DEG.to_radians() * 0.5).tan(),
        forward: forward.to_array(),
        aspect: w as f32 / h as f32,
        right: right.to_array(),
        n,
        up: up.to_array(),
        pad: 0.0,
        dims: [w, h, k, 0],
    }
}

/// What the turntable orbits and frames: a world-space pivot plus the span it
/// keeps in view. Built from the whole grid ([`grid`](Self::grid), the legacy
/// behaviour) or, better, from the object's own occupied bounding box
/// ([`aabb`](Self::aabb)) so a model that voxelizes into a corner of a large
/// grid is centred and framed on *itself*, not on the empty grid around it.
///
/// `extent` drives both the orbit radius and the eye height, so the target
/// fills a consistent fraction of the view no matter how small it is within
/// the grid. The radius/height ratios (`1.6`, `0.35`) are the legacy formula's,
/// with the framed extent substituted for the grid size `n`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitFrame {
    /// World-space pivot: the camera looks here and circles it.
    pub center: Vec3,
    /// The framed span (the target's largest axis), in world voxels.
    pub extent: f32,
}

impl OrbitFrame {
    /// The legacy whole-grid frame: pivot at the grid centre, framed to the
    /// full `n³` extent. The fallback when a scene has no occupied voxels.
    #[must_use]
    pub fn grid(n: f32) -> Self {
        Self {
            center: Vec3::splat(n * 0.5),
            extent: n,
        }
    }

    /// Frames an **inclusive** world-voxel bounding box (`min`/`max` corners, as
    /// [`SparseTree::occupied_bbox`](voxel_core::SparseTree::occupied_bbox)
    /// returns) so the whole box stays in view, pivoting on its centre. A
    /// degenerate box (a flat plane or a single voxel) floors to a one-voxel
    /// extent, keeping the framing distance finite.
    #[must_use]
    pub fn aabb(min: Vec3, max: Vec3) -> Self {
        // Inclusive corners span (max - min + 1) voxels along each axis.
        let span = max - min + Vec3::ONE;
        Self {
            center: (min + max) * 0.5 + Vec3::splat(0.5), // voxel centres, not corners
            extent: span.max_element().max(1.0),
        }
    }

    /// The frame for a scene's occupied bounding box (from
    /// [`SparseTree::occupied_bbox`](voxel_core::SparseTree::occupied_bbox)):
    /// [`aabb`](Self::aabb) of the box, or [`grid`](Self::grid)`(n)` when the
    /// scene is empty. The one-call convenience both front ends use to point
    /// the turntable at the object.
    #[must_use]
    pub fn for_bbox(bbox: Option<(VoxelCoord, VoxelCoord)>, n: f32) -> Self {
        match bbox {
            Some((min, max)) => Self::aabb(voxel(min), voxel(max)),
            None => Self::grid(n),
        }
    }
}

/// A world-voxel coordinate as a float position.
fn voxel(c: VoxelCoord) -> Vec3 {
    Vec3::new(c.x as f32, c.y as f32, c.z as f32)
}

/// The load-time orbit radius — a fraction of the frame's extent (the units of
/// [`OrbitControl::radius`]) — that **fits the scene's bounding box to the
/// viewport**, so a wide/flat model fills the screen as much as a compact one
/// instead of floating small when only its largest world axis is framed
/// ([`OrbitFrame::aabb`] frames that axis alone, ignoring the object's other
/// two dimensions and the viewport's `aspect`).
///
/// It sizes the distance from the object's real box and the frustum: the
/// worst-case **horizontal reach over a turntable Y-spin** (`√(x²+z²)/2`, so the
/// object never clips as it rotates) against the horizontal FOV, and the
/// down-tilted **half-height** against the vertical FOV — whichever binds. Empty
/// scenes keep the legacy pose. Scale-invariant: the result is a fraction of the
/// extent, so a resolution change re-frames with no remap.
#[must_use]
pub fn fit_orbit_radius(bbox: Option<(VoxelCoord, VoxelCoord)>, aspect: f32) -> f32 {
    let Some((min, max)) = bbox else {
        return OrbitControl::default().radius; // empty scene: the legacy framing
    };
    // Inclusive corners → per-axis span in voxels, floored to one voxel.
    let span = (voxel(max) - voxel(min) + Vec3::ONE).max(Vec3::ONE);
    let extent = span.max_element();
    let reach = (span.x * span.x + span.z * span.z).sqrt() * 0.5;
    let (se, ce) = OrbitControl::default().elevation.sin_cos();
    let half_height = span.y * 0.5 * ce + reach * se;
    let half_fov_v = FOV_Y_DEG.to_radians() * 0.5;
    let half_fov_h = (aspect.max(1e-3) * half_fov_v.tan()).atan();
    let dist =
        (reach / (LOAD_FILL * half_fov_h).tan()).max(half_height / (LOAD_FILL * half_fov_v).tan());
    (dist / extent).clamp(MIN_ORBIT_RADIUS_FRAC, MAX_ORBIT_RADIUS_FRAC)
}

/// The orbiting turntable camera at `angle` radians around `frame`, over an
/// `n³` grid (`n` is the renderer's world-to-grid scale, independent of what
/// the frame chooses to orbit).
///
/// Deterministic in `angle`, so a scripted profiling run (the viewer's
/// `--frames`) is reproducible regardless of frame rate.
#[must_use]
pub fn orbit_camera(angle: f32, frame: OrbitFrame, n: f32, w: u32, h: u32, k: u32) -> GpuCamera {
    let (eye, forward) = orbit_eye_forward(angle, frame);
    pack(eye, forward, w, h, n, k)
}

/// The orbit camera's eye position and forward direction at `angle` around
/// `frame`. Exposed so the free camera can be seeded from the current orbit
/// pose without a jump.
#[must_use]
pub fn orbit_eye_forward(angle: f32, frame: OrbitFrame) -> (Vec3, Vec3) {
    let radius = frame.extent * 1.6;
    let eye = frame.center
        + Vec3::new(
            angle.cos() * radius,
            frame.extent * 0.35,
            angle.sin() * radius,
        );
    (eye, (frame.center - eye).normalize())
}

/// An interactive turntable: the eye circles the frame's pivot on a sphere the
/// user drives — drag rotates `azimuth`/`elevation`, the wheel dollies the
/// `radius`. The pivot stays locked on the object, which is what makes it good
/// for painting/editing (unlike the fly camera, which drifts off the object).
///
/// All three fields are **scale-invariant**: `azimuth`/`elevation` are angles,
/// and `radius` is a *fraction of the frame's extent* — so a resolution change
/// (which rescales the whole scene in voxel space) needs no remap; the same
/// control re-frames onto the new object automatically.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitControl {
    /// Angle about world `+Y` (radians); `0` places the eye toward `+X`.
    pub azimuth: f32,
    /// Angle above the horizontal plane (radians), clamped shy of the poles.
    pub elevation: f32,
    /// Distance from the pivot as a fraction of [`OrbitFrame::extent`].
    pub radius: f32,
    /// World-space offset added to the frame's pivot (Alt-drag panning): the
    /// camera orbits `frame.center + pivot_offset`, so any point in space can
    /// be the focus. Reset by [`reset_pivot`](Self::reset_pivot).
    pub pivot_offset: Vec3,
    /// Angular velocity (azimuth, elevation), radians/second — the momentum
    /// spin that keeps going after a fling and decays under [`FRICTION`].
    vel: glam::Vec2,
    /// The pose at the previous [`integrate`](Self::integrate) — velocity is
    /// *observed* from pose motion, so `drag` keeps its immediate-apply feel.
    prev: glam::Vec2,
    /// A drag is in progress (between the first [`drag`](Self::drag) and
    /// [`release`](Self::release)): the hand owns the pose; a held-still
    /// pointer pins the object.
    held: bool,
    /// Pristine: the ambient display spin runs until the first grab.
    ambient: bool,
}

impl Default for OrbitControl {
    /// The default framing — matches the classic turntable pose (radius `1.6`,
    /// height `0.35` of the extent), spinning ambiently until first grabbed.
    fn default() -> Self {
        Self {
            azimuth: 0.0,
            elevation: 0.35_f32.atan2(1.6),
            radius: (1.6_f32 * 1.6 + 0.35 * 0.35).sqrt(),
            pivot_offset: Vec3::ZERO,
            vel: glam::Vec2::ZERO,
            prev: glam::Vec2::new(0.0, 0.35_f32.atan2(1.6)),
            held: false,
            ambient: true,
        }
    }
}

impl OrbitControl {
    /// This pose with the radius replaced — the load framing keeps the default
    /// angles + ambient spin but overrides the radius with a
    /// [`fit_orbit_radius`] value tuned to the object and viewport.
    #[must_use]
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// The eye position and unit forward for `frame`. Spherical coordinates
    /// about the pivot: radius scales with the frame's extent.
    #[must_use]
    pub fn eye_forward(self, frame: OrbitFrame) -> (Vec3, Vec3) {
        let pivot = frame.center + self.pivot_offset;
        let r = self.radius * frame.extent;
        let (se, ce) = self.elevation.sin_cos();
        let (sa, ca) = self.azimuth.sin_cos();
        let eye = pivot + Vec3::new(ce * ca * r, se * r, ce * sa * r);
        (eye, (pivot - eye).normalize())
    }

    /// Packs the current pose into a [`GpuCamera`] for a `w×h` viewport over an
    /// `n³` grid with `k` internal levels.
    #[must_use]
    pub fn to_gpu(self, frame: OrbitFrame, w: u32, h: u32, n: f32, k: u32) -> GpuCamera {
        let (eye, forward) = self.eye_forward(frame);
        pack(eye, forward, w, h, n, k)
    }

    /// Applies a pointer drag: horizontal motion spins azimuth, vertical motion
    /// tilts elevation (drag down lowers the eye), clamped shy of the poles.
    /// The first drag ends the ambient spin and takes ownership of the pose;
    /// momentum is observed at [`integrate`](Self::integrate) time, so this
    /// stays immediate.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        self.ambient = false;
        self.held = true;
        self.azimuth += dx * LOOK_SENSITIVITY;
        self.elevation =
            (self.elevation - dy * LOOK_SENSITIVITY).clamp(-MAX_ELEVATION, MAX_ELEVATION);
    }

    /// The drag pointer released: the pose's recent motion becomes the fling
    /// velocity, which [`integrate`](Self::integrate) glides out under
    /// friction.
    pub fn release(&mut self) {
        self.held = false;
    }

    /// Advances the hybrid dynamics by `dt` seconds: the pristine control
    /// spins ambiently; a held control observes its own motion (a still hand
    /// pins the object, a moving one charges the fling); a released control
    /// glides on its velocity, decaying under friction and stopping at the
    /// elevation poles.
    pub fn integrate(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        if self.ambient {
            self.azimuth += AMBIENT_RATE * dt;
            self.prev = glam::Vec2::new(self.azimuth, self.elevation);
            return;
        }
        if self.held {
            let pose = glam::Vec2::new(self.azimuth, self.elevation);
            let inst = (pose - self.prev) / dt;
            // Smoothed: jittery event timing doesn't spike the fling, and a
            // still frame drives the velocity toward zero (the pin).
            self.vel = self.vel.lerp(inst, 0.5);
            self.prev = pose;
            return;
        }
        self.azimuth += self.vel.x * dt;
        let e = self.elevation + self.vel.y * dt;
        if e >= MAX_ELEVATION || e <= -MAX_ELEVATION {
            self.elevation = e.clamp(-MAX_ELEVATION, MAX_ELEVATION);
            self.vel.y = 0.0; // the pole absorbs vertical momentum
        } else {
            self.elevation = e;
        }
        self.vel *= (-FRICTION * dt).exp();
        if self.vel.length_squared() < SPIN_EPSILON * SPIN_EPSILON {
            self.vel = glam::Vec2::ZERO;
        }
        self.prev = glam::Vec2::new(self.azimuth, self.elevation);
    }

    /// Pans the orbit pivot in the camera plane (Alt-drag): the scene follows
    /// the pointer, so any point in space can become the focus. Scaled by the
    /// current view distance — a zoomed-in pan is precise, a zoomed-out one
    /// covers ground. Ends the ambient spin like any grab.
    pub fn pan(&mut self, dx: f32, dy: f32, frame: OrbitFrame) {
        self.ambient = false;
        let (_, forward) = self.eye_forward(frame);
        let (_, right, up) = basis(forward);
        let scale = self.radius * frame.extent * PAN_SENSITIVITY;
        self.pivot_offset += (-right * dx + up * dy) * scale;
    }

    /// Recentres the orbit on the frame's own pivot (double-click).
    pub fn reset_pivot(&mut self) {
        self.pivot_offset = Vec3::ZERO;
    }

    /// Applies scroll-wheel notches as a zoom: positive notches (wheel up)
    /// dolly toward the pivot, clamped to the radius bounds.
    pub fn zoom(&mut self, notches: f32) {
        self.radius = (self.radius * ZOOM_PER_NOTCH.powf(-notches))
            .clamp(MIN_ORBIT_RADIUS_FRAC, MAX_ORBIT_RADIUS_FRAC);
    }

    /// Seeds the control from a world-space eye looking at `frame`'s pivot, so a
    /// mode switch *into* orbit continues the current view without a jump. A
    /// degenerate (pivot-coincident or zero-extent) input falls back to the
    /// default framing.
    #[must_use]
    pub fn from_view(eye: Vec3, frame: OrbitFrame) -> Self {
        let d = eye - frame.center;
        let dist = d.length();
        if dist <= f32::EPSILON || frame.extent <= 0.0 {
            return Self::default();
        }
        let azimuth = d.z.atan2(d.x);
        let elevation = (d.y / dist)
            .clamp(-1.0, 1.0)
            .asin()
            .clamp(-MAX_ELEVATION, MAX_ELEVATION);
        Self {
            azimuth,
            elevation,
            radius: (dist / frame.extent).clamp(MIN_ORBIT_RADIUS_FRAC, MAX_ORBIT_RADIUS_FRAC),
            pivot_offset: Vec3::ZERO,
            vel: glam::Vec2::ZERO,
            prev: glam::Vec2::new(azimuth, elevation),
            held: false,
            // Entered from another view deliberately: no ambient surprise.
            ambient: false,
        }
    }
}

/// An interactive free-fly camera: world position plus yaw/pitch look angles and
/// a movement speed (world units per second).
#[derive(Clone, Copy, Debug)]
pub struct FlyCamera {
    /// World-space eye position.
    pub eye: Vec3,
    /// Yaw (radians) about world `+Y`; `0` looks toward `+Z`.
    pub yaw: f32,
    /// Pitch (radians); positive looks up, clamped to ±`MAX_PITCH`.
    pub pitch: f32,
    /// Base movement speed in world units per second.
    pub speed: f32,
}

impl FlyCamera {
    /// Seeds a free camera from an eye position and forward direction (e.g. the
    /// current orbit pose), with a movement speed scaled to the grid size `n`.
    #[must_use]
    pub fn from_eye_forward(eye: Vec3, forward: Vec3, n: f32) -> Self {
        let forward = forward.normalize();
        Self {
            eye,
            yaw: forward.x.atan2(forward.z),
            pitch: forward.y.clamp(-1.0, 1.0).asin(),
            speed: (n * 0.6).max(1.0),
        }
    }

    /// Remaps this camera from the `old` scene's framing to the `new` one — a
    /// uniform scale about the two frame centres — so a rebuilt scene keeps the
    /// current view instead of snapping back to a default orbit. Voxel
    /// coordinates scale with the grid, so a resolution change moves the whole
    /// scene; this moves the camera with it (same relative position, distance,
    /// and look direction). Look angles are scale-invariant; only the eye
    /// position and the movement speed scale. A degenerate `old` extent (an
    /// empty prior scene) leaves the pose unscaled.
    #[must_use]
    pub fn reframed(self, old: OrbitFrame, new: OrbitFrame) -> Self {
        let scale = if old.extent > 0.0 {
            new.extent / old.extent
        } else {
            1.0
        };
        Self {
            eye: new.center + (self.eye - old.center) * scale,
            yaw: self.yaw,
            pitch: self.pitch,
            speed: (self.speed * scale).max(1.0),
        }
    }

    /// The unit forward direction implied by the current yaw/pitch.
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    /// Advances the camera by `dt` seconds under the current [`Input`]: applies
    /// mouse-look, scroll-to-speed, and `WASDQE` movement. Pure — `dt` and
    /// `input` are supplied by the event loop.
    pub fn apply(&mut self, dt: f32, input: &Input) {
        // Look: yaw follows horizontal motion, pitch follows vertical (inverted
        // so dragging up looks up), clamped to keep the basis well-defined.
        self.yaw += input.look_dx * LOOK_SENSITIVITY;
        self.pitch = (self.pitch - input.look_dy * LOOK_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);

        // Speed: each scroll notch scales the base speed geometrically.
        if input.scroll != 0.0 {
            self.speed = (self.speed * SCROLL_SPEED_FACTOR.powf(input.scroll)).clamp(0.05, 1.0e6);
        }

        // Movement: along the look basis, with world-up for vertical.
        let (forward, right, _up) = basis(self.forward());
        let axis = |pos: bool, neg: bool| f32::from(pos) - f32::from(neg);
        let dir = forward * axis(input.forward, input.back)
            + right * axis(input.right, input.left)
            + Vec3::Y * axis(input.up, input.down);
        if dir.length_squared() > 0.0 {
            let boost = if input.boost { BOOST_MULTIPLIER } else { 1.0 };
            self.eye += dir.normalize() * self.speed * boost * dt;
        }
    }

    /// Packs the current pose into a [`GpuCamera`] for a `w×h` viewport over an
    /// `n³` grid with `k` internal levels.
    #[must_use]
    pub fn to_gpu(self, w: u32, h: u32, n: f32, k: u32) -> GpuCamera {
        pack(self.eye, self.forward(), w, h, n, k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn forward_at_origin_angles_points_along_z() {
        let cam = FlyCamera {
            eye: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 1.0,
        };
        let f = cam.forward();
        assert!(
            approx(f.x, 0.0) && approx(f.y, 0.0) && approx(f.z, 1.0),
            "{f:?}"
        );
    }

    #[test]
    fn gpu_basis_is_orthonormal() {
        let cam = FlyCamera {
            eye: Vec3::new(10.0, 5.0, -3.0),
            yaw: 0.7,
            pitch: 0.3,
            speed: 1.0,
        };
        let g = cam.to_gpu(800, 600, 512.0, 3);
        let f = Vec3::from_array(g.forward);
        let r = Vec3::from_array(g.right);
        let u = Vec3::from_array(g.up);
        for v in [f, r, u] {
            assert!(approx(v.length(), 1.0), "not unit: {v:?}");
        }
        assert!(approx(f.dot(r), 0.0) && approx(f.dot(u), 0.0) && approx(r.dot(u), 0.0));
    }

    #[test]
    fn moving_forward_advances_along_forward() {
        let mut cam = FlyCamera {
            eye: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 10.0,
        };
        let f = cam.forward();
        let before = cam.eye;
        let input = Input {
            forward: true,
            ..Default::default()
        };
        cam.apply(0.5, &input);
        let moved = cam.eye - before;
        assert!(moved.dot(f) > 0.0, "should move along forward: {moved:?}");
        assert!(
            approx(moved.length(), 5.0),
            "10 u/s * 0.5 s = 5: {}",
            moved.length()
        );
    }

    #[test]
    fn pitch_clamps_to_avoid_gimbal() {
        let mut cam = FlyCamera {
            eye: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 1.0,
        };
        let input = Input {
            look_dy: -1.0e6, // slam the look way up
            ..Default::default()
        };
        cam.apply(0.016, &input);
        assert!(cam.pitch <= MAX_PITCH && cam.pitch >= -MAX_PITCH);
        assert!(approx(cam.pitch, MAX_PITCH));
    }

    #[test]
    fn from_eye_forward_round_trips_direction() {
        let eye = Vec3::new(1.0, 2.0, 3.0);
        let dir = Vec3::new(0.3, -0.6, 0.74).normalize();
        let cam = FlyCamera::from_eye_forward(eye, dir, 512.0);
        let f = cam.forward();
        assert!(
            approx(f.x, dir.x) && approx(f.y, dir.y) && approx(f.z, dir.z),
            "{f:?} vs {dir:?}"
        );
    }

    #[test]
    fn scroll_scales_speed() {
        let mut cam = FlyCamera {
            eye: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 100.0,
        };
        let input = Input {
            scroll: 1.0,
            ..Default::default()
        };
        cam.apply(0.016, &input);
        assert!(approx(cam.speed, 100.0 * SCROLL_SPEED_FACTOR));
    }

    #[test]
    fn grid_frame_reproduces_the_legacy_orbit_pose() {
        // Backward-compat pin: OrbitFrame::grid(n) must place the eye exactly
        // where the old n-only orbit did — pivot at the grid centre, radius
        // 1.6n, eye height 0.35n above it.
        let n = 512.0;
        let frame = OrbitFrame::grid(n);
        assert_eq!(frame.center, Vec3::splat(n * 0.5));
        assert!(approx(frame.extent, n));
        let (eye, forward) = orbit_eye_forward(0.0, frame);
        // angle 0: eye offset is (+radius, +height, 0) from the centre.
        assert!(approx(eye.x, n * 0.5 + n * 1.6), "{eye:?}");
        assert!(approx(eye.y, n * 0.5 + n * 0.35), "{eye:?}");
        assert!(approx(eye.z, n * 0.5), "{eye:?}");
        // The camera looks back at the pivot.
        let to_centre = (frame.center - eye).normalize();
        assert!(approx(forward.dot(to_centre), 1.0), "{forward:?}");
    }

    #[test]
    fn aabb_frame_pivots_on_the_box_centre_regardless_of_grid() {
        // A small box tucked in a corner of a large grid frames on itself: the
        // pivot is the box's voxel-centre midpoint and the eye stays at
        // 1.6 * (its own extent), not the grid's.
        let (min, max) = (Vec3::new(10.0, 20.0, 30.0), Vec3::new(18.0, 22.0, 34.0));
        let frame = OrbitFrame::aabb(min, max);
        // Inclusive corners → centre is the midpoint of the voxel *centres*.
        assert_eq!(frame.center, (min + max) * 0.5 + Vec3::splat(0.5));
        // Largest span is x: (18-10)+1 = 9 voxels.
        assert!(approx(frame.extent, 9.0), "{}", frame.extent);
        let (eye, forward) = orbit_eye_forward(0.0, frame);
        assert!(approx(
            (eye - frame.center).length(),
            (9.0f32 * 1.6).hypot(9.0 * 0.35)
        ));
        assert!(approx(forward.dot((frame.center - eye).normalize()), 1.0));
    }

    #[test]
    fn aabb_frame_floors_a_degenerate_box_to_a_finite_distance() {
        // A single voxel (min == max) has span 1 on every axis: the framing
        // distance is finite, not zero.
        let p = Vec3::new(9.0, 9.0, 9.0);
        let frame = OrbitFrame::aabb(p, p);
        assert!(approx(frame.extent, 1.0));
        let (eye, _) = orbit_eye_forward(1.0, frame);
        assert!(eye.is_finite() && (eye - frame.center).length() > 0.0);
    }

    #[test]
    fn for_bbox_frames_the_object_or_falls_back_to_the_grid() {
        // Some(box) → the object frame; None (empty scene) → the grid frame.
        let object = OrbitFrame::for_bbox(
            Some((VoxelCoord::new(10, 20, 30), VoxelCoord::new(18, 22, 34))),
            512.0,
        );
        assert_eq!(
            object,
            OrbitFrame::aabb(Vec3::new(10.0, 20.0, 30.0), Vec3::new(18.0, 22.0, 34.0)),
        );
        let empty = OrbitFrame::for_bbox(None, 512.0);
        assert_eq!(empty, OrbitFrame::grid(512.0));
    }

    #[test]
    fn fit_frames_a_cube_tighter_than_the_loose_legacy_pose() {
        // A compact cube should fill more of the frame than the fixed 1.638×
        // legacy radius, and stay within the interactive bounds.
        let cube = Some((VoxelCoord::new(0, 0, 0), VoxelCoord::new(99, 99, 99)));
        let r = fit_orbit_radius(cube, 16.0 / 9.0);
        assert!(
            r < OrbitControl::default().radius,
            "cube should fill more: {r}"
        );
        assert!((MIN_ORBIT_RADIUS_FRAC..=MAX_ORBIT_RADIUS_FRAC).contains(&r));
    }

    #[test]
    fn fit_frames_a_flat_wide_slab_by_its_width_not_its_max_axis() {
        // The case max-axis framing under-fills: a wide/flat slab should come in
        // close (its width binds) rather than float small at the legacy distance.
        let slab = Some((VoxelCoord::new(0, 0, 0), VoxelCoord::new(199, 9, 199)));
        let r = fit_orbit_radius(slab, 16.0 / 9.0);
        assert!(
            r < OrbitControl::default().radius,
            "slab should not float small: {r}"
        );
        assert!((MIN_ORBIT_RADIUS_FRAC..=MAX_ORBIT_RADIUS_FRAC).contains(&r));
    }

    #[test]
    fn fit_pulls_back_on_a_narrower_viewport() {
        // Less horizontal room (a more portrait viewport) frames a wide object
        // from further out.
        let wide = Some((VoxelCoord::new(0, 0, 0), VoxelCoord::new(199, 40, 60)));
        let wide_vp = fit_orbit_radius(wide, 21.0 / 9.0);
        let narrow_vp = fit_orbit_radius(wide, 4.0 / 3.0);
        assert!(
            narrow_vp >= wide_vp,
            "narrower frames further: {narrow_vp} vs {wide_vp}"
        );
    }

    #[test]
    fn fit_falls_back_to_the_default_pose_for_an_empty_scene() {
        assert!(approx(
            fit_orbit_radius(None, 1.6),
            OrbitControl::default().radius
        ));
    }

    #[test]
    fn orbit_angle_sweeps_a_circle_in_the_xz_plane_about_the_pivot() {
        let frame = OrbitFrame::aabb(Vec3::ZERO, Vec3::splat(7.0));
        let radius = frame.extent * 1.6;
        for angle in [0.0, 1.0, 2.5, std::f32::consts::PI] {
            let (eye, _) = orbit_eye_forward(angle, frame);
            let flat = Vec3::new(eye.x - frame.center.x, 0.0, eye.z - frame.center.z);
            assert!(approx(flat.length(), radius), "angle {angle}: {eye:?}");
            // Height is constant with angle.
            assert!(approx(eye.y - frame.center.y, frame.extent * 0.35));
        }
    }

    #[test]
    fn orbit_control_default_matches_the_turntable_framing() {
        // Switching rotate → orbit must not jump: OrbitControl::default at
        // azimuth 0 lands where the turntable's angle-0 pose does.
        let frame = OrbitFrame::aabb(Vec3::ZERO, Vec3::splat(99.0)); // extent 100
        let (turn_eye, _) = orbit_eye_forward(0.0, frame);
        let (orbit_eye, orbit_fwd) = OrbitControl::default().eye_forward(frame);
        assert!(
            approx(orbit_eye.x, turn_eye.x),
            "{orbit_eye:?} vs {turn_eye:?}"
        );
        assert!(
            approx(orbit_eye.y, turn_eye.y),
            "{orbit_eye:?} vs {turn_eye:?}"
        );
        assert!(
            approx(orbit_eye.z, turn_eye.z),
            "{orbit_eye:?} vs {turn_eye:?}"
        );
        // Forward points at the pivot.
        assert!(approx(
            orbit_fwd.dot((frame.center - orbit_eye).normalize()),
            1.0
        ));
    }

    #[test]
    fn orbit_drag_spins_azimuth_and_clamps_elevation_at_the_poles() {
        let mut c = OrbitControl::default();
        let az0 = c.azimuth;
        c.drag(100.0, 0.0);
        assert!(c.azimuth > az0, "horizontal drag spins azimuth");
        // Slam the elevation past the pole; it clamps, keeping the basis valid.
        c.drag(0.0, -100_000.0);
        assert!(c.elevation <= MAX_ELEVATION && c.elevation >= -MAX_ELEVATION);
        assert!(approx(c.elevation, MAX_ELEVATION));
        let frame = OrbitFrame::aabb(Vec3::ZERO, Vec3::splat(7.0));
        let g = c.to_gpu(frame, 800, 600, 512.0, 3);
        for v in [
            Vec3::from_array(g.forward),
            Vec3::from_array(g.right),
            Vec3::from_array(g.up),
        ] {
            assert!(
                approx(v.length(), 1.0),
                "basis stays orthonormal near the pole: {v:?}"
            );
        }
    }

    #[test]
    fn orbit_zoom_dollies_within_bounds() {
        let mut c = OrbitControl::default();
        let r0 = c.radius;
        c.zoom(1.0); // wheel up → closer
        assert!(c.radius < r0, "zoom in shrinks the radius");
        // Spin the wheel hard both ways; the radius stays in bounds.
        for _ in 0..200 {
            c.zoom(1.0);
        }
        assert!(
            approx(c.radius, MIN_ORBIT_RADIUS_FRAC),
            "clamps at the near bound"
        );
        for _ in 0..400 {
            c.zoom(-1.0);
        }
        assert!(
            approx(c.radius, MAX_ORBIT_RADIUS_FRAC),
            "clamps at the far bound"
        );
        // Radius is extent-relative: same control, different-extent frames.
        let near = OrbitFrame::aabb(Vec3::ZERO, Vec3::splat(9.0)); // extent 10
        let far = OrbitFrame::aabb(Vec3::ZERO, Vec3::splat(99.0)); // extent 100
        let (e_near, _) = c.eye_forward(near);
        let (e_far, _) = c.eye_forward(far);
        assert!(approx(
            (e_far - far.center).length(),
            (e_near - near.center).length() * 10.0
        ));
    }

    #[test]
    fn from_view_round_trips_an_eye_position() {
        // Seeding orbit from an arbitrary eye reproduces that eye (within the
        // clamps) — so switching into orbit continues the current view.
        let frame = OrbitFrame {
            center: Vec3::splat(64.0),
            extent: 100.0,
        };
        let eye = Vec3::new(64.0 + 80.0, 64.0 + 40.0, 64.0 - 30.0);
        let (back, _) = OrbitControl::from_view(eye, frame).eye_forward(frame);
        assert!(
            approx(back.x, eye.x) && approx(back.y, eye.y) && approx(back.z, eye.z),
            "{back:?}"
        );
    }

    #[test]
    fn from_view_falls_back_when_the_eye_sits_on_the_pivot() {
        let frame = OrbitFrame {
            center: Vec3::splat(10.0),
            extent: 50.0,
        };
        let c = OrbitControl::from_view(frame.center, frame);
        assert_eq!(c, OrbitControl::default());
    }

    #[test]
    fn reframed_is_identity_when_the_frame_is_unchanged() {
        let frame = OrbitFrame::aabb(Vec3::splat(10.0), Vec3::splat(20.0));
        let cam = FlyCamera {
            eye: Vec3::new(3.0, 40.0, -5.0),
            yaw: 1.2,
            pitch: -0.4,
            speed: 50.0,
        };
        let same = cam.reframed(frame, frame);
        assert!(approx(same.eye.x, cam.eye.x) && approx(same.eye.y, cam.eye.y));
        assert!(approx(same.eye.z, cam.eye.z));
        assert!(approx(same.speed, cam.speed));
        assert!(approx(same.yaw, cam.yaw) && approx(same.pitch, cam.pitch));
    }

    #[test]
    fn reframed_scales_position_and_speed_about_the_centres_keeping_look() {
        // A 4× resolution bump: the new frame is 4× the extent, its centre 4×
        // farther out. The camera's offset from the centre and its speed scale
        // 4×; its look angles are untouched.
        let old = OrbitFrame {
            center: Vec3::splat(64.0),
            extent: 100.0,
        };
        let new = OrbitFrame {
            center: Vec3::splat(256.0),
            extent: 400.0,
        };
        let cam = FlyCamera {
            eye: Vec3::new(64.0 + 30.0, 64.0 - 10.0, 64.0 + 200.0),
            yaw: 0.7,
            pitch: 0.3,
            speed: 60.0,
        };
        let out = cam.reframed(old, new);
        // eye = new_center + (old_eye - old_center) * 4
        assert!(approx(out.eye.x, 256.0 + 30.0 * 4.0), "{:?}", out.eye);
        assert!(approx(out.eye.y, 256.0 - 10.0 * 4.0), "{:?}", out.eye);
        assert!(approx(out.eye.z, 256.0 + 200.0 * 4.0), "{:?}", out.eye);
        assert!(approx(out.speed, 240.0));
        assert!(
            approx(out.yaw, 0.7) && approx(out.pitch, 0.3),
            "look preserved"
        );
    }

    #[test]
    fn reframed_leaves_the_pose_unscaled_when_the_old_frame_is_degenerate() {
        // An empty prior scene has a zero extent — no meaningful ratio, so the
        // pose is translated (centre shift) but not scaled, and never divides
        // by zero.
        let old = OrbitFrame {
            center: Vec3::ZERO,
            extent: 0.0,
        };
        let new = OrbitFrame {
            center: Vec3::splat(50.0),
            extent: 128.0,
        };
        let cam = FlyCamera {
            eye: Vec3::new(10.0, 20.0, 30.0),
            yaw: 0.0,
            pitch: 0.0,
            speed: 40.0,
        };
        let out = cam.reframed(old, new);
        assert!(out.eye.is_finite() && approx(out.speed, 40.0));
        assert!(approx(out.eye.x, 60.0), "{:?}", out.eye); // 50 + 10*1
    }

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn ambient_spin_advances_until_the_first_grab() {
        let mut c = OrbitControl::default();
        let az0 = c.azimuth;
        c.integrate(0.5);
        assert!(
            approx(c.azimuth, az0 + AMBIENT_RATE * 0.5),
            "pristine spins"
        );

        // The first drag takes ownership: no more ambient motion, ever.
        c.drag(1.0, 0.0);
        c.release();
        let az1 = c.azimuth;
        for _ in 0..120 {
            c.integrate(DT);
        }
        assert!(
            (c.azimuth - az1).abs() < 1e-3,
            "grabbed control does not auto-spin (drift {})",
            c.azimuth - az1
        );
    }

    #[test]
    fn a_still_hand_pins_the_object() {
        let mut c = OrbitControl::default();
        // Charge a fling by dragging steadily…
        for _ in 0..10 {
            c.drag(10.0, 0.0);
            c.integrate(DT);
        }
        // …then hold the pointer perfectly still for a few frames.
        for _ in 0..20 {
            c.integrate(DT);
        }
        c.release();
        let az = c.azimuth;
        for _ in 0..120 {
            c.integrate(DT);
        }
        assert!(
            (c.azimuth - az).abs() < 1e-3,
            "a held-still release must not fling (drift {})",
            c.azimuth - az
        );
    }

    #[test]
    fn release_commits_a_fling_that_friction_brings_to_rest() {
        let mut c = OrbitControl::default();
        for _ in 0..10 {
            c.drag(10.0, 0.0);
            c.integrate(DT);
        }
        c.release();
        let az_release = c.azimuth;
        c.integrate(DT);
        assert!(c.azimuth > az_release, "momentum carries past release");
        // Ten simulated seconds of friction: the glide must have settled.
        for _ in 0..600 {
            c.integrate(DT);
        }
        let az_rest = c.azimuth;
        c.integrate(DT);
        assert!(approx(c.azimuth, az_rest), "friction reaches full rest");
        assert!(az_rest > az_release, "the glide covered ground first");
    }

    #[test]
    fn the_elevation_pole_absorbs_vertical_momentum() {
        let mut c = OrbitControl::default();
        c.drag(0.0, -300.0); // jump near the top…
        for _ in 0..5 {
            c.drag(0.0, -10.0); // …then charge upward velocity
            c.integrate(DT);
        }
        c.release();
        for _ in 0..600 {
            c.integrate(DT);
            assert!(c.elevation <= MAX_ELEVATION + 1e-6, "never past the pole");
        }
        assert!(
            approx(c.elevation, MAX_ELEVATION),
            "the fling parks at the pole instead of bouncing ({})",
            c.elevation
        );
    }

    #[test]
    fn pan_shifts_the_pivot_in_the_camera_plane_and_reset_restores_it() {
        let frame = OrbitFrame::grid(64.0);
        let mut c = OrbitControl::default();
        let (_, forward) = c.eye_forward(frame);
        c.pan(30.0, -12.0, frame);
        assert!(c.pivot_offset.length() > 0.0, "pan moved the pivot");
        assert!(
            c.pivot_offset.normalize().dot(forward).abs() < 1e-4,
            "pan stays in the camera plane (no dolly component)"
        );
        // The whole view translates with the pivot: forward is unchanged.
        let (_, forward_after) = c.eye_forward(frame);
        assert!(forward_after.abs_diff_eq(forward, 1e-5));
        // Panning is a grab too: the ambient spin must not resume.
        let az = c.azimuth;
        c.integrate(0.5);
        assert!(approx(c.azimuth, az), "pan ended the ambient spin");

        c.reset_pivot();
        assert_eq!(c.pivot_offset, Vec3::ZERO);
    }

    #[test]
    fn from_view_enters_settled_with_no_ambient_spin() {
        let frame = OrbitFrame::grid(64.0);
        let c0 = OrbitControl::default();
        let (eye, _) = c0.eye_forward(frame);
        let mut c = OrbitControl::from_view(eye, frame);
        let az = c.azimuth;
        c.integrate(0.5);
        assert!(
            approx(c.azimuth, az),
            "a deliberately-entered orbit does not surprise-spin"
        );
    }
}
