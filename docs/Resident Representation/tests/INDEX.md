# Consistency Test Specifications

**Type:** reference
**Status:** current
**Date:** 2026-03-22

> Tests that prove logical consistency across data structures and pipeline stages before GPU implementation.

Each document answers: Does the output of stage N satisfy the preconditions of stage N+1? Do invariants hold across the full pipeline?

---

## Test Categories

### 1. Data Structure Invariant Tests
Prove each structure's invariants hold in isolation.

| Test suite | Doc | What it proves |
|---|---|---|
| Occupancy invariants | [occupancy-invariants](occupancy-invariants.md) | Bit layout, column addressing, boundary padding |
| Chunk pool invariants | [pool-invariants](pool-invariants.md) | Slot allocation, no double-alloc, version monotonicity |
| Material consistency | [material-consistency](material-consistency.md) | Palette indices in range, global table lookups valid |

### 2. Stage Contract Tests
Prove each stage's postconditions satisfy the next stage's preconditions.

| Test suite | Doc | What it proves |
|---|---|---|
| Ingest chain | [ingest-chain](ingest-chain.md) | I-1 → I-2 → I-3: voxelization output is valid pool input, summary derived correctly |
| Mesh → Depth | [mesh-to-depth](mesh-to-depth.md) | R-1 output is valid R-2 input: draw metadata, vertex/index buffers |
| Depth → Cull | [depth-to-cull](depth-to-cull.md) | R-2 → R-3 → R-4: depth texture → Hi-Z → valid cull decisions |
| Cull → Color | [cull-to-color](cull-to-color.md) | R-4 indirect args are valid R-5 draw calls |
| Color → Cascade | [color-to-cascade](color-to-cascade.md) | R-5 depth+color are valid R-6 inputs |

### 3. Cross-Cutting Property Tests
Prove system-wide properties hold under randomized inputs.

| Test suite | Doc | What it proves |
|---|---|---|
| Edit roundtrip | [edit-roundtrip](edit-roundtrip.md) | Edit → propagate → rebuild → render produces correct visual |
| Pool lifecycle | [pool-lifecycle](pool-lifecycle.md) | Load → render → edit → unload cycle leaves pool in valid state |
| Full pipeline | [full-pipeline](full-pipeline.md) | Randomized scene → ingest → N frames → no invariant violations |
