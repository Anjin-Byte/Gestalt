//! The observable phases of the web engine's long-running jobs, reported on the
//! single progress channel `(phase, done, total)` (`total = 0` marks an
//! indeterminate phase — a phase whose underlying work reports no unit count).
//!
//! **One enum across every job** — mesh voxelization, fixture/noise builds,
//! `.vox`/`.cvox` decode, and `.vox`/`.cvox` export — so the shell renders one
//! uniform segmented bar rather than a bespoke progress channel per operation
//! (`docs/design/web-frontend-api.md` §5, stage 8). Each operation emits its
//! own *ordered subset* of these phases; the shell owns the per-operation
//! order and the human wording. Keys are the stable wire form, mirrored by the
//! shell's `ProgressPhase` type — the kernel names *what* is happening; the
//! shell formats it. (The shell's type carries one extra key this enum never
//! emits: `install`, the render-worker scene upload, which the shell reports
//! itself — that step crosses a different worker whose protocol carries no
//! counts.)

/// A phase of some web job. See the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Decoding container bytes into triangles (mesh) or voxels (`.vox`/`.cvox`)
    /// — indeterminate, the parsers expose no unit of work.
    Parse,
    /// GPU surface voxelization, one unit per brick chunk (mesh).
    Voxelize,
    /// Per-chunk owner→material compaction, one unit per chunk (mesh).
    Compact,
    /// MASK alpha-cutout, one unit per compacted voxel (mesh, truecolor).
    Cutout,
    /// Tree + structure assembly. Metered per binned voxel where the builder
    /// reports it (mesh); indeterminate otherwise (fixture/decode assembly runs
    /// inside opaque core calls).
    Assemble,
    /// The per-voxel truecolor bake, one unit per occupied voxel (mesh).
    ColorBake,
    /// Fixture occupancy — a CPU field scan or the GPU noise generator.
    /// Indeterminate: neither reports intermediate counts.
    Generate,
    /// Walking every occupied voxel to gather it for export, one unit per leaf
    /// brick — a loop the web layer owns, so it carries real counts.
    Gather,
    /// Encoding the gathered voxels into the export format bytes. Indeterminate:
    /// the format writers live in the voxelizer crate and report no counts.
    Write,
    /// Serializing the built scene into the cross-worker transfer blob —
    /// metered by bytes written (the layout arithmetic knows the exact total
    /// up front). Every scene-producing job ends with this.
    Pack,
}

impl Phase {
    /// The stable wire key (mirrored by the shell's `ProgressPhase` type).
    pub(crate) fn key(self) -> &'static str {
        match self {
            Phase::Parse => "parse",
            Phase::Voxelize => "voxelize",
            Phase::Compact => "compact",
            Phase::Cutout => "cutout",
            Phase::Assemble => "assemble",
            Phase::ColorBake => "colorBake",
            Phase::Generate => "generate",
            Phase::Gather => "gather",
            Phase::Write => "write",
            Phase::Pack => "pack",
        }
    }
}
