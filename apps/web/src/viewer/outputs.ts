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
  PerspectiveCamera,
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

const LOG_VIEWER = true;

type LodContext = {
  camera: PerspectiveCamera;
  viewportHeight: number;
};

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

const buildVoxels = (
  output: Extract<ModuleOutput, { kind: "voxels" }>,
  lodContext?: LodContext
) => {
  const positions = ensureFloat32Array(output.voxels.positions);
  let cubePositions = positions;
  let farPositions: Float32Array | null = null;
  const renderMode = output.voxels.renderMode ?? "cubes";
  const lod = output.voxels.lod;
  const lodActive = lod?.mode === "camera" && renderMode === "cubes" && lodContext;
  if (lodActive) {
    const pixelThreshold = lod.pixelThreshold ?? 2;
    const camera = lodContext.camera;
    const fov = (camera.fov * Math.PI) / 180;
    const scale = lodContext.viewportHeight / (2 * Math.tan(fov * 0.5));
    const near: number[] = [];
    const far: number[] = [];
    for (let i = 0; i < positions.length; i += 3) {
      const dx = positions[i] - camera.position.x;
      const dy = positions[i + 1] - camera.position.y;
      const dz = positions[i + 2] - camera.position.z;
      const distance = Math.max(0.0001, Math.sqrt(dx * dx + dy * dy + dz * dz));
      const pixelSize = (output.voxels.voxelSize * scale) / distance;
      if (pixelSize >= pixelThreshold) {
        near.push(positions[i], positions[i + 1], positions[i + 2]);
      } else {
        far.push(positions[i], positions[i + 1], positions[i + 2]);
      }
    }
    cubePositions = new Float32Array(near);
    farPositions = far.length > 0 ? new Float32Array(far) : null;
  }
  const voxelCount = cubePositions.length / 3;
  const chunkSize =
    output.voxels.chunkSize && output.voxels.chunkSize > 0
      ? output.voxels.chunkSize
      : voxelCount;
  const group = new Group();
  group.name = output.label ?? "voxels";
  const addPoints = (pointsPositions: Float32Array, material: PointsMaterial) => {
    const pointCount = pointsPositions.length / 3;
    const pointChunk = chunkSize > 0 ? chunkSize : pointCount;
    for (let start = 0; start < pointCount; start += pointChunk) {
      const end = Math.min(start + pointChunk, pointCount);
      const slice = pointsPositions.subarray(start * 3, end * 3);
      const geometry = new BufferGeometry();
      geometry.setAttribute("position", new BufferAttribute(slice, 3));
      geometry.computeBoundingBox();
      geometry.computeBoundingSphere();
      const points = new Points(geometry, material);
      points.name = output.label ?? "voxels-points";
      group.add(points);
    }
  };

  if (renderMode === "points" && !lodActive) {
    const material = new PointsMaterial({
      color: colorFromTuple(output.voxels.color),
      size: output.voxels.pointSize ?? output.voxels.voxelSize * 0.5
    });
    addPoints(positions, material);
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
  const maxInstancesPerMesh =
    output.voxels.maxInstancesPerMesh && output.voxels.maxInstancesPerMesh > 0
      ? output.voxels.maxInstancesPerMesh
      : voxelCount;
  const cubeChunk = Math.max(1, Math.min(chunkSize, maxInstancesPerMesh));
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
  const totalMeshes = Math.ceil(voxelCount / maxInstancesPerMesh);
  const totalChunks = Math.ceil(voxelCount / cubeChunk);
  if (totalMeshes > 1) {
    console.info(
      "[viewer] voxels cubes instancing",
      `meshes=${totalMeshes}`,
      `chunkSize=${cubeChunk}`,
      `maxInstancesPerMesh=${maxInstancesPerMesh}`
    );
  }
  const buildToken = Symbol("voxels-cubes");
  group.userData.buildToken = buildToken;
  let next = 0;
  const instancedMeshes: InstancedMesh[] = [];
  for (let meshIndex = 0; meshIndex < totalMeshes; meshIndex += 1) {
    const start = meshIndex * maxInstancesPerMesh;
    const count = Math.min(maxInstancesPerMesh, voxelCount - start);
    const instanced = new InstancedMesh(geometry, material, count);
    instanced.count = count;
    instanced.instanceMatrix.setUsage(DynamicDrawUsage);
    instanced.frustumCulled = false;
    instanced.name = output.label ?? "voxels";
    instancedMeshes.push(instanced);
    group.add(instanced);
  }
  let skippedWrites = 0;
  const buildStep = () => {
    if (group.userData.buildToken !== buildToken) {
      return;
    }
    const frameStart = performance.now();
    const updated = new Set<number>();
    const maxWritten: number[] = [];
    while (next < voxelCount && performance.now() - frameStart < 8) {
      const end = Math.min(next + cubeChunk, voxelCount);
      for (let i = next; i < end; i += 1) {
        const index = i * 3;
        position.set(cubePositions[index], cubePositions[index + 1], cubePositions[index + 2]);
        matrix.makeTranslation(position.x, position.y, position.z);
        const meshIndex = Math.floor(i / maxInstancesPerMesh);
        const localIndex = i - meshIndex * maxInstancesPerMesh;
        const instanced = instancedMeshes[meshIndex];
        if (!instanced || localIndex >= instanced.count) {
          skippedWrites += 1;
          continue;
        }
        instanced.setMatrixAt(localIndex, matrix);
        updated.add(meshIndex);
        const current = maxWritten[meshIndex] ?? -1;
        if (localIndex > current) {
          maxWritten[meshIndex] = localIndex;
        }
      }
      next = end;
    }
    for (const meshIndex of updated) {
      const instanced = instancedMeshes[meshIndex];
      const maxIndex = maxWritten[meshIndex] ?? instanced.count - 1;
      const start = 0;
      const count = (maxIndex + 1) * 16;
      const maxElements = instanced.instanceMatrix.array.length;
      if (count <= 0 || start + count > maxElements) {
        if (LOG_VIEWER) {
          console.warn(
            "[viewer] voxels cubes instancing",
            `updateRange invalid mesh=${meshIndex}`,
            `start=${start}`,
            `count=${count}`,
            `max=${maxElements}`
          );
        }
        instanced.instanceMatrix.needsUpdate = true;
        continue;
      }
      instanced.instanceMatrix.clearUpdateRanges();
      instanced.instanceMatrix.addUpdateRange(start, count);
      instanced.instanceMatrix.needsUpdate = true;
    }
    if (updated.size > 0) {
      const onChunkAdded = group.userData.onChunkAdded as undefined | (() => void);
      onChunkAdded?.();
      markSceneUpdate();
    }
    if (skippedWrites > 0 && LOG_VIEWER) {
      console.warn(
        "[viewer] voxels cubes instancing",
        `skippedWrites=${skippedWrites}`
      );
      skippedWrites = 0;
    }
    if (next < voxelCount) {
      requestAnimationFrame(buildStep);
    } else if (voxelCount > 0) {
      for (const instanced of instancedMeshes) {
        instanced.computeBoundingBox();
        instanced.computeBoundingSphere();
      }
      const buildMs = performance.now() - buildStart;
      if (LOG_VIEWER) {
        console.log(
          "[viewer] voxels cubes complete",
          `voxels=${voxelCount}`,
          `chunks=${totalChunks}`,
          `meshes=${totalMeshes}`,
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
  if (lodActive && farPositions && farPositions.length > 0) {
    const pointsMaterial = new PointsMaterial({
      color: voxelColor,
      size: output.voxels.pointSize ?? output.voxels.voxelSize * 0.5
    });
    addPoints(farPositions, pointsMaterial);
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

export const buildOutputObject = (
  output: ModuleOutput,
  lodContext?: LodContext
): {
  object: Group | Mesh | Points | LineSegments | InstancedMesh | Sprite;
  texture?: Texture;
} => {
  switch (output.kind) {
    case "mesh":
      return { object: buildMesh(output) };
    case "voxels":
      return { object: buildVoxels(output, lodContext) };
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
