//! Fixture-name → structure building for the web engine's control plane.
//!
//! Two entry points: [`build_fixture`] is the synchronous CPU path (the render
//! engine's initial scene), and [`build_fixture_gpu`] additionally covers the
//! noise fixtures (`perlin`, `caves`) via the async GPU generator — the IO
//! worker's path (`docs/design/web-frontend-api.md` §5, stage 5).

use voxel_core::fixtures::{Checkerboard, Dust, Gyroid, NoiseField, OctantFractal, WireLattice};
use voxel_core::{Resolution, SchoolBBuffer, SparseTree};
use voxel_gpu::GpuContext;

use crate::phases::Phase;

/// Why a fixture name could not be built into a scene.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    /// The name matches no fixture this front end knows.
    #[error(
        "unknown fixture `{0}` (expected gyroid, sierpinski, cantor, checkerboard, wire-lattice, or dust)"
    )]
    Unknown(String),
    /// The name is a noise fixture, which only the GPU path
    /// ([`build_fixture_gpu`]) can build.
    #[error("fixture `{0}` needs the GPU noise generator (use the IO worker's fixture path)")]
    NeedsGpuNoise(String),
    /// The GPU generator failed (device limit or readback failure).
    #[error("noise generation: {0}")]
    Noise(String),
    /// A CPU fixture at a grid too large for the single-threaded wasm scan.
    #[error(
        "fixture `{0}` scans every voxel on the CPU — above 512³ use the GPU \
         generators (perlin, caves) or import a mesh"
    )]
    CpuFixtureTooLarge(String),
}

/// Largest grid the single-threaded CPU occupancy scan builds in tolerable
/// time on the worker (a 2048³ scan is ~8.6G field evaluations — a minute or
/// more of wasm time; the GPU generators cover that regime).
const MAX_CPU_FIXTURE_RES: u32 = 512;

/// Builds the named CPU fixture at `resolution` into a tree plus its School-B
/// buffer, ready for [`voxel_gpu::GpuRenderer::new`].
///
/// Reports two indeterminate phases — [`Generate`](Phase::Generate) (the
/// `O(n³)` occupancy scan) then [`Assemble`](Phase::Assemble) (tree +
/// structure) — so the shell's bar shows liveness through the CPU scan, which
/// at 512³ is a real wait. Neither underlying call reports unit counts, so the
/// phases are honestly indeterminate (a pulsing segment). Pass a no-op sink
/// (`&mut |_, _, _| {}`) where progress is not observed.
pub(crate) fn build_fixture(
    name: &str,
    resolution: Resolution,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<(SparseTree, SchoolBBuffer), FixtureError> {
    if resolution.voxels_per_axis() > MAX_CPU_FIXTURE_RES {
        return Err(FixtureError::CpuFixtureTooLarge(name.to_string()));
    }
    on_progress(Phase::Generate, 0, 0);
    let tree = match name {
        "sierpinski" => SparseTree::build(&OctantFractal::sierpinski_tetrahedron(resolution)),
        "cantor" => SparseTree::build(&OctantFractal::cantor_dust(resolution)),
        "checkerboard" => SparseTree::build(&Checkerboard { resolution }),
        "wire-lattice" => SparseTree::build(&WireLattice::new(resolution)),
        "gyroid" => SparseTree::build(&Gyroid::new(resolution)),
        "dust" => SparseTree::build(&Dust::new(resolution)),
        "perlin" | "caves" => return Err(FixtureError::NeedsGpuNoise(name.to_string())),
        other => return Err(FixtureError::Unknown(other.to_string())),
    };
    on_progress(Phase::Assemble, 0, 0);
    let structure = SchoolBBuffer::from_sparse(&tree);
    Ok((tree, structure))
}

/// [`build_fixture`] plus the GPU-generated noise fixtures: `perlin` and
/// `caves` evaluate their occupancy on `ctx`'s device (the brick-compaction
/// generator — no dense readback), everything else takes the CPU path. Reports
/// the same [`Generate`](Phase::Generate) → [`Assemble`](Phase::Assemble)
/// phase pair as [`build_fixture`].
pub(crate) async fn build_fixture_gpu(
    ctx: &GpuContext,
    name: &str,
    resolution: Resolution,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<(SparseTree, SchoolBBuffer), FixtureError> {
    let field = match name {
        "perlin" => NoiseField::perlin(resolution),
        "caves" => NoiseField::caves(resolution),
        other => return build_fixture(other, resolution, on_progress),
    };
    on_progress(Phase::Generate, 0, 0);
    let tree = voxel_gpu::generate_noise_tree_async(ctx, &field)
        .await
        .map_err(|e| FixtureError::Noise(e.to_string()))?;
    on_progress(Phase::Assemble, 0, 0);
    let structure = SchoolBBuffer::from_sparse(&tree);
    Ok((tree, structure))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A progress sink that observes nothing.
    fn noop(_: Phase, _: u64, _: u64) {}

    #[test]
    fn builds_a_cpu_fixture() {
        let res = Resolution::new(32).expect("32 = 8·4 is a valid resolution");
        let (tree, structure) =
            build_fixture("wire-lattice", res, &mut noop).expect("cpu fixture builds");
        assert!(tree.leaf_count() > 0);
        assert_eq!(structure.node_count(), tree.node_count());
    }

    #[test]
    fn cpu_fixture_reports_generate_then_assemble() {
        let res = Resolution::new(32).expect("valid resolution");
        let mut phases = Vec::new();
        build_fixture("dust", res, &mut |p, _, _| phases.push(p)).expect("builds");
        assert_eq!(phases, vec![Phase::Generate, Phase::Assemble]);
    }

    #[test]
    fn a_failed_build_reports_no_assemble() {
        // An unknown name at a valid resolution enters generate, then errors
        // out of the match — assemble must not fire when no tree was built.
        let res = Resolution::new(32).expect("valid resolution");
        let mut phases = Vec::new();
        let _ = build_fixture("nope", res, &mut |p, _, _| phases.push(p));
        assert_eq!(
            phases,
            vec![Phase::Generate],
            "generate only; no assemble on failure"
        );
    }

    #[test]
    fn cpu_fixtures_above_512_are_typed_errors() {
        let res = Resolution::new(2048).expect("2048 = 8·4^4 is legal");
        assert!(matches!(
            build_fixture("wire-lattice", res, &mut noop),
            Err(FixtureError::CpuFixtureTooLarge(_))
        ));
    }

    #[test]
    fn noise_fixtures_are_typed_errors_not_panics() {
        let res = Resolution::new(32).expect("valid resolution");
        assert!(matches!(
            build_fixture("caves", res, &mut noop),
            Err(FixtureError::NeedsGpuNoise(_))
        ));
        assert!(matches!(
            build_fixture("nope", res, &mut noop),
            Err(FixtureError::Unknown(_))
        ));
    }
}
