//! Radiance cascades v2 — legacy implementation.
//!
//! REMOVE_WITH_V2 FILE: this entire directory tree
//! (`crates/wasm_renderer/src/gi/v2_legacy/`) is deleted at v3 completion.
//! See `docs/Resident Representation/radiance-cascades-v3-design.md` for the
//! migration plan and `radiance-cascades-symptoms.md` for the catalog of
//! v2 failure modes that motivated the rewrite.

pub mod backend;
pub mod cascade_build;

pub use cascade_build::CascadeBuildPass;
