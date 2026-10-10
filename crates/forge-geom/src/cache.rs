//! A disk cache of cooked meshes (issue #34). Cooking a 3 M-triangle prop's cluster DAG takes
//! seconds; loading the result takes milliseconds. A [`MeshletMesh`] is stored as it lies in
//! memory, its pages packed, under a key made from the text of its generator's parameters and
//! [`COOK_VERSION`], so a demo cooks each prop once per change of either.
//!
//! File format (`<name>-<key>.fmesh`, little-endian hosts):
//! - A header: `"FGMS"`, the format version, the key, the cook version, the element counts, the
//!   root pages, the triangle counts, the bounding sphere and the flags (1: the payloads carry
//!   UVs, D-047).
//! - The meshlet records and the clusters per level, as raw `Pod` bytes.
//! - The page table: per page its packed length, and its codec in the top byte.
//! - The pages (issue #36), each packed on its own (#215, `forge_core::pack`: LZ4, on its
//!   16-byte records' planes when that is smaller, or as is), so a page can still be read alone.
//!
//! A file that does not match (another key, version or size) is ignored and cooked again. A new
//! file is written beside the old name and renamed into place.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use bytemuck::Pod;
use forge_core::pack::{Codec, pack, par_chunks_mut, unpack};

use crate::meshlet::{CookOptions, GpuMeshlet, MeshletMesh, PackedPage, PageFile};
use crate::page::PAGE_SIZE;
use crate::procedural::TriMesh;

/// Version of what cooking produces: bump it when the DAG builder, the meshlet format or
/// meshoptimizer changes the output, so every cached mesh is cooked again.
pub const COOK_VERSION: u32 = 5;
const MAGIC: [u8; 4] = *b"FGMS";
/// 4: the pages packed (#215).
const FORMAT_VERSION: u32 = 4;

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

/// Bytes of the header, before the meshlet records.
const HEADER_BYTES: usize = 72;

/// Where the page table starts in a file of `meshlets` records and `levels` levels.
fn table_offset(meshlets: u32, levels: u32) -> usize {
    HEADER_BYTES + meshlets as usize * size_of::<GpuMeshlet>() + levels as usize * 4
}

/// A page's entry in the table: its packed length, and its codec in the top byte.
fn table_entry(codec: Codec, len: usize) -> u32 {
    len as u32 | (codec as u32) << 24
}

/// Each page's place in the file from the `table`'s entries, the pages starting at `start`;
/// `None` if an entry is not one a page can have.
fn page_table(table: &[u8], start: u64) -> Option<Vec<PackedPage>> {
    let mut offset = start;
    table
        .as_chunks::<4>()
        .0
        .iter()
        .map(|entry| {
            let entry = u32::from_le_bytes(*entry);
            let len = entry & 0x00ff_ffff;
            let codec = Codec::from_byte((entry >> 24) as u8).ok()?;
            if len as usize > PAGE_SIZE {
                return None;
            }
            let page = PackedPage { offset, len, codec };
            offset += u64::from(len);
            Some(page)
        })
        .collect()
}

/// The file's bytes for `mesh` under `key` (its pages must be in memory).
fn encode(mesh: &MeshletMesh, key: u64) -> Vec<u8> {
    assert_eq!(
        mesh.pages.len(),
        mesh.page_count as usize * PAGE_SIZE,
        "a mesh is stored with its pages"
    );
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    push_u32(&mut out, FORMAT_VERSION);
    push_u64(&mut out, key);
    push_u32(&mut out, COOK_VERSION);
    push_u32(&mut out, mesh.meshlets.len() as u32);
    push_u32(&mut out, mesh.clusters_per_level.len() as u32);
    push_u32(&mut out, mesh.page_count);
    push_u32(&mut out, mesh.root_pages);
    push_u64(&mut out, mesh.triangle_count as u64);
    push_u64(&mut out, mesh.dag_triangle_count as u64);
    for c in mesh.center {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&mesh.radius.to_le_bytes());
    push_u32(&mut out, u32::from(mesh.uvs));
    assert_eq!(out.len(), HEADER_BYTES);
    out.extend_from_slice(bytemuck::cast_slice(&mesh.meshlets));
    out.extend_from_slice(bytemuck::cast_slice(&mesh.clusters_per_level));
    let mut packed = vec![(Codec::Raw, Vec::new()); mesh.page_count as usize];
    par_chunks_mut(&mut packed, 1, 4, |p, page| {
        page[0] = pack(&mesh.pages[p * PAGE_SIZE..(p + 1) * PAGE_SIZE], true);
    });
    for (codec, bytes) in &packed {
        push_u32(&mut out, table_entry(*codec, bytes.len()));
    }
    for (_, bytes) in &packed {
        out.extend_from_slice(bytes);
    }
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

/// The mesh whose header, records and page table start `bytes` if they hold one under `key`,
/// without its pages (`page_file` unset), with where each page lies and the file size it
/// needs; else `None`.
fn decode_hierarchy(bytes: &[u8], key: u64) -> Option<(MeshletMesh, Vec<PackedPage>, u64)> {
    let mut r = Reader { bytes };
    if r.take(4)? != MAGIC || r.u32()? != FORMAT_VERSION || r.u64()? != key {
        return None;
    }
    if r.u32()? != COOK_VERSION {
        return None;
    }
    let (meshlets, levels, pages, root_pages) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let triangle_count = r.u64()? as usize;
    let dag_triangle_count = r.u64()? as usize;
    let center = [r.f32()?, r.f32()?, r.f32()?];
    let radius = r.f32()?;
    let uvs = r.u32()? & 1 != 0;
    let mesh = MeshletMesh {
        meshlets: r.array::<GpuMeshlet>(meshlets)?,
        clusters_per_level: r.array::<u32>(levels)?,
        pages: Vec::new(),
        page_count: pages,
        root_pages,
        page_file: None,
        triangle_count,
        dag_triangle_count,
        center,
        radius,
        uvs,
    };
    let start = (table_offset(meshlets, levels) + pages as usize * 4) as u64;
    let table = page_table(r.take(pages as usize * 4)?, start)?;
    let size = table.last().map_or(start, |p| p.offset + u64::from(p.len));
    Some((mesh, table, size))
}

/// The mesh in `bytes` if they hold one under `key`, pages included, else `None`.
fn decode(bytes: &[u8], key: u64) -> Option<MeshletMesh> {
    let (mut mesh, table, size) = decode_hierarchy(bytes, key)?;
    if bytes.len() as u64 != size {
        return None;
    }
    let mut pages = vec![0; table.len() * PAGE_SIZE];
    let failed = AtomicBool::new(false);
    par_chunks_mut(&mut pages, PAGE_SIZE, 8, |p, out| {
        let page = table[p];
        let packed = &bytes[page.offset as usize..][..page.len as usize];
        if unpack(page.codec, packed, out).is_err() {
            failed.store(true, Ordering::Relaxed);
        }
    });
    if failed.into_inner() {
        return None;
    }
    mesh.pages = pages;
    Some(mesh)
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

/// The mesh stored at `path` under `key` without its pages, which stay in the file for a
/// streamer to read one at a time ([`MeshletMesh::page_file`]); `None` when there is no such
/// mesh or the file is not the size its header says.
pub fn load_hierarchy(path: &Path, key: u64) -> Option<MeshletMesh> {
    use std::io::Read;
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut head = vec![0; HEADER_BYTES];
    file.read_exact(&mut head).ok()?;
    let count = |at: usize| Some(u32::from_le_bytes(head[at..at + 4].try_into().ok()?));
    let (meshlets, levels, pages) = (count(20)?, count(24)?, count(28)?);
    let records = table_offset(meshlets, levels) + pages as usize * 4;
    if (len as usize) < records {
        return None;
    }
    head.resize(records, 0);
    file.read_exact(&mut head[HEADER_BYTES..]).ok()?;
    let (mut mesh, table, size) = decode_hierarchy(&head, key)?;
    if len != size {
        return None;
    }
    mesh.page_file = Some(PageFile {
        path: path.to_path_buf(),
        pages: table,
    });
    Some(mesh)
}

/// What each mesh cache directory held when this process first stored into it, by mesh, kept up
/// to date with the process's own stores (#220). Files of other keys can only come from earlier
/// processes, so one listing finds them all and a store reads no directory: a planet's tiles keep
/// thousands of meshes.
static LISTINGS: Mutex<Vec<(PathBuf, MeshFiles)>> = Mutex::new(Vec::new());

/// A directory's cache files, by the mesh they hold.
type MeshFiles = HashMap<String, Vec<String>>;

/// Removes the files of mesh `name` cached under another key than `key` (earlier parameters
/// or cook versions), so the cache holds one file per mesh.
fn remove_stale(dir: &Path, name: &str, key: u64) {
    let keep = format!("{name}-{key:016x}.fmesh");
    let mut listings = LISTINGS.lock().unwrap_or_else(|e| e.into_inner());
    let at = match listings.iter().position(|(listed, _)| listed == dir) {
        Some(at) => at,
        None => {
            let mut by_mesh = MeshFiles::new();
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                if let Some(file) = entry.file_name().to_str()
                    && let Some(mesh) = mesh_of(file)
                {
                    by_mesh
                        .entry(mesh.to_owned())
                        .or_default()
                        .push(file.to_owned());
                }
            }
            listings.push((dir.to_path_buf(), by_mesh));
            listings.len() - 1
        }
    };
    let files = listings[at].1.entry(name.to_owned()).or_default();
    files.retain(|file| {
        let current = *file == keep;
        if !current {
            let _ = fs::remove_file(dir.join(file));
        }
        current
    });
    if files.is_empty() {
        files.push(keep);
    }
}

/// The mesh a cache file holds, from its name `<mesh>-<16 hex digits>.fmesh`, exactly:
/// `boulder-1` must not take `boulder-10`'s files.
fn mesh_of(file: &str) -> Option<&str> {
    let (mesh, key) = file.strip_suffix(".fmesh")?.rsplit_once('-')?;
    (key.len() == 16 && key.chars().all(|c| c.is_ascii_hexdigit())).then_some(mesh)
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
/// logged by the caller's choice: it only costs the next run a cook). With
/// `pages_in_memory` false the pages stay in the cache file ([`load_hierarchy`]), unless
/// the file could not be written.
pub fn cook_cached(
    dir: &Path,
    name: &str,
    key_text: &str,
    options: CookOptions,
    pages_in_memory: bool,
    generate: impl FnOnce() -> TriMesh,
) -> (Cooked, io::Result<()>) {
    cook_cached_with(dir, name, key_text, pages_in_memory, || {
        MeshletMesh::build_with(&generate(), options)
    })
}

/// [`cook_cached`] with the caller's own cook: `cook` makes and cooks the mesh when the cache
/// lacks it (a planet's tile clustered by its cells, [`MeshletMesh::build_clustered`]). How it
/// cooks belongs in `key_text`.
pub fn cook_cached_with(
    dir: &Path,
    name: &str,
    key_text: &str,
    pages_in_memory: bool,
    cook: impl FnOnce() -> MeshletMesh,
) -> (Cooked, io::Result<()>) {
    let start = Instant::now();
    let key = key(key_text);
    let file = path(dir, name, key);
    // The props nothing has asked for in a month go (#208), once a process.
    forge_core::derived::sweep_once(dir, &["fmesh"], forge_core::derived::UNUSED_FOR);
    let cached = if pages_in_memory {
        load(&file, key)
    } else {
        load_hierarchy(&file, key)
    };
    if let Some(mesh) = cached {
        // Its last use, for the sweep.
        forge_core::derived::touch(&file);
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
    let mut mesh = cook();
    let ms = start.elapsed().as_secs_f64() * 1e3;
    let stored = save(&file, &mesh, key).map(|()| remove_stale(dir, name, key));
    if stored.is_ok() && !pages_in_memory {
        // Where the packed pages lie, as the next start reads it.
        if let Some(streamed) = load_hierarchy(&file, key) {
            mesh.pages = Vec::new();
            mesh.page_file = streamed.page_file;
        }
    }
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
        let bytes = encode(&mesh, 7);
        let back = decode(&bytes, 7).expect("decodes");
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&back.meshlets),
            bytemuck::cast_slice::<_, u8>(&mesh.meshlets)
        );
        assert_eq!(back.pages, mesh.pages);
        assert_eq!(back.root_pages, mesh.root_pages);
        // The pages are packed: the file is smaller than they are.
        assert!(bytes.len() < mesh.pages.len(), "{} bytes", bytes.len());
        assert_eq!(back.clusters_per_level, mesh.clusters_per_level);
        assert_eq!(back.triangle_count, mesh.triangle_count);
        assert_eq!(back.dag_triangle_count, mesh.dag_triangle_count);
        assert_eq!(back.center, mesh.center);
        assert_eq!(back.radius, mesh.radius);
        assert!(!back.uvs);
        // A mesh with UVs keeps its flag.
        let mut textured = asteroid(Seed::new(3), 12, 1.0, 0.3);
        textured.uvs = textured.positions.iter().map(|p| [p[0], p[1]]).collect();
        let mesh = MeshletMesh::build(&textured);
        let back = decode(&encode(&mesh, 7), 7).expect("decodes");
        assert!(back.uvs);
        assert_eq!(back.pages, mesh.pages);
    }

    #[test]
    fn a_streamed_mesh_leaves_its_pages_in_the_file() {
        let dir = std::env::temp_dir().join(format!("forge-cache-pages-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mesh = small();
        let file = path(&dir, "rock", 5);
        save(&file, &mesh, 5).expect("saved");
        let streamed = load_hierarchy(&file, 5).expect("the hierarchy loads");
        assert!(streamed.pages.is_empty());
        assert_eq!(streamed.page_count, mesh.page_count);
        assert_eq!(streamed.root_pages, mesh.root_pages);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&streamed.meshlets),
            bytemuck::cast_slice::<_, u8>(&mesh.meshlets)
        );
        // Each page unpacks on its own from where the table says it lies.
        let pages = streamed.page_file.expect("a page file");
        let bytes = fs::read(&pages.path).expect("read");
        assert_eq!(pages.pages.len(), mesh.page_count as usize);
        for (p, page) in pages.pages.iter().enumerate() {
            let mut out = vec![0; PAGE_SIZE];
            let packed = &bytes[page.offset as usize..][..page.len as usize];
            unpack(page.codec, packed, &mut out).expect("unpacks");
            assert_eq!(out, mesh.pages[p * PAGE_SIZE..(p + 1) * PAGE_SIZE]);
        }
        assert!(load_hierarchy(&file, 6).is_none(), "another key");
        let _ = fs::remove_dir_all(&dir);
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
