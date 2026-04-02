//! Pure Rust OBJ parser — no wasm_bindgen, no JS types.
//!
//! Parses vertex positions, face indices (with fan triangulation for n-gons),
//! and `usemtl` material group assignments. Normals and texcoords are ignored
//! (voxelization discards them).

/// Parsed OBJ mesh data.
#[derive(Debug, Clone)]
pub struct ParsedObj {
    /// Vertex positions, one [x, y, z] per vertex.
    pub positions: Vec<[f32; 3]>,
    /// Triangle index triples (indices into `positions`).
    pub triangles: Vec<[u32; 3]>,
    /// Material group index per triangle (indices into `material_names`).
    pub triangle_materials: Vec<u32>,
    /// Material group names in order of first appearance.
    /// Index 0 is always "(default)" for faces before any `usemtl`.
    pub material_names: Vec<String>,
}

/// Parse an OBJ string into mesh data.
///
/// Handles `v` (vertex positions), `f` (faces with fan triangulation),
/// and `usemtl` (material group switching). Face indices may use the
/// `v/vt/vn` format — only the vertex index is extracted.
///
/// Negative indices are not supported. Degenerate faces (< 3 vertices)
/// are silently skipped.
pub fn parse_obj(input: &str) -> ParsedObj {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut material_names: Vec<String> = vec!["(default)".to_string()];
    let mut triangle_materials: Vec<u32> = Vec::new();
    let mut current_material: u32 = 0;

    for line in input.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("usemtl ") {
            let name = trimmed["usemtl ".len()..].trim();
            current_material =
                if let Some(idx) = material_names.iter().position(|n| n == name) {
                    idx as u32
                } else {
                    let idx = material_names.len() as u32;
                    material_names.push(name.to_string());
                    idx
                };
        } else if trimmed.starts_with("v ") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 4 {
                if let (Ok(x), Ok(y), Ok(z)) = (
                    parts[1].parse::<f32>(),
                    parts[2].parse::<f32>(),
                    parts[3].parse::<f32>(),
                ) {
                    positions.push([x, y, z]);
                }
            }
        } else if trimmed.starts_with("f ") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 4 {
                let mut face_indices: Vec<u32> = Vec::new();
                for part in parts.iter().skip(1) {
                    // Handle v/vt/vn format — extract only the vertex index.
                    let raw = part.split('/').next().unwrap_or("");
                    if let Ok(idx) = raw.parse::<i32>() {
                        if idx > 0 {
                            face_indices.push((idx - 1) as u32);
                        }
                    }
                }

                // Fan triangulation: first vertex is the pivot.
                if face_indices.len() >= 3 {
                    let base = face_indices[0];
                    for i in 1..face_indices.len() - 1 {
                        triangles.push([base, face_indices[i], face_indices[i + 1]]);
                        triangle_materials.push(current_material);
                    }
                }
            }
        }
    }

    ParsedObj {
        positions,
        triangles,
        triangle_materials,
        material_names,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_triangle() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
f 1 2 3
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.positions.len(), 3);
        assert_eq!(parsed.triangles.len(), 1);
        assert_eq!(parsed.triangles[0], [0, 1, 2]);
        assert_eq!(parsed.triangle_materials[0], 0); // default group
        assert_eq!(parsed.material_names[0], "(default)");
    }

    #[test]
    fn parse_quad_fan_triangulation() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
f 1 2 3 4
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.triangles.len(), 2);
        assert_eq!(parsed.triangles[0], [0, 1, 2]);
        assert_eq!(parsed.triangles[1], [0, 2, 3]);
    }

    #[test]
    fn parse_pentagon_fan_triangulation() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.5 1.5 0.0
v 0.0 1.0 0.0
f 1 2 3 4 5
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.triangles.len(), 3);
        assert_eq!(parsed.triangles[0], [0, 1, 2]);
        assert_eq!(parsed.triangles[1], [0, 2, 3]);
        assert_eq!(parsed.triangles[2], [0, 3, 4]);
    }

    #[test]
    fn parse_vt_vn_format() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vt 0.0 0.0
vt 1.0 0.0
vt 0.0 1.0
vn 0.0 0.0 1.0
f 1/1/1 2/2/1 3/3/1
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.triangles.len(), 1);
        assert_eq!(parsed.triangles[0], [0, 1, 2]);
    }

    #[test]
    fn parse_usemtl_groups() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
v 2.0 0.0 0.0
v 2.0 1.0 0.0
v 3.0 0.0 0.0
usemtl red
f 1 2 3
usemtl blue
f 4 5 6
usemtl red
f 1 3 4
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.material_names.len(), 3); // (default), red, blue
        assert_eq!(parsed.material_names[1], "red");
        assert_eq!(parsed.material_names[2], "blue");
        assert_eq!(parsed.triangle_materials[0], 1); // red
        assert_eq!(parsed.triangle_materials[1], 2); // blue
        assert_eq!(parsed.triangle_materials[2], 1); // red again (reused index)
    }

    #[test]
    fn parse_faces_before_usemtl_get_default() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
f 1 2 3
usemtl wood
v 2.0 0.0 0.0
f 1 2 4
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.triangle_materials[0], 0); // default
        assert_eq!(parsed.triangle_materials[1], 1); // wood
    }

    #[test]
    fn parse_empty_input() {
        let parsed = parse_obj("");
        assert_eq!(parsed.positions.len(), 0);
        assert_eq!(parsed.triangles.len(), 0);
        assert_eq!(parsed.material_names.len(), 1); // always has (default)
    }

    #[test]
    fn parse_comments_and_blank_lines() {
        let obj = "\
# This is a comment
v 0.0 0.0 0.0

v 1.0 0.0 0.0
# Another comment
v 0.0 1.0 0.0
f 1 2 3
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.positions.len(), 3);
        assert_eq!(parsed.triangles.len(), 1);
    }

    #[test]
    fn parse_degenerate_face_skipped() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
f 1 2
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.triangles.len(), 0);
    }

    #[test]
    fn parse_cube() {
        let obj = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
v 0.0 0.0 1.0
v 1.0 0.0 1.0
v 1.0 1.0 1.0
v 0.0 1.0 1.0
f 1 2 3 4
f 5 8 7 6
f 1 5 6 2
f 3 7 8 4
f 2 6 7 3
f 1 4 8 5
";
        let parsed = parse_obj(obj);
        assert_eq!(parsed.positions.len(), 8);
        // 6 quads → 12 triangles
        assert_eq!(parsed.triangles.len(), 12);
    }
}
