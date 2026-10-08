//! 3D model previews (roadmap F3): STL (binary / ASCII), Wavefront OBJ and glTF 2.0
//! (`.gltf` with embedded or side-car buffers, `.glb`) are flattened into one triangle
//! soup, centred and scaled to the unit sphere, and decimated to a display budget. The
//! UI draws it as a flat-shaded `egui::Mesh` (painter's algorithm, two-sided lighting):
//! no GPU callback, no z-buffer, robust on every backend including the glow fallback.

use std::path::Path;

use base64::Engine as _;
use egui::{Color32, Pos2, epaint};

/// Triangles drawn per frame at most (larger models are decimated).
pub const DISPLAY_TRIS: usize = 80_000;
/// Triangles read from a file at most.
const MAX_TRIS: usize = 4_000_000;

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub format: &'static str,
    /// Unit-sphere positions.
    pub positions: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
    /// Material colour index per triangle (into `colors`).
    pub tri_color: Vec<u16>,
    pub colors: Vec<[f32; 3]>,
    /// Triangles in the file (before decimation).
    pub total_tris: usize,
    /// Size of the original bounding box.
    pub extent: [f32; 3],
    pub meshes: usize,
}

pub fn kind_for_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "stl" => "STL",
        "obj" => "Wavefront OBJ",
        "gltf" => "glTF",
        "glb" => "glTF binary",
        _ => return None,
    })
}

/// Parse by extension; `dir` resolves glTF side-car buffers (`None` inside archives).
pub fn parse(data: &[u8], ext: &str, dir: Option<&Path>, cap: u64) -> Result<Model, String> {
    let mut m = match ext {
        "stl" => stl(data)?,
        "obj" => obj(&String::from_utf8_lossy(data))?,
        "gltf" | "glb" => gltf_model(data, dir, cap)?,
        _ => return Err("Unknown model format".into()),
    };
    if m.tris.is_empty() {
        return Err("The model has no triangles".into());
    }
    if m.colors.is_empty() {
        m.colors.push([0.78, 0.80, 0.84]);
    }
    if m.tri_color.len() != m.tris.len() {
        m.tri_color = vec![0; m.tris.len()];
    }
    m.total_tris = m.tris.len();
    normalize(&mut m);
    decimate(&mut m, DISPLAY_TRIS);
    Ok(m)
}

fn f32le(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn stl(data: &[u8]) -> Result<Model, String> {
    let mut m = Model { format: "STL", ..Default::default() };
    let n = data.get(80..84).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let binary = n.is_some_and(|n| 84 + n * 50 == data.len());
    let ascii = data.trim_ascii_start().starts_with(b"solid") && !binary;
    if ascii {
        let text = String::from_utf8_lossy(data);
        let mut verts: Vec<[f32; 3]> = Vec::new();
        let mut it = text.split_ascii_whitespace();
        while let Some(tok) = it.next() {
            if tok == "vertex" {
                let mut v = [0f32; 3];
                for c in &mut v {
                    *c = it.next().and_then(|t| t.parse().ok()).ok_or("Damaged ASCII STL")?;
                }
                verts.push(v);
                if verts.len() / 3 >= MAX_TRIS {
                    break;
                }
            }
        }
        for t in verts.as_chunks::<3>().0 {
            let base = m.positions.len() as u32;
            m.positions.extend_from_slice(t);
            m.tris.push([base, base + 1, base + 2]);
        }
    } else {
        let n = n.ok_or("Damaged STL file")?;
        let n = n.min((data.len().saturating_sub(84)) / 50).min(MAX_TRIS);
        m.positions.reserve(n * 3);
        m.tris.reserve(n);
        for i in 0..n {
            let at = 84 + i * 50 + 12;
            let base = m.positions.len() as u32;
            for k in 0..3 {
                let o = at + k * 12;
                m.positions.push([f32le(data, o), f32le(data, o + 4), f32le(data, o + 8)]);
            }
            m.tris.push([base, base + 1, base + 2]);
        }
    }
    m.meshes = 1;
    Ok(m)
}

pub fn obj(text: &str) -> Result<Model, String> {
    let mut m = Model { format: "Wavefront OBJ", ..Default::default() };
    let mut objects = 0usize;
    for line in text.lines() {
        let mut it = line.split_ascii_whitespace();
        match it.next() {
            Some("v") => {
                let mut v = [0f32; 3];
                for c in &mut v {
                    *c = it.next().and_then(|t| t.parse().ok()).unwrap_or(0.0);
                }
                m.positions.push(v);
            }
            Some("f") => {
                let n = m.positions.len() as i64;
                let idx: Vec<u32> = it
                    .filter_map(|t| t.split('/').next()?.parse::<i64>().ok())
                    .filter_map(|i| {
                        let i = if i < 0 { n + i } else { i - 1 };
                        (0..n).contains(&i).then_some(i as u32)
                    })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    m.tris.push([idx[0], idx[k], idx[k + 1]]);
                }
                if m.tris.len() >= MAX_TRIS {
                    break;
                }
            }
            Some("o" | "g") => objects += 1,
            _ => {}
        }
    }
    m.meshes = objects.max(1);
    Ok(m)
}

fn mat_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // Column-major: out[c][r] = sum_k a[k][r] * b[c][k].
    let mut out = [[0f32; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, cell) in col.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

fn transform(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3];
    let w = if w.abs() < 1e-8 { 1.0 } else { w };
    [
        (m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0]) / w,
        (m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1]) / w,
        (m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2]) / w,
    ]
}

fn gltf_model(data: &[u8], dir: Option<&Path>, cap: u64) -> Result<Model, String> {
    let g = gltf::Gltf::from_slice(data).map_err(|e| format!("Not a readable glTF file ({e})"))?;
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for b in g.buffers() {
        let bytes = match b.source() {
            gltf::buffer::Source::Bin => g.blob.clone().unwrap_or_default(),
            gltf::buffer::Source::Uri(uri) => {
                if let Some(rest) = uri.strip_prefix("data:") {
                    let payload = rest.split_once(";base64,").map(|(_, d)| d).unwrap_or("");
                    base64::engine::general_purpose::STANDARD.decode(payload).unwrap_or_default()
                } else if let Some(d) = dir {
                    // Side-car buffer next to the .gltf (no absolute paths, no `..`).
                    let rel = Path::new(uri);
                    let safe = rel.components().all(|c| matches!(c, std::path::Component::Normal(_)));
                    let p = d.join(rel);
                    match std::fs::metadata(&p) {
                        Ok(md) if safe && md.len() <= cap => std::fs::read(&p).unwrap_or_default(),
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                }
            }
        };
        buffers.push(bytes);
    }
    let mut m = Model { format: if data.starts_with(b"glTF") { "glTF binary" } else { "glTF" }, ..Default::default() };
    let mut mat_index: std::collections::HashMap<Option<usize>, u16> = std::collections::HashMap::new();
    let ident = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut stack: Vec<(gltf::Node<'_>, [[f32; 4]; 4], usize)> = Vec::new();
    match g.default_scene().or_else(|| g.scenes().next()) {
        Some(scene) => stack.extend(scene.nodes().map(|n| (n, ident, 0))),
        None => stack.extend(g.nodes().map(|n| (n, ident, 0))),
    }
    let mut visited = 0usize;
    while let Some((node, parent, depth)) = stack.pop() {
        visited += 1;
        if visited > 100_000 || depth > 64 {
            break;
        }
        let world = mat_mul(&parent, &node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            m.meshes += 1;
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = prim.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
                let Some(pos) = reader.read_positions() else { continue };
                let base = m.positions.len() as u32;
                m.positions.extend(pos.map(|p| transform(&world, p)));
                let count = m.positions.len() as u32 - base;
                let mat = prim.material();
                let key = mat.index();
                let next = m.colors.len() as u16;
                let ci = *mat_index.entry(key).or_insert_with(|| {
                    let c = mat.pbr_metallic_roughness().base_color_factor();
                    m.colors.push([c[0], c[1], c[2]]);
                    next
                });
                match reader.read_indices() {
                    Some(idx) => {
                        let idx: Vec<u32> = idx.into_u32().collect();
                        for t in idx.as_chunks::<3>().0 {
                            if t.iter().all(|&i| i < count) {
                                m.tris.push([base + t[0], base + t[1], base + t[2]]);
                            }
                        }
                    }
                    None => {
                        for t in 0..count / 3 {
                            m.tris.push([base + t * 3, base + t * 3 + 1, base + t * 3 + 2]);
                        }
                    }
                }
                m.tri_color.resize(m.tris.len(), ci);
                if m.tris.len() >= MAX_TRIS {
                    return Ok(m);
                }
            }
        }
        stack.extend(node.children().map(|c| (c, world, depth + 1)));
    }
    Ok(m)
}

/// Centre on the bounding box and scale into the unit sphere.
fn normalize(m: &mut Model) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in &m.positions {
        if p.iter().any(|c| !c.is_finite()) {
            continue;
        }
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] > hi[0] {
        lo = [0.0; 3];
        hi = [0.0; 3];
    }
    m.extent = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let mut r = 0f32;
    for p in &mut m.positions {
        for k in 0..3 {
            p[k] = if p[k].is_finite() { p[k] - c[k] } else { 0.0 };
        }
        r = r.max((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt());
    }
    if r > 0.0 {
        for p in &mut m.positions {
            for v in p.iter_mut() {
                *v /= r;
            }
        }
    }
}

/// Keep every n-th triangle so at most `max` remain.
fn decimate(m: &mut Model, max: usize) {
    if m.tris.len() <= max {
        return;
    }
    let step = m.tris.len().div_ceil(max);
    m.tris = m.tris.iter().step_by(step).copied().collect();
    m.tri_color = m.tri_color.iter().step_by(step).copied().collect();
}

/// The view: yaw / pitch in radians, zoom factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        View { yaw: 0.6, pitch: 0.42, zoom: 1.0 }
    }
}

/// Project, shade and depth-sort the model into an egui mesh centred on `center`, with
/// the unit sphere `radius` points wide. `tint` multiplies the material colours.
pub fn project(m: &Model, view: View, center: Pos2, radius: f32, tint: [f32; 3]) -> epaint::Mesh {
    let (sy, cy) = view.yaw.sin_cos();
    let (sp, cp) = view.pitch.sin_cos();
    let rot = |p: &[f32; 3]| -> [f32; 3] {
        // Yaw around Y, then pitch around X; +Z points at the viewer.
        let x = cy * p[0] + sy * p[2];
        let z = -sy * p[0] + cy * p[2];
        let y = cp * p[1] - sp * z;
        let z = sp * p[1] + cp * z;
        [x, y, z]
    };
    let pts: Vec<[f32; 3]> = m.positions.iter().map(rot).collect();
    let d = 4.0f32;
    let scale = radius * view.zoom;
    let screen = |p: &[f32; 3]| -> Pos2 {
        let f = d / (d - p[2]).max(0.5);
        Pos2::new(center.x + p[0] * f * scale, center.y - p[1] * f * scale)
    };
    let light = {
        let l = [0.35f32, 0.55, 0.9];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    let mut order: Vec<(f32, usize)> = Vec::with_capacity(m.tris.len());
    for (i, t) in m.tris.iter().enumerate() {
        let (Some(a), Some(b), Some(c)) = (pts.get(t[0] as usize), pts.get(t[1] as usize), pts.get(t[2] as usize))
        else {
            continue;
        };
        order.push(((a[2] + b[2] + c[2]) / 3.0, i));
    }
    order.sort_unstable_by(|x, y| x.0.total_cmp(&y.0));
    let mut mesh = epaint::Mesh::default();
    mesh.vertices.reserve(order.len() * 3);
    mesh.indices.reserve(order.len() * 3);
    for (_, i) in order {
        let t = m.tris[i];
        let (a, b, c) = (&pts[t[0] as usize], &pts[t[1] as usize], &pts[t[2] as usize]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let lambert = if len > 0.0 { ((n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / len).abs() } else { 0.5 };
        // Slight rim from the view-facing component keeps silhouettes readable.
        let facing = if len > 0.0 { (n[2] / len).abs() } else { 0.5 };
        let shade = 0.18 + 0.67 * lambert + 0.15 * facing;
        let base = m.colors.get(*m.tri_color.get(i).unwrap_or(&0) as usize).copied().unwrap_or([0.8; 3]);
        let to8 = |c: f32| ((c.clamp(0.0, 1.0)).powf(1.0 / 1.6) * 255.0) as u8;
        let col = Color32::from_rgb(
            to8(base[0] * tint[0] * shade),
            to8(base[1] * tint[1] * shade),
            to8(base[2] * tint[2] * shade),
        );
        let idx = mesh.vertices.len() as u32;
        for p in [a, b, c] {
            mesh.vertices.push(epaint::Vertex { pos: screen(p), uv: epaint::WHITE_UV, color: col });
        }
        mesh.indices.extend_from_slice(&[idx, idx + 1, idx + 2]);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_stl(tris: &[[[f32; 3]; 3]]) -> Vec<u8> {
        let mut b = vec![0u8; 80];
        b.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for t in tris {
            b.extend_from_slice(&[0u8; 12]);
            for v in t {
                for c in v {
                    b.extend_from_slice(&c.to_le_bytes());
                }
            }
            b.extend_from_slice(&[0, 0]);
        }
        b
    }

    const TET: [[[f32; 3]; 3]; 4] = [
        [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
        [[0.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]],
        [[0.0, 0.0, 0.0], [0.0, 0.0, 10.0], [10.0, 0.0, 0.0]],
        [[10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]],
    ];

    #[test]
    fn binary_and_ascii_stl() {
        let m = parse(&binary_stl(&TET), "stl", None, 1 << 20).unwrap();
        assert_eq!((m.total_tris, m.tris.len()), (4, 4));
        assert_eq!(m.extent, [10.0, 10.0, 10.0]);
        let r = m.positions.iter().map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()).fold(0f32, f32::max);
        assert!((r - 1.0).abs() < 1e-4, "normalized to the unit sphere");

        let ascii = "solid t\n facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex 1 0 0\n vertex 0 1 0\n endloop\n endfacet\nendsolid t\n";
        let m = parse(ascii.as_bytes(), "stl", None, 1 << 20).unwrap();
        assert_eq!(m.total_tris, 1);
        // A binary STL whose header starts with "solid" is still binary.
        let mut b = binary_stl(&TET);
        b[..5].copy_from_slice(b"solid");
        assert_eq!(parse(&b, "stl", None, 1 << 20).unwrap().total_tris, 4);
        assert!(parse(b"solid x\nendsolid", "stl", None, 1 << 20).is_err());
        assert!(parse(&[1, 2, 3], "stl", None, 1 << 20).is_err());
    }

    #[test]
    fn obj_faces_fan_and_negative_indices() {
        let m =
            parse(b"o a\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1/1 2/2 3/3 4/4\nf -4 -3 -2\n", "obj", None, 1 << 20)
                .unwrap();
        assert_eq!(m.total_tris, 3);
        assert_eq!(m.meshes, 1);
    }

    #[test]
    fn gltf_with_embedded_buffer() {
        let mut buf = Vec::new();
        for v in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in v {
                buf.extend_from_slice(&c.to_le_bytes());
            }
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"mesh":0,"translation":[5,0,0]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"material":0}}]}}],
            "materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1]}}}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}}],
            "buffers":[{{"byteLength":36,"uri":"data:application/octet-stream;base64,{b64}"}}]}}"#
        );
        let m = parse(json.as_bytes(), "gltf", None, 1 << 20).unwrap();
        assert_eq!(m.total_tris, 1);
        assert_eq!(m.colors[0], [1.0, 0.0, 0.0]);
        assert_eq!(m.extent, [1.0, 1.0, 0.0]);
    }

    #[test]
    fn decimation_and_projection() {
        let mut big = Vec::new();
        for i in 0..(DISPLAY_TRIS + 500) {
            let x = i as f32;
            big.push([[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 1.0]]);
        }
        let m = parse(&binary_stl(&big), "stl", None, u64::MAX).unwrap();
        assert_eq!(m.total_tris, DISPLAY_TRIS + 500);
        assert!(m.tris.len() <= DISPLAY_TRIS);
        let tet = parse(&binary_stl(&TET), "stl", None, 1 << 20).unwrap();
        let mesh = project(&tet, View::default(), Pos2::new(100.0, 100.0), 50.0, [1.0; 3]);
        assert_eq!(mesh.vertices.len(), 12);
        assert_eq!(mesh.indices.len(), 12);
        assert!(mesh.vertices.iter().all(|v| (30.0..=170.0).contains(&v.pos.x) && (30.0..=170.0).contains(&v.pos.y)));
    }
}
