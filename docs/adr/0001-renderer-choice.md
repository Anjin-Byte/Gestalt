# 0001 - Renderer Choice

**Type:** adr
**Status:** superseded
**Superseded by:** [ADR-0011](0011-hybrid-gpu-driven.md)
**Date:** 2026-01-11

## Context
The test bed needs WebGPU with a fallback for browsers without WebGPU, and should allow swapping rendering backends later.

## Decision
Use Three.js with WebGPURenderer as the default backend, with automatic fallback to WebGL2 via WebGLRenderer.

## Consequences
- WebGPU is attempted when available, with a transparent fallback path.
- The viewer backend is wrapped so the host can swap in a different renderer later.
