**Technical Write-Up**
*Deadlock, “This Tiny Algorithm Can Render BILLIONS of Voxels in Real Time”*  
Published on **September 5, 2025**

This video is a compact engineering walkthrough of a direct voxel renderer built around **fast voxel traversal in a dense uniform grid**. The central point is not that voxel ray tracing is new, but that a very small traversal algorithm, when mapped correctly to GPU execution and paired with better memory layout, is enough to make dense voxel rendering practical at surprisingly large scales.

## System Overview

The renderer takes a dense 3D voxel volume and renders it directly, without generating triangle meshes. The pipeline is:

1. Store scene occupancy in a voxel grid.
2. For each screen pixel, generate a camera ray.
3. Intersect the ray with the grid bounding box.
4. Traverse the grid voxel-by-voxel until:
   - a solid voxel is found, or
   - the ray exits the volume.
5. Shade based on hit position, hit face, and distance.

The implementation shown in the video uses **Rust + Vulkan**, with rays cast in a **compute shader** rather than via rasterization.

## Core Algorithm

The traversal method is the **Amanatides and Woo** algorithm from 1987: a DDA-like ray march through a regular grid. Its advantage over naive fixed-step marching is that it only visits voxels the ray actually intersects.

### Why naive stepping fails

A fixed-step ray marcher has two bad options:

- Large step size:
  - can skip thin occupied voxels
  - produces incorrect misses
- Small step size:
  - revisits the same voxel many times
  - burns ALU and memory bandwidth

The grid structure gives you a better option: step exactly from one cell boundary to the next.

### Entry phase

Before traversal, the renderer computes the ray’s intersection against the voxel volume’s AABB using the **slab method**. If the ray misses the AABB, traversal is skipped entirely.

If it hits, the algorithm computes:

- `t_enter`: first parametric hit with the box
- `t_exit`: last parametric hit with the box
- `entry_point = origin + t_enter * dir`

The first voxel is then:

```text
voxel = floor(entry_point)
```

with clamping or bounds checks to handle floating-point edge cases near the box boundary.

### Traversal phase

For each axis, the renderer tracks:

- `step`: whether voxel coordinates increase or decrease on that axis
- `tDelta`: how far in parametric `t` the ray must move to cross one full voxel on that axis
- `tMax`: the `t` value of the next voxel boundary crossing on that axis

Typical setup:

```text
step.x   = sign(dir.x)
tDelta.x = abs(1 / dir.x)
tMax.x   = t at next x boundary
```

Same for `y` and `z`.

At each iteration:

1. Compare `tMax.x`, `tMax.y`, `tMax.z`
2. Step along the axis with the smallest value
3. Add the corresponding `tDelta` to that axis’s `tMax`
4. Test the new voxel for occupancy

Minimal form:

```text
while voxel in bounds:
    if voxel is solid:
        return hit

    if tMax.x < tMax.y and tMax.x < tMax.z:
        voxel.x += step.x
        tMax.x += tDelta.x
        face = x
    else if tMax.y < tMax.z:
        voxel.y += step.y
        tMax.y += tDelta.y
        face = y
    else:
        voxel.z += step.z
        tMax.z += tDelta.z
        face = z
```

This is the critical engineering property: traversal cost scales with **voxels crossed**, not **grid resolution as a whole**.

## Why the Algorithm Is Fast

The original 1987 paper describes the neighbor-to-neighbor step as requiring only **two floating-point comparisons and one floating-point addition** per voxel transition. That matters because a dense voxel renderer is fundamentally dominated by repeated traversal.

The key benefits are:

- No oversampling within a voxel
- No skipped cells
- Branch structure is simple
- State per ray is small
- GPU threads can run one ray per pixel with a regular loop body

The video’s implementation also precomputes `1 / dir` and uses it repeatedly, replacing divisions with multiplies where possible.

## GPU Mapping

This renderer is a natural fit for a compute pass:

- One invocation per pixel
- Read-only voxel data
- Independent primary rays
- Simple output buffer write

The GPU is not being used as a triangle rasterizer here. Instead, it is running a custom visibility algorithm over a structured spatial dataset.

That said, traversal is only half the problem. On the GPU, **memory locality** often dominates performance once arithmetic is cheap enough. The video’s most important engineering lesson is that the traversal math stays mostly constant while performance changes dramatically as the storage layout improves.

## The Actual Performance Story: Memory Layout

The video reports the following at **2048^3** grid resolution and **1000 x 1000** output resolution:

- Flat dense layout: about **12 FPS**
- Z-order / Morton layout: about **102 FPS**
- 3D texture storage: about **121 FPS**

Those numbers imply the traversal algorithm itself was already viable, but memory access was the bottleneck.

### Flat 1D layout

A simple flattening function maps `(x, y, z)` into a linear array. This is easy to implement, but poor for traversal locality. Neighboring voxels in 3D are often far apart in memory depending on which axis changes.

That leads to:

- more cache misses
- worse coherence between nearby rays
- more pressure on memory bandwidth

### Z-order / Morton layout

The first major optimization is switching from row-major flattening to **Z-order indexing**.

Why it helps:

- nearby voxels in 3D are more likely to be nearby in memory
- a ray stepping into an adjacent voxel has a better chance of hitting cached data
- neighboring screen rays often sample overlapping regions, improving cache reuse across threads

The algorithm does not change. Only the addressing scheme does. Yet this is where most of the speedup comes from.

### 3D texture storage

The final optimization is storing voxel data in a **3D image / texture** instead of a custom linear buffer.

This is the GPU-native version of the same idea:

- spatial locality is handled by hardware-oriented layouts
- caches are tuned for texture-like access patterns
- address translation and locality handling are better than a hand-rolled flat array in many cases

The video’s result is a further increase from roughly 102 FPS to 121 FPS. The important conclusion is that **the 10x result comes primarily from data layout, not a new traversal algorithm**.

## What the Renderer Produces

The traversal only tells you the first solid voxel hit, but that is enough to derive:

- hit distance
- which face was entered
- surface coordinates on that face
- basic shading inputs

That supports:

- flat shading
- texturing
- normal derivation from face direction
- secondary-ray launch points for more advanced ray tracing

So even though the scene representation is just occupancy voxels, the output can support a standard lighting pipeline.

## Engineering Constraints and Tradeoffs

The renderer is fast, but it is not a complete solution for arbitrary-scale scenes.

### Strengths

- Very simple traversal logic
- Exact cell visitation in a regular grid
- Works directly on voxel data
- Highly parallel primary-ray workload
- No meshing step

### Weaknesses

- Dense grids are memory heavy
- Empty space still consumes storage
- Rays still walk empty cells one-by-one
- Performance depends heavily on memory layout and occupancy distribution

A **2048^3** dense grid is enormous in address space. Even if occupancy is tightly packed, the representation is still costly. This is the main reason sparse structures become attractive.

## Natural Next Step: Sparse Hierarchies

The video points toward **sparse voxel octrees** as the next evolution.

That direction addresses both major limitations:

- storage cost drops because empty regions are collapsed
- traversal can skip large empty spaces in fewer steps

In other words:

- uniform-grid traversal solves the “how do I walk voxels exactly?” problem
- sparse hierarchies solve the “how do I avoid paying for empty space?” problem

That is the right framing. The algorithm in this video is best understood as the dense-grid baseline that makes later sparse acceleration structures easier to reason about.

## Assessment

From an engineering perspective, the video is strong because it separates three concerns cleanly:

1. **Correctness**
   - derive exact traversal over a grid
2. **Execution model**
   - map one-ray-per-pixel work to a compute shader
3. **Performance**
   - fix memory locality before replacing the algorithm

That last point is the most valuable. It is common to look for a “better algorithm” when the real issue is that the current algorithm is starved by poor data access patterns. This video demonstrates the opposite: the classic algorithm was already good enough; the large performance gains came from storing the same voxel data in layouts the GPU could use efficiently.

## Sources

- YouTube video: https://www.youtube.com/watch?v=ztkh1r1ioZo
- Transcript mirror: https://ytscribe.com/ko/v/ztkh1r1ioZo
- Amanatides and Woo, *A Fast Voxel Traversal Algorithm for Ray Tracing* (1987): https://diglib.eg.org/items/60c72224-00f3-416d-9952-ee41e8c408da/full

If you want, I can turn this into:
1. a formal blog post with headings and intro/conclusion polish
2. a paper-style write-up with equations
3. a project README section aimed at implementers