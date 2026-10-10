//! The planet's tiles as the camera flies (#220): the cut made again around where the camera is
//! going, its scene built on a worker and swapped in at a frame's start.
//!
//! - **The cut** follows the points the demo gives ([`PlanetWorld::tile_cut_around`]): the
//!   camera where it will be when the scene is ready, and the place it is heading for (the tour's
//!   next stop, the descent's target), so a stop's tiles are made before the camera gets there.
//! - **The worker** keeps the meshes of the last cut it built; a new cut reuses them and cooks or
//!   loads the tiles it lacks from the cache, on a pool of half the cores, so the frames keep
//!   theirs. It then builds a whole scene: its tables, its rays' structures and the pages its
//!   start view wants, through the device's own one-shot submits.
//! - **The swap:** the new scene takes over at the start of a frame and the old one goes to the
//!   frames' deferred deletion, freed once no frame in flight reads it. The tiles keep their
//!   places in the world, so nothing else changes: no TAA reset, the same sky.
//!
//! A scene rebuilt whole costs about a second (`docs/demos/planet.md` has the numbers); the
//! renderer taking and freeing meshes at run time is the step after.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use anyhow::Result;
use forge_core::MaterialId;
use forge_geom::{CookOptions, MeshletMesh};
use forge_gpu::{Device, ShaderCompiler};
use forge_render::material::GpuMaterial;
use forge_render::material::TextureSet;
use forge_render::textures::TextureData;
use forge_render::{CellPos, MeshletScene, MeshletSceneBuilder, Residency, StartView};
use forge_task::{PoolConfig, TaskPool};
use forge_terrain::Planet;
use forge_terrain::planet::{
    NORMAL_MAP_SIZE, normal_map_levels, tile_mesh, tile_name, tile_normal_map_cached, tile_origin,
};
use forge_world::CellId;

use crate::Placement;

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

/// A tile's normal map on the GPU: a texture set of its own, which every scene drawing the tile
/// keeps alive ([`MeshletSceneBuilder::keep`]).
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
                let (done, stored) = forge_geom::cache::cook_cached(
                    cache,
                    &name,
                    key,
                    CookOptions { normal_weight: 0.0 },
                    in_memory,
                    || tile_mesh(planet, cell),
                );
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

/// The scene of `cells` (their tiles in `tiles`, in the same order), its pages for `start`
/// loaded when it streams them. The tiles' normal maps not on the GPU yet go there.
pub(crate) fn build_scene(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    parts: &SceneParts,
    cells: &[CellId],
    tiles: &mut [Tile],
    start: StartView,
) -> Result<MeshletScene> {
    let world = &parts.planet.world;
    for (cell, tile) in cells.iter().zip(tiles.iter_mut()) {
        if let NormalMap::Texels(texels) = &tile.normals {
            let mut set = TextureSet::new(device);
            let id = set.add(&TextureData {
                name: format!("{} normals", tile_name(&parts.body, *cell)).into(),
                size: NORMAL_MAP_SIZE,
                height: NORMAL_MAP_SIZE,
                srgb: false,
                levels: normal_map_levels(texels.as_ref().clone(), NORMAL_MAP_SIZE),
            })?;
            let sampled = set.sampled(id);
            tile.normals = NormalMap::Texture(Arc::new(TileTexture { _set: set, sampled }));
        }
    }
    let mut builder = MeshletSceneBuilder::new();
    builder.set_material_rows(parts.rows.clone());
    builder.set_ray_traced(parts.ray_traced);
    let ids: Vec<_> = tiles.iter().map(|t| builder.add_mesh(&t.mesh)).collect();
    // Every tile is ground to the rays, the finest under a kilometre across too.
    builder.set_ray_terrain(&ids);
    // The rays cut each level's tiles as one surface: a tile far from the camera is coarse
    // anyway, and its level's budget keeps the structures bounded: 120 000 triangles for every
    // 48 tiles or fewer, the most a level holds around one point.
    for level in 0..=world.tiles.finest {
        let members: Vec<_> = cells
            .iter()
            .zip(&ids)
            .filter(|(c, _)| c.level() == level)
            .map(|(_, &id)| id)
            .collect();
        if !members.is_empty() {
            let budget = 120_000 * members.len().div_ceil(48) as u32;
            builder.set_ray_group(&members, budget);
        }
    }
    let rotation = parts.placement.rotation.as_quat();
    for (i, ((&cell, &id), tile)) in cells.iter().zip(&ids).zip(tiles.iter()).enumerate() {
        let at = parts.placement.world_point(tile_origin(world, cell));
        builder.add_instance_at(id, CellPos::from_f64(at), rotation, 1.0, parts.ground);
        if let NormalMap::Texture(texture) = &tile.normals {
            builder.set_instance_texture(i, texture.sampled);
            builder.keep(Arc::clone(texture));
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

/// Whether the drawn cut `current` should give way to `wanted`: somewhere it is coarser, or it
/// holds half as many tiles again. Finer tiles where they are no longer needed cost little (their
/// DAGs coarsen with distance), so a cut that would only coarsen waits for the next that refines:
/// fewer scenes built, each a few frames' work for the GPU.
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

/// A scene the worker built.
pub(crate) struct Built {
    pub scene: MeshletScene,
    pub cells: Vec<CellId>,
    /// Tiles cooked and loaded from the cache for it; the rest it kept from the scene before.
    pub cooked: usize,
    pub loaded: usize,
    /// Seconds from the ask to the scene ready, and of them the tiles' and the scene's.
    pub seconds: f64,
    pub cook_seconds: f64,
    pub build_seconds: f64,
}

/// A cut to build, and the view its pages are loaded for.
struct Request {
    cells: Vec<CellId>,
    start: StartView,
    asked: Instant,
}

/// The worker that builds the scenes of new cuts (see the module notes).
pub(crate) struct TileStream {
    requests: Option<mpsc::Sender<Request>>,
    built: mpsc::Receiver<Result<Built>>,
    worker: Option<thread::JoinHandle<()>>,
    /// The cut of the scene drawn, and the one being built.
    pub current: Vec<CellId>,
    pending: Option<Vec<CellId>>,
    /// How long the last scene took, seconds: how far ahead to ask for the next.
    pub latency: f64,
    /// Scenes swapped in so far.
    pub swaps: u32,
}

impl TileStream {
    /// Starts the worker, which holds the tiles `tiles` of the drawn scene's cut `cells`.
    pub fn new(
        device: Arc<Device>,
        shaders: ShaderCompiler,
        parts: SceneParts,
        cells: Vec<CellId>,
        tiles: Vec<Tile>,
        latency: f64,
    ) -> Result<Self> {
        let (requests, asked) = mpsc::channel::<Request>();
        let (done, built) = mpsc::channel::<Result<Built>>();
        let current = cells.clone();
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
                let mut have: HashMap<CellId, Tile> = cells.into_iter().zip(tiles).collect();
                while let Ok(mut request) = asked.recv() {
                    // Only the latest ask counts.
                    while let Ok(newer) = asked.try_recv() {
                        request = newer;
                    }
                    let result = (|| {
                        let started = Instant::now();
                        let (mut tiles, cooked, loaded) = cook_cells(
                            &parts.planet,
                            &parts.body,
                            matches!(parts.residency, Residency::All),
                            &request.cells,
                            &have,
                            &pool,
                        );
                        let cook_seconds = started.elapsed().as_secs_f64();
                        let scene = build_scene(
                            &device,
                            &shaders,
                            &parts,
                            &request.cells,
                            &mut tiles,
                            request.start,
                        )?;
                        have = request.cells.iter().copied().zip(tiles).collect();
                        Ok(Built {
                            scene,
                            cells: request.cells,
                            cooked,
                            loaded,
                            seconds: request.asked.elapsed().as_secs_f64(),
                            cook_seconds,
                            build_seconds: started.elapsed().as_secs_f64() - cook_seconds,
                        })
                    })();
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
            pending: None,
            latency,
            swaps: 0,
        })
    }

    /// Asks for the scene of `cells`, its pages loaded for `start`, unless it is the drawn one's
    /// cut or a scene is being built (the next ask after it lands then counts).
    pub fn ask(&mut self, cells: Vec<CellId>, start: StartView) {
        if self.pending.is_some() || cells == self.current {
            return;
        }
        self.pending = Some(cells.clone());
        if let Some(requests) = &self.requests {
            let _ = requests.send(Request {
                cells,
                start,
                asked: Instant::now(),
            });
        }
    }

    /// The scene built since the last call, if one is ready.
    pub fn take(&mut self) -> Option<Result<Built>> {
        let result = self.built.try_recv().ok()?;
        self.pending = None;
        if let Ok(built) = &result {
            self.current.clone_from(&built.cells);
            self.latency = built.seconds;
            self.swaps += 1;
        }
        Some(result)
    }
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
