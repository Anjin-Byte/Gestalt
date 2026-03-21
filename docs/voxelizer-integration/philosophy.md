# The Canonical Store Principle

**Type:** philosophy
**Status:** current
**Date:** 2026-02-22

---

## The Principle

The greedy mesh chunk manager is the **canonical store for voxel space**.

It defines the contract. Data reaches it from any source — procedural noise,
manual player edits, rasterized triangle meshes — all through the same interface:
a set of `(world_position, MaterialId)` writes. The chunk manager does not know
or care about the source.

This is not an implementation detail. It is the architectural decision that
determines every other decision in this section.

---

## What It Means for the Voxelizer

The GPU voxelizer was built before the chunk manager existed. Its output format
was designed to support its own renderer, not to feed a downstream store. It
emits sparse bricks, bitpacked occupancy arrays, and per-voxel triangle indices.
None of those are the chunk manager's native currency.

Now that the chunk manager is the target, **the voxelizer must adapt to meet it —
not the other way around.** The question is not "how do we make the chunk manager
accept sparse brick output?" The question is "what form of output would cost the
chunk manager the least to ingest?"

---

## The Answer: GPU-Compact Output

The chunk manager's ingestion cost has two dominant parts:

1. **Finding occupied voxels.** If the voxelizer returns all 16 million slots of a
   256³ grid (most of them empty) and the CPU must scan every bit to find the 1-2
   million occupied voxels, the CPU has done work proportional to *grid volume* —
   not to the actual data. This is waste.

2. **Grouping by chunk.** The chunk manager organizes the world into 62³-voxel
   chunks. Every voxel write must be directed to the right chunk.

The GPU already solves the first problem: the existing compact pass scans the
occupancy bitfield on the GPU and outputs *only occupied voxels* via an atomic
counter. The CPU occupancy scan is not necessary — it has always been duplicate
work that the GPU had already done.

The GPU can also solve material resolution: reading a `material_table` buffer and
writing `MaterialId` directly, rather than writing a triangle index that the CPU
must later translate.

With those two steps on the GPU, the CPU is left with only what it must do:
group entries by chunk coordinate and write them into the chunk manager.

This is Architecture B. It is the design this section documents.

---

## What This Section Does Not Re-Open

The archive folder contains seven earlier documents that describe a different
integration approach (Architecture A): CPU-side occupancy scanning, CPU material
lookup, and an intermediate wire format (`VoxelizerChunkDeltaBatch`) between the
voxelizer and chunk manager.

That work identified the right destination — the chunk manager — and correctly
characterized the coordinate frames and material requirements. It is preserved in
the archive because it contains useful analysis. But its implementation plans are
superseded. The CPU occupancy scan is not done. The intermediate wire format is
not built. The attribution step is on the GPU.

The canonical store principle is the reason. If the chunk manager defines the
contract, the right question is always: *how close to that contract can the GPU
get?* The answer is: very close. Close enough that the CPU needs only one clean
grouping pass.

---

## The Contract in Practice

From the chunk manager's perspective, every data source — noise, voxelizer, editor
— calls the same interface. The voxelizer's job is to become indistinguishable from
any other producer. After voxelization, the chunk manager sees a flat list of
`(global_vx, global_vy, global_vz, MaterialId)` entries. It does not know they
came from a GPU triangle intersection computation. It does not know what mesh they
represent. It groups them by chunk coordinate, writes them into its palette-based
storage, marks the affected chunks dirty, and lets the greedy mesher run.

That invisibility — the voxelizer disappearing into the same API that everything
else uses — is what this integration achieves.
