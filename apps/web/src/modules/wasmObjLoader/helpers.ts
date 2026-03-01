export const parseObjFallback = (input: string) => {
  const positions: number[] = [];
  const indices: number[] = [];
  // Group 0 is always "(default)" — covers faces before the first usemtl.
  const materialGroupNames: string[] = ["(default)"];
  const triangleMaterials: number[] = [];
  let currentMaterial = 0;

  const lines = input.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("usemtl ")) {
      const name = trimmed.slice("usemtl ".length).trim();
      let idx = materialGroupNames.indexOf(name);
      if (idx === -1) {
        idx = materialGroupNames.length;
        materialGroupNames.push(name);
      }
      currentMaterial = idx;
    } else if (trimmed.startsWith("v ")) {
      const parts = trimmed.split(/\s+/);
      if (parts.length >= 4) {
        const x = Number(parts[1]);
        const y = Number(parts[2]);
        const z = Number(parts[3]);
        if (Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(z)) {
          positions.push(x, y, z);
        }
      }
    } else if (trimmed.startsWith("f ")) {
      const parts = trimmed.split(/\s+/).slice(1);
      const faceIndices = parts
        .map((part) => Number(part.split("/")[0]))
        .filter((value) => Number.isFinite(value) && value > 0)
        .map((value) => value - 1);

      if (faceIndices.length >= 3) {
        const base = faceIndices[0];
        const outTriCount = faceIndices.length - 2;
        for (let i = 1; i < faceIndices.length - 1; i += 1) {
          indices.push(base, faceIndices[i], faceIndices[i + 1]);
        }
        for (let t = 0; t < outTriCount; t += 1) {
          triangleMaterials.push(currentMaterial);
        }
      }
    }
  }

  return {
    positions: new Float32Array(positions),
    indices: new Uint32Array(indices),
    materialGroupNames,
    triangleMaterials: new Uint32Array(triangleMaterials)
  };
};

/**
 * Convert material group parse results into the flat Uint16Array material_table
 * required by voxelize_and_apply (spec §2.3).
 *
 * MaterialId is 1-based: group index 0 → MaterialId 1, group index 1 → MaterialId 2, etc.
 * MaterialId 0 (MATERIAL_EMPTY) is never emitted.
 */
export const buildMaterialTable = (
  triangleMaterials: Uint32Array,
  _materialGroupNames: string[]
): Uint16Array => {
  const table = new Uint16Array(triangleMaterials.length);
  for (let i = 0; i < triangleMaterials.length; i += 1) {
    table[i] = triangleMaterials[i] + 1;
  }
  return table;
};

export const buildMatrixFallback = (
  scale: number,
  rotX: number,
  rotY: number,
  rotZ: number,
  tx: number,
  ty: number,
  tz: number
) => {
  const toRad = (deg: number) => (deg * Math.PI) / 180;
  const [sx, cx] = [Math.sin(toRad(rotX)), Math.cos(toRad(rotX))];
  const [sy, cy] = [Math.sin(toRad(rotY)), Math.cos(toRad(rotY))];
  const [sz, cz] = [Math.sin(toRad(rotZ)), Math.cos(toRad(rotZ))];

  const s = [
    scale, 0, 0, 0,
    0, scale, 0, 0,
    0, 0, scale, 0,
    0, 0, 0, 1
  ];
  const rx = [
    1, 0, 0, 0,
    0, cx, sx, 0,
    0, -sx, cx, 0,
    0, 0, 0, 1
  ];
  const ry = [
    cy, 0, -sy, 0,
    0, 1, 0, 0,
    sy, 0, cy, 0,
    0, 0, 0, 1
  ];
  const rz = [
    cz, sz, 0, 0,
    -sz, cz, 0, 0,
    0, 0, 1, 0,
    0, 0, 0, 1
  ];
  const t = [
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    tx, ty, tz, 1
  ];

  const mul = (a: number[], b: number[]) => {
    const out = new Array<number>(16).fill(0);
    for (let c = 0; c < 4; c += 1) {
      for (let r = 0; r < 4; r += 1) {
        let sum = 0;
        for (let k = 0; k < 4; k += 1) {
          sum += a[k * 4 + r] * b[c * 4 + k];
        }
        out[c * 4 + r] = sum;
      }
    }
    return out;
  };

  const rs = mul(rz, mul(ry, mul(rx, s)));
  const m = mul(t, rs);
  return new Float32Array(m);
};

export const applyMatrix = (positions: Float32Array, matrix: Float32Array) => {
  const count = Math.floor(positions.length / 3);
  const out = new Float32Array(count * 3);
  for (let i = 0; i < count; i += 1) {
    const src = i * 3;
    const x = positions[src];
    const y = positions[src + 1];
    const z = positions[src + 2];
    const ox =
      matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12];
    const oy =
      matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13];
    const oz =
      matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14];
    out[src] = ox;
    out[src + 1] = oy;
    out[src + 2] = oz;
  }
  return out;
};
