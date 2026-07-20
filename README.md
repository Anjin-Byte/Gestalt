<div align="center">

# Gestalt

**Sparse, MIP-mapped voxel rendering in Rust + wgpu.**

A stackless hierarchical-DDA traversal on the GPU, held bit-exact against a
CPU reference — the same core running natively and in the browser.

<p>
  <img src="https://img.shields.io/badge/Rust-2024%20edition-1e293b?style=flat-square&logo=rust&logoColor=white" alt="Rust 2024 edition" />
  <img src="https://img.shields.io/badge/Renderer-wgpu-1e293b?style=flat-square" alt="wgpu" />
  <img src="https://img.shields.io/badge/Web-WebGPU%20%2B%20WASM-1e293b?style=flat-square&logo=webassembly&logoColor=white" alt="WebGPU + WASM" />
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-1e293b?style=flat-square" alt="MIT OR Apache-2.0" />
</p>

</div>

---

Gestalt turns triangle meshes into a sparse voxel structure and ray-traverses it
on the GPU. A mesh is conservatively voxelized into a Morton-ordered brick
hierarchy, then walked by a stackless HDDA compute kernel — and every GPU path is
diffed against a pure CPU oracle, so the fast path is not just *fast* but provably
*correct*. One core drives both a native viewer and a WebGPU/WASM shell.

## Architecture

The dependency graph runs strictly **inward** toward a pure, GPU- and IO-free
`voxel-core`; everything effectful — devices, windows, mesh IO — lives at the edges.

| Crate | Role |
| --- | --- |
| **voxel-core** | Pure domain core: the sparse MIP structure, the reference traversal oracle, and the GPU buffer contract. No GPU, no IO. |
| **voxel-gpu** | wgpu adapter: the stackless HDDA compute kernel, held bit-exact against `voxel-core`. |
| **voxelizer** | Conservative surface voxelizer (glTF · OBJ · STL) — a GPU compute path diffed against a CPU SAT oracle. |
| **voxel-brush** | Deterministic sculpt / paint kernels: fields in, voxel operations out. |
| **voxel-camera** | Shared camera math and input snapshot for the front ends. |
| **voxel-viewer** | Native interactive viewer (winit + wgpu surface). |
| **voxel-web** | WASM / WebGPU kernel behind the TypeScript shell in [`web/`](web/). |
| **voxel-cli** | Headless driver: measurement, benchmarking, and the CPU↔GPU differential. |

## Quickstart

```sh
make viewer                    # native viewer, default fixture
make mesh MESH=model.glb       # voxelize + view a mesh (glTF · OBJ · STL)
make web                       # build the WASM kernel and serve the web shell
```

Native rendering needs a `wgpu`-capable adapter (Metal / Vulkan / DX12). The web
shell additionally needs [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) and
Node. Run `make help` for the full command catalog.

## Correctness

Every GPU path is validated against a pure CPU reference. The merge gate,
`cargo xtask ci-gpu`, *fails* rather than *skips* when those bit-exact contracts
can't be witnessed on real hardware, and `unsafe_code` is denied workspace-wide.
See [CONTRIBUTING.md](CONTRIBUTING.md) for the gate.

## License

Dual-licensed under **MIT** or **Apache-2.0**, at your option.
