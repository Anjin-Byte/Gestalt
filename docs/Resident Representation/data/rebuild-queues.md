# Rebuild Queues

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Control Plane — compact work lists for rebuild scheduling.

> Compact arrays of slot indices consumed by rebuild passes. Produced by the compaction pass from stale bitsets.

---

## Identity

- **Buffer names:** `mesh_rebuild_queue`, `summary_rebuild_queue`
- **WGSL type:** `array<u32>` (slot indices) + `queue_counts: array<atomic<u32>>` (one atomic counter per queue)
- **GPU usage:** `STORAGE`
- **Binding:** control-plane group, written by compaction pass, read by R-1 (mesh rebuild) and I-3 (summary rebuild)

---

## Layout

Each queue is a flat `array<u32>` of slot indices, with maximum capacity `MAX_SLOTS`.

```
mesh_rebuild_queue:    array<u32, MAX_SLOTS>
summary_rebuild_queue: array<u32, MAX_SLOTS>
queue_counts:          array<atomic<u32>, 2>
                       // queue_counts[0] = mesh queue length
                       // queue_counts[1] = summary queue length
```

Total size per queue: `MAX_SLOTS * 4` bytes.
Total size for `queue_counts`: `8` bytes.

### Compaction Pass

The compaction pass scans the corresponding stale bitset and appends slot indices to the queue using atomic increment on the counter:

```wgsl
// For each slot S where stale_mesh[S >> 5] has bit (S & 31) set:
let idx = atomicAdd(&queue_counts[0], 1u);
mesh_rebuild_queue[idx] = S;
```

The resulting queue is unordered — slot indices appear in whatever order threads complete.

### Consumption

Rebuild passes consume entries `[0 .. min(queue_counts[Q], frame_budget)]`. The CPU (Stage 2) or the indirect dispatch args (Stage 3) determine how many entries to process per frame.

```
R-1 consumes mesh_rebuild_queue[0..N]    where N = min(queue_counts[0], mesh_budget)
I-3 consumes summary_rebuild_queue[0..M] where M = min(queue_counts[1], summary_budget)
```

---

## UNDERSPECIFIED

**What happens to unconsumed entries?**

If `queue_counts[Q] > frame_budget`, entries beyond the budget are not processed this frame. Two strategies are possible:

1. **Re-compact each frame:** Queue counters are reset to zero at the start of each frame. The compaction pass re-scans the (still-set) stale bits and repopulates the queues. Unconsumed entries are rediscovered because their stale bits were never cleared (the rebuild pass did not run for them).

2. **Persistent queues with tail pointer:** A `queue_consumed` counter tracks how many entries were processed. Next frame, consumption starts at `queue_consumed` rather than 0. New stale entries are appended after `queue_counts`.

Strategy 1 is simpler and correct: stale bits are the source of truth, and re-compaction is cheap (one compute dispatch over `ceil(MAX_SLOTS/32)` words). Strategy 2 avoids redundant compaction but introduces a second counter and requires careful handling of entries that become un-stale (eviction) between frames.

**Recommendation:** Start with Strategy 1 (re-compact each frame). The compaction cost is negligible relative to rebuild work.

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| QUE-1 | Every entry in `mesh_rebuild_queue[0..queue_counts[0]]` is a valid slot index in `[0, MAX_SLOTS)` | Compaction pass only appends slots with set stale bits |
| QUE-2 | Every entry in `summary_rebuild_queue[0..queue_counts[1]]` is a valid slot index in `[0, MAX_SLOTS)` | Compaction pass only appends slots with set stale bits |
| QUE-3 | `queue_counts[Q] <= MAX_SLOTS` for all Q | Stale bitset has at most MAX_SLOTS set bits |
| QUE-4 | No duplicate slot indices within a single queue (within one compaction pass) | Each stale bit is scanned exactly once per compaction |
| QUE-5 | Queue contents are undefined outside the range `[0, queue_counts[Q])` | Only the counted region is valid |
| QUE-6 | `queue_counts` are reset to zero before each compaction pass | Compaction pass precondition (Strategy 1) |
| QUE-7 | A rebuild pass must not read beyond `min(queue_counts[Q], frame_budget)` | Budget enforcement by CPU (Stage 2) or indirect args (Stage 3) |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| Each queue entry | `0 .. MAX_SLOTS-1` | Slot index |
| `queue_counts[Q]` | `0 .. MAX_SLOTS` | Atomic counter; 0 = no work |

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Compaction pass | After propagation pass, before rebuild passes | Queue entries + queue_counts via atomic increment |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Mesh rebuild (R-1) | Per-frame render | Reads `mesh_rebuild_queue[0..N]` where N is budgeted count |
| Summary rebuild (I-3) | Per-frame or on dirty | Reads `summary_rebuild_queue[0..M]` where M is budgeted count |
| CPU (Stage 2 only) | Async readback | Reads `queue_counts` to decide per-frame budget |
| Indirect dispatch args (Stage 3) | After compaction | Reads `queue_counts` to write `indirect_dispatch_args` |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Compaction correctness:** Set known stale bits, run compaction reference, verify queue contains exactly the set slot indices.
2. **Counter accuracy:** Verify `queue_counts[Q]` equals the popcount of the corresponding stale bitset.
3. **No duplicates:** After compaction, verify all queue entries are unique.
4. **Empty bitset:** All stale bits clear results in `queue_counts[Q] == 0`.
5. **Full bitset:** All stale bits set results in `queue_counts[Q] == MAX_SLOTS`.

### Property tests (Rust, randomized)

6. **Roundtrip:** Generate random stale bitset, compact, verify queue entries are exactly the set of set-bit slot indices (order-independent).
7. **Budget clamp:** Verify rebuild pass reads at most `frame_budget` entries regardless of queue length.

### GPU validation (WGSL compute)

8. **Readback test:** Write known stale pattern from CPU, dispatch compaction, readback queues and counts, verify against CPU reference.
9. **Atomic correctness:** Dispatch compaction with high slot count, verify no lost or duplicate entries in readback.
10. **Reset test:** Verify queue_counts are zero after reset and before compaction dispatch.
