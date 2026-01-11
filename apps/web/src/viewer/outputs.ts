import {
  BoxGeometry,
  BufferAttribute,
  BufferGeometry,
  Color,
  DataTexture,
  Group,
  InstancedMesh,
  LineBasicMaterial,
  LineSegments,
  Matrix4,
  Mesh,
  MeshStandardMaterial,
  Points,
  PointsMaterial,
  SRGBColorSpace,
  Sprite,
  SpriteMaterial,
  DoubleSide,
  Texture,
  Vector3,
  DynamicDrawUsage
} from "three";
import type { ModuleOutput, Vec3Tuple } from "../modules/types";

export type OutputStats = {
  triangles: number;
  instances: number;
};

const LOG_VIEWER = false;

const colorFromTuple = (color?: Vec3Tuple): Color => {
  if (!color) {
    return new Color(0.6, 0.8, 1.0);
  }
  return new Color(color[0], color[1], color[2]);
};

const ensureFloat32Array = (input: Float32Array<ArrayBufferLike>): Float32Array<ArrayBuffer> => {
  if (input.buffer instanceof ArrayBuffer) {
    return input as Float32Array<ArrayBuffer>;
  }
  return new Float32Array(input);
};

const ensureUint32Array = (input: Uint32Array<ArrayBufferLike>): Uint32Array<ArrayBuffer> => {
  if (input.buffer instanceof ArrayBuffer) {
    return input as Uint32Array<ArrayBuffer>;
  }
  return new Uint32Array(input);
};

const buildMesh = (output: Extract<ModuleOutput, { kind: "mesh" }>) => {
  const safePositions = ensureFloat32Array(output.mesh.positions);
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new BufferAttribute(safePositions, 3));
  if (output.mesh.indices) {
    geometry.setIndex(new BufferAttribute(ensureUint32Array(output.mesh.indices), 1));
  }
  if (output.mesh.normals) {
    geometry.setAttribute(
      "normal",
      new BufferAttribute(ensureFloat32Array(output.mesh.normals), 3)
    );
  } else {
    geometry.computeVertexNormals();
  }
  if (output.mesh.colors) {
    geometry.setAttribute(
      "color",
      new BufferAttribute(ensureFloat32Array(output.mesh.colors), 3)
    );
  }

  const material = new MeshStandardMaterial({
    color: 0x7ad8ff,
    roughness: 0.35,
    metalness: 0.1,
    vertexColors: Boolean(output.mesh.colors),
    side: DoubleSide
  });
  const mesh = new Mesh(geometry, material);
  mesh.name = output.label ?? "mesh";
  return mesh;
};

const buildVoxels = (output: Extract<ModuleOutput, { kind: "voxels" }>) => {
  const positions = ensureFloat32Array(output.voxels.positions);
  const voxelCount = positions.length / 3;
  const chunkSize =
    output.voxels.chunkSize && output.voxels.chunkSize > 0
      ? output.voxels.chunkSize
      : voxelCount;
  const renderMode = output.voxels.renderMode ?? "cubes";
  const group = new Group();
  group.name = output.label ?? "voxels";

  if (renderMode === "points") {
    const material = new PointsMaterial({
      color: colorFromTuple(output.voxels.color),
      size: output.voxels.pointSize ?? output.voxels.voxelSize * 0.5
    });
    for (let start = 0; start < voxelCount; start += chunkSize) {
      const end = Math.min(start + chunkSize, voxelCount);
      const slice = positions.subarray(start * 3, end * 3);
      const geometry = new BufferGeometry();
      geometry.setAttribute("position", new BufferAttribute(slice, 3));
      geometry.computeBoundingBox();
      geometry.computeBoundingSphere();
      const points = new Points(geometry, material);
      points.name = output.label ?? "voxels-points";
      group.add(points);
    }
    return group;
  }

  const geometry = new BoxGeometry(
    output.voxels.voxelSize,
    output.voxels.voxelSize,
    output.voxels.voxelSize
  );
  const buildStart = performance.now();
  const voxelColor = colorFromTuple(output.voxels.color);
  const material = new MeshStandardMaterial({
    color: voxelColor,
    roughness: 0.6,
    metalness: 0.1,
    emissive: voxelColor,
    emissiveIntensity: 0.2
  });
  const matrix = new Matrix4();
  const position = new Vector3();
  const cubeChunk = Math.max(1, Math.min(chunkSize, voxelCount));
  if (LOG_VIEWER && chunkSize !== cubeChunk) {
    console.log(
      "[viewer] cube chunk clamped",
      `requested=${chunkSize}`,
      `used=${cubeChunk}`
    );
  }
  const markSceneUpdate = () => {
    let node: { parent?: unknown; isScene?: boolean; needsUpdate?: boolean } | null = group;
    while (node) {
      if (node.isScene) {
        node.needsUpdate = true;
        break;
      }
      node = (node.parent as typeof node) ?? null;
    }
  };
  const totalChunks = Math.ceil(voxelCount / cubeChunk);
  const buildToken = Symbol("voxels-cubes");
  group.userData.buildToken = buildToken;
  let next = 0;
  const instanced = new InstancedMesh(geometry, material, voxelCount);
  instanced.count = voxelCount;
  instanced.instanceMatrix.setUsage(DynamicDrawUsage);
  instanced.frustumCulled = false;
  instanced.name = output.label ?? "voxels";
  group.add(instanced);
  const buildStep = () => {
    if (group.userData.buildToken !== buildToken) {
      return;
    }
    const frameStart = performance.now();
    while (next < voxelCount && performance.now() - frameStart < 8) {
      const end = Math.min(next + cubeChunk, voxelCount);
      for (let i = next; i < end; i += 1) {
        const index = i * 3;
        position.set(positions[index], positions[index + 1], positions[index + 2]);
        matrix.makeTranslation(position.x, position.y, position.z);
        instanced.setMatrixAt(i, matrix);
      }
      instanced.instanceMatrix.needsUpdate = true;
      const onChunkAdded = group.userData.onChunkAdded as undefined | (() => void);
      onChunkAdded?.();
      markSceneUpdate();
      next = end;
    }
    if (next < voxelCount) {
      requestAnimationFrame(buildStep);
    } else if (voxelCount > 0) {
      instanced.computeBoundingBox();
      instanced.computeBoundingSphere();
      const buildMs = performance.now() - buildStart;
      if (LOG_VIEWER) {
        console.log(
          "[viewer] voxels cubes complete",
          `voxels=${voxelCount}`,
          `chunks=${totalChunks}`,
          `ms=${buildMs.toFixed(1)}`
        );
      }
      const onChunkAdded = group.userData.onChunkAdded as undefined | (() => void);
      onChunkAdded?.();
    }
  };
  if (voxelCount > 0) {
    buildStep();
  }
  return group;
};

const buildLines = (output: Extract<ModuleOutput, { kind: "lines" }>) => {
  const safePositions = ensureFloat32Array(output.lines.positions);
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new BufferAttribute(safePositions, 3));
  const material = new LineBasicMaterial({ color: colorFromTuple(output.lines.color) });
  const lines = new LineSegments(geometry, material);
  lines.name = output.label ?? "lines";
  return lines;
};

const buildPoints = (output: Extract<ModuleOutput, { kind: "points" }>) => {
  const safePositions = ensureFloat32Array(output.points.positions);
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new BufferAttribute(safePositions, 3));
  const material = new PointsMaterial({
    color: colorFromTuple(output.points.color),
    size: output.points.size ?? 0.05
  });
  const points = new Points(geometry, material);
  points.name = output.label ?? "points";
  return points;
};

const buildTexture = (output: Extract<ModuleOutput, { kind: "texture2d" }>) => {
  const safeData =
    output.texture.data.buffer instanceof ArrayBuffer
      ? (output.texture.data as Uint8Array<ArrayBuffer>)
      : (new Uint8Array(output.texture.data) as Uint8Array<ArrayBuffer>);
  const texture = new DataTexture(
    safeData,
    output.texture.width,
    output.texture.height
  );
  texture.needsUpdate = true;
  texture.colorSpace = SRGBColorSpace;
  return texture;
};

export const buildOutputObject = (output: ModuleOutput): {
  object: Group | Mesh | Points | LineSegments | InstancedMesh | Sprite;
  texture?: Texture;
} => {
  switch (output.kind) {
    case "mesh":
      return { object: buildMesh(output) };
    case "voxels":
      return { object: buildVoxels(output) };
    case "lines":
      return { object: buildLines(output) };
    case "points":
      return { object: buildPoints(output) };
    case "texture2d": {
      const texture = buildTexture(output);
      const sprite = new Sprite(
        new SpriteMaterial({ map: texture, depthTest: false })
      );
      sprite.scale.set(2, 2, 1);
      sprite.name = output.label ?? "texture";
      return { object: sprite, texture };
    }
  }
};

export const computeStats = (group: Group): OutputStats => {
  let triangles = 0;
  let instances = 0;
  group.traverse((child) => {
    if (child instanceof InstancedMesh) {
      instances += child.count;
      const geom = child.geometry;
      const baseTriangles = geom.index
        ? geom.index.count / 3
        : geom.attributes.position.count / 3;
      triangles += baseTriangles * child.count;
    }
    if (child instanceof Mesh) {
      const geom = child.geometry;
      triangles += geom.index ? geom.index.count / 3 : geom.attributes.position.count / 3;
    }
    if (child instanceof Points || child instanceof LineSegments) {
      instances += child.geometry.attributes.position.count / 3;
    }
  });
  return { triangles, instances };
};
