//! A disk cache of cooked meshes (issue #34). Cooking a 3 M-triangle prop's cluster DAG takes
//! seconds; loading the result takes milliseconds. A [`MeshletMesh`] is stored as it lies in
//! memory, under a key made from the text of its generator's parameters and
//! [`COOK_VERSION`], so a demo cooks each prop once per change of either.
//!
//! File format (`<name>-<key>.fmesh`, little-endian hosts): a header of
//! `"FGMS"`, the format version, the key, the cook version, the element counts, the triangle
//! counts and the bounding sphere; then the vertices, the meshlet records, the meshlet vertex
//! indices, the meshlet triangle bytes and the clusters per level, each as raw `Pod` bytes.
//! A file that does not match (another key, version or size) is ignored and cooked again;
//! a new file is written beside the old name and renamed into place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bytemuck::Pod;

use crate::meshlet::{CookOptions, GpuMeshlet, GpuVertex, MeshletMesh};
use crate::procedural::TriMesh;

/// Version of what cooking produces: bump it when the DAG builder, the meshlet format or
/// meshoptimizer changes the output, so every cached mesh is cooked again.
pub const COOK_VERSION: u32 = 1;
const MAGIC: [u8; 4] = *b"FGMS";
const FORMAT_VERSION: u32 = 1;

/// FNV-1a over `bytes` (stable across platforms and runs).
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The cache key of a mesh whose generator's parameters print as `key_text`.
pub fn key(key_text: &str) -> u64 {
    fnv1a64(format!("{key_text}#cook{COOK_VERSION}").as_bytes())
}

/// Where the mesh `name` with `key` lives in `dir`.
pub fn path(dir: &Path, name: &str, key: u64) -> PathBuf {
    dir.join(format!("{name}-{key:016x}.fmesh"))
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// The file's bytes for `mesh` under `key`.
fn encode(mesh: &MeshletMesh, key: u64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    push_u32(&mut out, FORMAT_VERSION);
    push_u64(&mut out, key);
    push_u32(&mut out, COOK_VERSION);
    push_u32(&mut out, mesh.vertices.len() as u32);
    push_u32(&mut out, mesh.meshlets.len() as u32);
    push_u32(&mut out, mesh.meshlet_vertices.len() as u32);
    push_u32(&mut out, mesh.meshlet_triangles.len() as u32);
    push_u32(&mut out, mesh.clusters_per_level.len() as u32);
    push_u64(&mut out, mesh.triangle_count as u64);
    push_u64(&mut out, mesh.dag_triangle_count as u64);
    for c in mesh.center {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&mesh.radius.to_le_bytes());
    out.extend_from_slice(bytemuck::cast_slice(&mesh.vertices));
    out.extend_from_slice(bytemuck::cast_slice(&mesh.meshlets));
    out.extend_from_slice(bytemuck::cast_slice(&mesh.meshlet_vertices));
    out.extend_from_slice(&mesh.meshlet_triangles);
    out.extend_from_slice(bytemuck::cast_slice(&mesh.clusters_per_level));
    out
}

/// Reads fields front to back; every read fails cleanly past the end.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.bytes.len() < n {
            return None;
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn array<T: Pod>(&mut self, count: u32) -> Option<Vec<T>> {
        let bytes = self.take(count as usize * size_of::<T>())?;
        let mut out = vec![T::zeroed(); count as usize];
        bytemuck::cast_slice_mut::<T, u8>(&mut out).copy_from_slice(bytes);
        Some(out)
    }
}

/// The mesh in `bytes` if they hold one under `key`, else `None`.
fn decode(bytes: &[u8], key: u64) -> Option<MeshletMesh> {
    let mut r = Reader { bytes };
    if r.take(4)? != MAGIC || r.u32()? != FORMAT_VERSION || r.u64()? != key {
        return None;
    }
    if r.u32()? != COOK_VERSION {
        return None;
    }
    let (vertices, meshlets, meshlet_vertices, triangles, levels) =
        (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let triangle_count = r.u64()? as usize;
    let dag_triangle_count = r.u64()? as usize;
    let center = [r.f32()?, r.f32()?, r.f32()?];
    let radius = r.f32()?;
    let mesh = MeshletMesh {
        vertices: r.array::<GpuVertex>(vertices)?,
        meshlets: r.array::<GpuMeshlet>(meshlets)?,
        meshlet_vertices: r.array::<u32>(meshlet_vertices)?,
        meshlet_triangles: r.array::<u8>(triangles)?,
        clusters_per_level: r.array::<u32>(levels)?,
        triangle_count,
        dag_triangle_count,
        center,
        radius,
    };
    r.bytes.is_empty().then_some(mesh)
}

/// Writes `mesh` to `path` under `key`: into a temporary file first, renamed into place, so
/// an interrupted write never leaves a file that looks complete.
pub fn save(path: &Path, mesh: &MeshletMesh, key: u64) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("fmesh.partial");
    fs::write(&partial, encode(mesh, key))?;
    fs::rename(&partial, path)
}

/// The mesh stored at `path` under `key`, or `None` when there is none (or another one).
pub fn load(path: &Path, key: u64) -> Option<MeshletMesh> {
    decode(&fs::read(path).ok()?, key)
}

/// Removes the files of mesh `name` cached under another key than `key` (earlier parameters
/// or cook versions), so the cache holds one file per mesh.
fn remove_stale(dir: &Path, name: &str, key: u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let keep = format!("{key:016x}");
    for entry in entries.flatten() {
        let file = entry.file_name();
        let Some(rest) = file.to_str().and_then(|f| f.strip_prefix(name)) else {
            continue;
        };
        // `-<16 hex digits>.fmesh`, exactly: `boulder-1` must not take `boulder-10`'s files.
        let stale = rest
            .strip_prefix('-')
            .and_then(|r| r.strip_suffix(".fmesh"))
            .is_some_and(|k| {
                k.len() == 16 && k.chars().all(|c| c.is_ascii_hexdigit()) && k != keep
            });
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A mesh and how it was obtained.
pub struct Cooked {
    /// The cooked mesh.
    pub mesh: MeshletMesh,
    /// Read from the cache (else generated and cooked, then stored).
    pub from_cache: bool,
    /// Milliseconds spent: loading, or generating plus cooking.
    pub ms: f64,
}

/// The mesh `name` from `dir` when cached under its parameters `key_text`; otherwise
/// `generate`s it, cooks it with `options` ([`MeshletMesh::build_with`]; they belong in
/// `key_text`) and stores it (a failed write is
/// logged by the caller's choice: it only costs the next run a cook).
pub fn cook_cached(
    dir: &Path,
    name: &str,
    key_text: &str,
    options: CookOptions,
    generate: impl FnOnce() -> TriMesh,
) -> (Cooked, io::Result<()>) {
    let start = Instant::now();
    let key = key(key_text);
    let file = path(dir, name, key);
    if let Some(mesh) = load(&file, key) {
        let ms = start.elapsed().as_secs_f64() * 1e3;
        return (
            Cooked {
                mesh,
                from_cache: true,
                ms,
            },
            Ok(()),
        );
    }
    let mesh = MeshletMesh::build_with(&generate(), options);
    let ms = start.elapsed().as_secs_f64() * 1e3;
    let stored = save(&file, &mesh, key).map(|()| remove_stale(dir, name, key));
    (
        Cooked {
            mesh,
            from_cache: false,
            ms,
        },
        stored,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural::asteroid;
    use forge_core::Seed;

    fn small() -> MeshletMesh {
        MeshletMesh::build(&asteroid(Seed::new(3), 12, 1.0, 0.3))
    }

    #[test]
    fn a_mesh_comes_back_as_it_went_in() {
        let mesh = small();
        let back = decode(&encode(&mesh, 7), 7).expect("decodes");
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&back.vertices),
            bytemuck::cast_slice::<_, u8>(&mesh.vertices)
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&back.meshlets),
            bytemuck::cast_slice::<_, u8>(&mesh.meshlets)
        );
        assert_eq!(back.meshlet_vertices, mesh.meshlet_vertices);
        assert_eq!(back.meshlet_triangles, mesh.meshlet_triangles);
        assert_eq!(back.clusters_per_level, mesh.clusters_per_level);
        assert_eq!(back.triangle_count, mesh.triangle_count);
        assert_eq!(back.dag_triangle_count, mesh.dag_triangle_count);
        assert_eq!(back.center, mesh.center);
        assert_eq!(back.radius, mesh.radius);
    }

    #[test]
    fn another_key_a_short_file_or_a_long_one_is_no_mesh() {
        let bytes = encode(&small(), 7);
        assert!(decode(&bytes, 8).is_none());
        assert!(decode(&bytes[..bytes.len() - 1], 7).is_none());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer, 7).is_none());
    }

    #[test]
    fn a_new_cook_replaces_the_stale_files_of_its_mesh_only() {
        let dir = std::env::temp_dir().join(format!("forge-cache-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mesh = small();
        for (name, key) in [("rock-1", 1), ("rock-1", 2), ("rock-10", 3)] {
            save(&path(&dir, name, key), &mesh, key).expect("saved");
        }
        remove_stale(&dir, "rock-1", 2);
        assert!(!path(&dir, "rock-1", 1).exists(), "the stale file goes");
        assert!(path(&dir, "rock-1", 2).exists(), "the current one stays");
        assert!(path(&dir, "rock-10", 3).exists(), "another mesh's stays");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_follow_the_parameters() {
        assert_eq!(key("a"), key("a"));
        assert_ne!(key("a"), key("b"));
    }
}
