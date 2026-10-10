//! The planet's tiles as the camera flies (#220): the cut made again around where the camera is
//! going, and the drawn scene edited in place to it, on a worker.
//!
//! - **The cut** follows the points the demo gives every [`ASK_EVERY`] seconds: the camera where
//!   it will be when the change is ready, and the place it is heading for (the tour's next stop,
//!   the descent's target), so a stop's tiles are made before the camera gets there. The worker
//!   makes it by the swap rule ([`forge_terrain::PlanetWorld::tile_cut_by_error`]): a cell splits
//!   where its samples would stand too far apart on ground rough enough to show it, or where its
//!   children's height would show. It keeps the cells' errors it worked out ([`CellErrors`]).
//! - **The worker** keeps the tiles of the drawn cut; a new cut reuses them and cooks or loads
//!   the tiles it lacks from the cache, on a pool of a quarter of the hardware threads, so the
//!   frames keep theirs.
//! - **In place** (a streamed scene, [`forge_render::SceneEditor`]): the worker removes the
//!   tiles the new cut drops and adds the ones it brings, each with its own cut for the rays,
//!   its structure, its normal map and the pages its first view wants; the frame takes the edit
//!   in at its start ([`MeshletScene::apply`]). Nothing else changes: no TAA reset, the same sky.
//! - **Whole:** the first scene, a scene with every page resident (`--resident`), and a scene
//!   whose room an edit would overflow are built whole and swapped in, the old one freed once no
//!   frame in flight reads it.
//!
//! `docs/demos/planet.md` has the numbers.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use anyhow::Result;
use forge_core::MaterialId;
use forge_geom::{CookOptions, MeshletMesh};
use forge_gpu::{Device, ShaderCompiler};
use forge_render::material::GpuMaterial;
use forge_render::material::{TextureSet, upload_textures};
use forge_render::textures::TextureData;
use forge_render::{
    CellPos, DynamicCapacity, MeshId, MeshRays, MeshletScene, MeshletSceneBuilder, Residency,
    SceneEdit, SceneEditor, StartView,
};
use forge_task::{PoolConfig, TaskPool};
use forge_terrain::Planet;
use forge_terrain::planet::{
    NORMAL_MAP_SIZE, normal_map_levels, tile_clusters, tile_mesh, tile_name,
    tile_normal_map_cached, tile_origin,
};
use forge_world::CellId;
use glam::DVec3;

use crate::Placement;

/// Triangles of a tile's cut for the rays. Each tile is cut on its own, so that a tile added in
/// place is cut as in a scene built whole, whichever tiles stand beside it.
const TILE_RAY_BUDGET: u32 = 4_000;

/// Seconds between two asks for the cut: the worker makes it again (a millisecond of its time)
/// at most this often.
const ASK_EVERY: f64 = 0.05;

/// Tiles a scene has room for at least: the tour's cuts hold up to about 700, and an edit needs
/// room for the tiles it adds before those it removes are freed.
const TILE_ROOM: usize = 1_536;

/// What every scene of the run is built from.
#[derive(Clone)]
pub(crate) struct SceneParts {
    pub planet: Arc<Planet>,
    /// Its name in the cache (`earth`, `moon`).
    pub body: String,
    pub placement: Placement,
    /// The material table's rows: the textures they sample live in the demo, for every scene.
    pub rows: Vec<GpuMaterial>,
    /// The ground's row.
    pub ground: MaterialId,
    /// Whether the scenes have rays (the sun's shadows).
    pub ray_traced: bool,
    pub residency: Residency,
}

impl SceneParts {
    /// Whether the scenes are edited in place: when they stream their pages.
    pub(crate) fn in_place(&self) -> bool {
        matches!(self.residency, Residency::Streamed(_))
    }
}

/// Where a tile of a new cut came from.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Kept,
    Loaded,
    Cooked,
}

/// A tile of a cut: its cooked mesh and its normal map ([`tile_normal_map`]).
#[derive(Clone)]
pub(crate) struct Tile {
    pub mesh: Arc<MeshletMesh>,
    normals: NormalMap,
}

/// A tile's normal map: its first level until a scene puts it on the GPU, then its texture.
#[derive(Clone)]
enum NormalMap {
    Texels(Arc<Vec<u8>>),
    Texture(Arc<TileTexture>),
}

/// A tile's normal map on the GPU: a texture set of its own, which the scenes drawing the tile
/// keep alive as long as its instance stands in them.
pub(crate) struct TileTexture {
    _set: TextureSet,
    sampled: u32,
}

/// The tiles of `cells`: those in `have` as they are, the others loaded from the cache under
/// `body`'s name or made and cooked on `pool`, their pages in memory or left in the cache's file,
/// with their normal maps. Also how many were cooked, and how many loaded.
pub(crate) fn cook_cells(
    planet: &Planet,
    body: &str,
    in_memory: bool,
    cells: &[CellId],
    have: &HashMap<CellId, Tile>,
    pool: &TaskPool,
) -> (Vec<Tile>, usize, usize) {
    let cache = forge_core::derived::cache_dir(forge_core::derived::CacheKind::Meshes);
    let key = planet.tile_key();
    let clusters = &tile_clusters(planet.world.tiles.samples);
    let mut slots: Vec<Option<(Tile, Source)>> = cells
        .iter()
        .map(|c| have.get(c).map(|t| (t.clone(), Source::Kept)))
        .collect();
    pool.scope(|s| {
        for (&cell, slot) in cells.iter().zip(slots.iter_mut()) {
            if slot.is_some() {
                continue;
            }
            let (cache, key) = (&cache, &key);
            s.spawn(move |_| {
                let name = tile_name(body, cell);
                // Its first level cut into its grid's cells, its DAG over them.
                let (done, stored) =
                    forge_geom::cache::cook_cached_with(cache, &name, key, in_memory, || {
                        MeshletMesh::build_clustered(
                            &tile_mesh(planet, cell),
                            clusters,
                            CookOptions { normal_weight: 0.0 },
                        )
                    });
                if let Err(error) = stored {
                    tracing::warn!(tile = %name, %error, "cooked tile not cached");
                }
                let source = if done.from_cache {
                    Source::Loaded
                } else {
                    Source::Cooked
                };
                let normals = tile_normal_map_cached(planet, body, cell);
                *slot = Some((
                    Tile {
                        mesh: Arc::new(done.mesh),
                        normals: NormalMap::Texels(Arc::new(normals)),
                    },
                    source,
                ));
            });
        }
    });
    let (mut cooked, mut loaded) = (0, 0);
    let tiles = slots
        .into_iter()
        .map(|slot| {
            let (tile, source) = slot.expect("every tile cooked");
            cooked += usize::from(source == Source::Cooked);
            loaded += usize::from(source == Source::Loaded);
            tile
        })
        .collect();
    (tiles, cooked, loaded)
}

/// Puts the normal maps of `tiles` (the tiles of `cells`) not on the GPU yet there, together: a
/// few submissions, not one a map.
fn upload_normal_maps(
    device: &Arc<Device>,
    parts: &SceneParts,
    cells: &[CellId],
    tiles: &mut [Tile],
) -> Result<()> {
    let new: Vec<usize> = tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t.normals, NormalMap::Texels(_)))
        .map(|(i, _)| i)
        .collect();
    let data: Vec<TextureData> = new
        .iter()
        .map(|&i| {
            let NormalMap::Texels(texels) = &tiles[i].normals else {
                unreachable!("a new tile's map is its texels")
            };
            TextureData {
                name: format!("{} normals", tile_name(&parts.body, cells[i])).into(),
                size: NORMAL_MAP_SIZE,
                height: NORMAL_MAP_SIZE,
                srgb: false,
                levels: normal_map_levels(texels.as_ref().clone(), NORMAL_MAP_SIZE),
            }
        })
        .collect();
    for (&i, (image, bytes)) in new.iter().zip(upload_textures(device, &data)?) {
        let mut set = TextureSet::new(device);
        let id = set.add_uploaded(image, bytes);
        let sampled = set.sampled(id);
        tiles[i].normals = NormalMap::Texture(Arc::new(TileTexture { _set: set, sampled }));
    }
    Ok(())
}

/// The room a scene reserves for tiles like `tiles`: twice as many tiles as they are, a
/// thousand at least, each with as many clusters as the largest of them and twice its pages
/// (page numbers cost a few bytes each, and tiles coming and going leave gaps between those in
/// use), and its rays' cut.
pub(crate) fn capacity_for(tiles: &[Tile]) -> DynamicCapacity {
    let room = (2 * tiles.len()).max(TILE_ROOM) as u32;
    let largest =
        |size: fn(&MeshletMesh) -> u32| tiles.iter().map(|t| size(&t.mesh)).max().unwrap_or(0);
    DynamicCapacity {
        meshes: room,
        instances: room,
        meshlets: room * largest(|m| m.meshlets.len() as u32),
        pages: 2 * room * largest(|m| m.page_count),
        // A cut's vertices are about as many as its triangles; half again for room.
        ray_vertices: room * TILE_RAY_BUDGET * 3 / 2,
        ray_triangles: room * TILE_RAY_BUDGET,
    }
}

/// The scene of `cells` (their tiles in `tiles`, in the same order), its pages for `start`
/// loaded when it streams them, with room for `capacity` when it is to be edited in place. The
/// tiles' normal maps not on the GPU yet go there.
pub(crate) fn build_scene(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    parts: &SceneParts,
    cells: &[CellId],
    tiles: &mut [Tile],
    start: StartView,
    capacity: Option<DynamicCapacity>,
) -> Result<MeshletScene> {
    let world = &parts.planet.world;
    upload_normal_maps(device, parts, cells, tiles)?;
    let mut builder = MeshletSceneBuilder::new();
    builder.set_material_rows(parts.rows.clone());
    builder.set_ray_traced(parts.ray_traced);
    if let Some(capacity) = capacity {
        builder.reserve_dynamic(capacity);
    }
    let ids: Vec<_> = tiles.iter().map(|t| builder.add_mesh(&t.mesh)).collect();
    // Every tile is ground to the rays, the finest under a kilometre across too, each cut on its
    // own (`TILE_RAY_BUDGET`).
    builder.set_ray_terrain(&ids);
    builder.set_ray_budget(&ids, TILE_RAY_BUDGET);
    let rotation = parts.placement.rotation.as_quat();
    for (i, ((&cell, &id), tile)) in cells.iter().zip(&ids).zip(tiles.iter()).enumerate() {
        let at = parts.placement.world_point(tile_origin(world, cell));
        builder.add_instance_at(id, CellPos::from_f64(at), rotation, 1.0, parts.ground);
        if let NormalMap::Texture(texture) = &tile.normals {
            builder.set_instance_texture(i, texture.sampled);
            builder.keep_for(i, Arc::clone(texture));
        }
    }
    // A streamed scene loads its start view's pages before its first frame (#121): the shots'
    // frames are the same every run, and a swapped-in scene shows no coarser ground.
    builder.set_start_view(start);
    let started = Instant::now();
    let mut scene = builder.build_with(device, parts.residency)?;
    let tables = started.elapsed().as_secs_f64() * 1e3;
    scene.build_tlas(device, shaders)?;
    let tlas = started.elapsed().as_secs_f64() * 1e3 - tables;
    scene.load_start_view(device)?;
    tracing::debug!(
        tables_ms = %format_args!("{tables:.0}"),
        tlas_ms = %format_args!("{tlas:.0}"),
        pages_ms = %format_args!("{:.0}", started.elapsed().as_secs_f64() * 1e3 - tables - tlas),
        "a scene of the tiles built"
    );
    Ok(scene)
}

/// A tile standing in the drawn scene, as the worker knows it.
pub(crate) struct LiveTile {
    tile: Tile,
    mesh: MeshId,
    instance: u32,
}

/// The tiles of a scene built whole from `cells` (their tiles `tiles`): mesh and instance `i`
/// the tile of `cells[i]`.
pub(crate) fn live_tiles(cells: &[CellId], tiles: Vec<Tile>) -> HashMap<CellId, LiveTile> {
    cells
        .iter()
        .zip(tiles)
        .enumerate()
        .map(|(i, (&cell, tile))| {
            (
                cell,
                LiveTile {
                    tile,
                    mesh: MeshId::from_index(i as u32),
                    instance: i as u32,
                },
            )
        })
        .collect()
}

/// Edits the scene `editor` draws, whose tiles are `live`, to the cut `cells` (their tiles
/// `tiles`, in the same order): the tiles it drops removed, the ones it brings added with the
/// pages `start` wants of them. `live` follows. Returns the edit and how many tiles it adds and
/// removes.
pub(crate) fn edit_scene(
    device: &Arc<Device>,
    parts: &SceneParts,
    editor: &mut SceneEditor,
    live: &mut HashMap<CellId, LiveTile>,
    cells: &[CellId],
    tiles: &[Tile],
    start: &StartView,
) -> Result<(SceneEdit, usize, usize)> {
    let world = &parts.planet.world;
    let wanted: HashSet<CellId> = cells.iter().copied().collect();
    let mut removed = 0;
    live.retain(|cell, tile| {
        let keep = wanted.contains(cell);
        if !keep {
            editor.remove_instance(tile.instance);
            editor.remove_mesh(tile.mesh);
            removed += 1;
        }
        keep
    });
    let new: Vec<usize> = (0..cells.len())
        .filter(|&i| !live.contains_key(&cells[i]))
        .collect();
    let new_cells: Vec<CellId> = new.iter().map(|&i| cells[i]).collect();
    let mut new_tiles: Vec<Tile> = new.iter().map(|&i| tiles[i].clone()).collect();
    upload_normal_maps(device, parts, &new_cells, &mut new_tiles)?;
    let rotation = parts.placement.rotation.as_quat();
    let rays = MeshRays {
        budget: TILE_RAY_BUDGET,
        terrain: true,
    };
    for (&cell, tile) in new_cells.iter().zip(new_tiles) {
        let mesh = editor.add_mesh(&tile.mesh, rays)?;
        let at = parts.placement.world_point(tile_origin(world, cell));
        let instance =
            editor.add_instance_at(mesh, CellPos::from_f64(at), rotation, 1.0, parts.ground)?;
        if let NormalMap::Texture(texture) = &tile.normals {
            editor.set_instance_texture(instance, texture.sampled);
            editor.keep(instance, Arc::clone(texture));
        }
        live.insert(
            cell,
            LiveTile {
                tile,
                mesh,
                instance,
            },
        );
    }
    let edit = editor.finish(Some(start))?;
    Ok((edit, new_cells.len(), removed))
}

/// Whether the drawn cut `current` should give way to `wanted`: somewhere it is coarser, or it
/// holds half as many tiles again. Finer tiles where they are no longer needed cost little (their
/// DAGs coarsen with distance), so a cut that would only coarsen waits for the next that refines.
pub(crate) fn needs_new_cut(current: &[CellId], wanted: &[CellId]) -> bool {
    if current == wanted {
        return false;
    }
    if 2 * current.len() > 3 * wanted.len() {
        return true;
    }
    let have: HashSet<CellId> = current.iter().copied().collect();
    // A wanted cell whose ancestor the drawn cut holds: the drawn one is coarser there.
    wanted.iter().any(|&cell| {
        let mut at = cell;
        while let Some(parent) = at.parent() {
            if have.contains(&parent) {
                return true;
            }
            at = parent;
        }
        false
    })
}

/// What the worker made of a new cut: a scene built whole, or an edit of the drawn one.
pub(crate) enum Change {
    Scene(Box<MeshletScene>),
    Edit {
        edit: Box<SceneEdit>,
        added: usize,
        removed: usize,
    },
}

/// A new cut the worker made ready.
pub(crate) struct Built {
    pub change: Change,
    pub cells: Vec<CellId>,
    /// Tiles cooked and loaded from the cache for it; the rest it kept from the scene before.
    pub cooked: usize,
    pub loaded: usize,
    /// Seconds from the ask to the change ready, and of them the tiles' and the change's.
    pub seconds: f64,
    pub cook_seconds: f64,
    pub build_seconds: f64,
}

/// Where the cut is wanted around, and the view its pages are loaded for.
struct Request {
    /// The points the cut is made around ([`forge_terrain::PlanetWorld::tile_cut_by_error`]) and the view's
    /// pixels a radian.
    points: Vec<DVec3>,
    pixels_per_radian: f64,
    start: StartView,
    asked: Instant,
    /// Build the scene whole, whatever the room: the drawn scene refused the last edit.
    whole: bool,
}

/// The cells' errors ([`Planet::cell_error`]) worked out so far: the swap rule's cut asks for a
/// level's at a time, and those it lacks are worked out in parallel. Each is a function of its
/// cell alone, so the cut is the same whichever were known.
#[derive(Default)]
pub(crate) struct CellErrors(HashMap<CellId, f64>);

impl CellErrors {
    /// The errors of `cells`, those not known yet worked out on `pool`.
    pub fn of(&mut self, planet: &Planet, cells: &[CellId], pool: &TaskPool) -> Vec<f64> {
        let missing: Vec<CellId> = cells
            .iter()
            .copied()
            .filter(|c| !self.0.contains_key(c))
            .collect();
        if !missing.is_empty() {
            let mut found = vec![0.0; missing.len()];
            pool.scope(|s| {
                for (cells, out) in missing.chunks(8).zip(found.chunks_mut(8)) {
                    s.spawn(move |_| {
                        for (&cell, error) in cells.iter().zip(out) {
                            *error = planet.cell_error(cell);
                        }
                    });
                }
            });
            self.0.extend(missing.into_iter().zip(found));
        }
        cells.iter().map(|c| self.0[c]).collect()
    }
}

/// The swap rule's cut of `planet` around `points` seen at `pixels_per_radian`
/// ([`forge_terrain::PlanetWorld::tile_cut_by_error`]), the errors it lacks worked out on `pool`.
pub(crate) fn cut_around(
    planet: &Planet,
    points: &[DVec3],
    pixels_per_radian: f64,
    errors: &mut CellErrors,
    pool: &TaskPool,
) -> Vec<CellId> {
    planet
        .world
        .tile_cut_by_error(points, pixels_per_radian, |cells| {
            errors.of(planet, cells, pool)
        })
}

/// The worker that makes the new cuts (see the module notes).
pub(crate) struct TileStream {
    requests: Option<mpsc::Sender<Request>>,
    built: mpsc::Receiver<Result<Option<Built>>>,
    worker: Option<thread::JoinHandle<()>>,
    /// The cut of the scene drawn.
    pub current: Vec<CellId>,
    /// An ask is being answered.
    pending: bool,
    /// When the last ask went: the cut is made again at most every [`ASK_EVERY`] seconds.
    asked: Option<Instant>,
    /// The next ask builds the scene whole.
    whole: bool,
    /// How long the last change took, seconds: how far ahead to ask for the next.
    pub latency: f64,
    /// Changes taken in so far.
    pub swaps: u32,
}

/// The worker's state: the drawn scene's editor (a scene edited in place), its tiles and its
/// cut, and the cells' errors known.
struct WorkerScene {
    editor: Option<SceneEditor>,
    live: HashMap<CellId, LiveTile>,
    cells: Vec<CellId>,
    errors: CellErrors,
}

impl TileStream {
    /// Starts the worker, which holds the drawn scene's cut `current`, its tiles `live` and its
    /// `editor` (one edited in place), and the cells' errors known.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: Arc<Device>,
        shaders: ShaderCompiler,
        parts: SceneParts,
        current: Vec<CellId>,
        live: HashMap<CellId, LiveTile>,
        editor: Option<SceneEditor>,
        errors: CellErrors,
        latency: f64,
    ) -> Result<Self> {
        let (requests, asked) = mpsc::channel::<Request>();
        let (done, built) = mpsc::channel::<Result<Option<Built>>>();
        let cells = current.clone();
        let worker = thread::Builder::new()
            .name("planet tiles".into())
            .spawn(move || {
                // A quarter of the hardware threads, half the cores: the frames keep theirs.
                let workers = thread::available_parallelism()
                    .map_or(2, |n| n.get() / 4)
                    .max(1);
                let pool = TaskPool::new(PoolConfig {
                    thread_name: "planet-tile".to_owned(),
                    ..PoolConfig::with_workers(workers)
                });
                let mut drawn = WorkerScene {
                    editor,
                    live,
                    cells,
                    errors,
                };
                while let Ok(mut request) = asked.recv() {
                    // Only the latest ask counts.
                    while let Ok(newer) = asked.try_recv() {
                        let whole = request.whole || newer.whole;
                        request = newer;
                        request.whole = whole;
                    }
                    let result = make_cut(&device, &shaders, &parts, &pool, &mut drawn, request);
                    if done.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            built,
            worker: Some(worker),
            current,
            pending: false,
            asked: None,
            whole: false,
            latency,
            swaps: 0,
        })
    }

    /// Asks for the cut around `points` seen at `pixels_per_radian`, its pages loaded for
    /// `start`, unless an ask is being answered or the last went under [`ASK_EVERY`] ago: the
    /// worker makes it and changes the drawn scene when it should ([`needs_new_cut`]).
    pub fn ask(&mut self, points: Vec<DVec3>, pixels_per_radian: f64, start: StartView) {
        if self.pending
            || self
                .asked
                .is_some_and(|t| t.elapsed().as_secs_f64() < ASK_EVERY)
        {
            return;
        }
        self.pending = true;
        self.asked = Some(Instant::now());
        if let Some(requests) = &self.requests {
            let _ = requests.send(Request {
                points,
                pixels_per_radian,
                start,
                asked: Instant::now(),
                whole: std::mem::take(&mut self.whole),
            });
        }
    }

    /// Has the next ask build its scene whole: the drawn scene refused an edit, after which the
    /// worker's view of it is wrong.
    pub fn rebuild(&mut self) {
        self.whole = true;
    }

    /// The change made since the last call, if the last ask brought one.
    pub fn take(&mut self) -> Option<Result<Built>> {
        let result = self.built.try_recv().ok()?;
        self.pending = false;
        let result = result.transpose()?;
        if let Ok(built) = &result {
            self.current.clone_from(&built.cells);
            self.latency = built.seconds;
            self.swaps += 1;
        }
        Some(result)
    }
}

/// The worker's answer to `request`: nothing when the drawn cut should stay, else an edit of
/// the drawn scene when it is edited in place and the edit fits, its scene built whole
/// otherwise.
fn make_cut(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    parts: &SceneParts,
    pool: &TaskPool,
    drawn: &mut WorkerScene,
    request: Request,
) -> Result<Option<Built>> {
    let cells = cut_around(
        &parts.planet,
        &request.points,
        request.pixels_per_radian,
        &mut drawn.errors,
        pool,
    );
    let needed = request.whole || needs_new_cut(&drawn.cells, &cells);
    tracing::trace!(
        tiles = cells.len(),
        drawn = drawn.cells.len(),
        needed,
        "the swap rule's cut"
    );
    if !needed {
        return Ok(None);
    }
    let started = Instant::now();
    let have: HashMap<CellId, Tile> = drawn
        .live
        .iter()
        .map(|(&cell, live)| (cell, live.tile.clone()))
        .collect();
    let (tiles, cooked, loaded) = cook_cells(
        &parts.planet,
        &parts.body,
        !parts.in_place(),
        &cells,
        &have,
        pool,
    );
    drop(have);
    let cook_seconds = started.elapsed().as_secs_f64();
    let built = |change, cells: &[CellId]| Built {
        change,
        cells: cells.to_vec(),
        cooked,
        loaded,
        seconds: request.asked.elapsed().as_secs_f64(),
        cook_seconds,
        build_seconds: started.elapsed().as_secs_f64() - cook_seconds,
    };
    if let (Some(editor), false) = (drawn.editor.as_mut(), request.whole) {
        match edit_scene(
            device,
            parts,
            editor,
            &mut drawn.live,
            &cells,
            &tiles,
            &request.start,
        ) {
            Ok((edit, added, removed)) => {
                drawn.cells.clone_from(&cells);
                let change = Change::Edit {
                    edit: Box::new(edit),
                    added,
                    removed,
                };
                return Ok(Some(built(change, &cells)));
            }
            // Out of room: the scene is built whole, with room for this cut.
            Err(error) => {
                tracing::info!(%error, "an edit of the tiles failed: the scene is built whole")
            }
        }
    }
    let mut tiles = tiles;
    let capacity = parts.in_place().then(|| capacity_for(&tiles));
    let mut scene = build_scene(
        device,
        shaders,
        parts,
        &cells,
        &mut tiles,
        request.start,
        capacity,
    )?;
    drawn.editor = scene.editor(device, shaders)?;
    drawn.live = live_tiles(&cells, tiles);
    drawn.cells.clone_from(&cells);
    Ok(Some(built(Change::Scene(Box::new(scene)), &cells)))
}

impl Drop for TileStream {
    fn drop(&mut self) {
        // The worker ends with its channel.
        self.requests = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut cells: Vec<CellId>) -> Vec<CellId> {
        cells.sort_unstable();
        cells
    }

    #[test]
    fn a_new_cut_is_built_where_it_refines_or_sheds_many_tiles() {
        let faces = [CellId::cube(0, 0, 0, 0), CellId::cube(1, 0, 0, 0)];
        let children = |c: CellId| c.children().unwrap().to_vec();
        // Both faces cut in four.
        let wanted = sorted(faces.iter().flat_map(|&f| children(f)).collect());
        // The same with one quarter cut again: three tiles more.
        let first = children(faces[0])[0];
        let finer = sorted(
            wanted
                .iter()
                .copied()
                .filter(|&c| c != first)
                .chain(children(first))
                .collect(),
        );
        assert!(!needs_new_cut(&wanted, &wanted));
        // Finer than drawn somewhere: built.
        assert!(needs_new_cut(&wanted, &finer));
        assert!(needs_new_cut(&faces, &wanted));
        // Coarser by three tiles of eleven: waits.
        assert!(!needs_new_cut(&finer, &wanted));
        // Coarser by six of eight: built.
        assert!(needs_new_cut(&wanted, &faces));
    }
}
