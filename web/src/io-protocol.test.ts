import { describe, expect, it } from "vitest";
import { MeshFormat } from "voxel-web";

import { importKindFor } from "./io-protocol";

describe("importKindFor", () => {
  it("routes mesh formats to the voxelizer with the right format tag", () => {
    expect(importKindFor("model.glb")).toEqual({ kind: "mesh", format: MeshFormat.Glb });
    expect(importKindFor("model.gltf")).toEqual({ kind: "mesh", format: MeshFormat.Glb });
    expect(importKindFor("model.obj")).toEqual({ kind: "mesh", format: MeshFormat.Obj });
    expect(importKindFor("model.stl")).toEqual({ kind: "mesh", format: MeshFormat.Stl });
  });

  it("routes voxel-native formats to their direct loaders", () => {
    expect(importKindFor("scene.vox")).toEqual({ kind: "vox" });
    expect(importKindFor("scene.cvox")).toEqual({ kind: "cvox" });
  });

  it("is case-insensitive and uses the last extension", () => {
    expect(importKindFor("MODEL.GLB")).toEqual({ kind: "mesh", format: MeshFormat.Glb });
    expect(importKindFor("archive.tar.vox")).toEqual({ kind: "vox" });
  });

  it("rejects unknown or missing extensions", () => {
    expect(importKindFor("notes.txt")).toBeUndefined();
    expect(importKindFor("no-extension")).toBeUndefined();
    expect(importKindFor("")).toBeUndefined();
  });
});
