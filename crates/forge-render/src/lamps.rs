//! Lamps (D-065 step 1, #270): small sphere lights the resolve lights the scene with, one shadow
//! ray each towards a point of the sphere that changes every frame, so TAA averages their
//! penumbrae as it did the sun's single ray (#54).
//!
//! A scene gives its lamps each frame ([`crate::MeshletScene::set_lamps`]): where each is, its
//! radius, its luminous intensity in candelas and its colour temperature in kelvin. The
//! [`MAX_LAMPS`] nearest the camera whose light reaches it are kept ([`nearest`]); each pixel of
//! the resolve takes every one whose [`reach`] it is in (`lamps_at` in `meshlet.slang`). D-065's
//! clustered lists (Olsson, Billeter and Assarsson 2012) come when a view holds more lamps than
//! a village's few rooms; the probes lit by the lamps after.

use crate::columns::ColumnTables;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::BTreeMap;

/// The most lamps a frame lights with.
pub const MAX_LAMPS: usize = 64;
/// The most lamps a frame's buffers hold: all those [`every`] keeps, for D-065's comparison of
/// ReSTIR DI with the light lists (the village lights 325 at 22:00).
pub const LAMP_CAPACITY: usize = 512;
/// The illuminance a lamp's light is let go below, lux: its reach. A candle's at about 1 m.
pub const LEAST_LUX: f32 = 0.05;
/// The most regions a lamp lights ([`Lamp::regions`]).
pub const LAMP_REGIONS: usize = 8;
/// An empty slot of [`Lamp::regions`] (`REGION_NONE` in `probes.slang`).
pub const NO_REGION: u32 = u32::MAX;
/// The regions of a lamp that lights every surface its rays reach: a scene without regions.
pub const EVERY_REGION: [u32; LAMP_REGIONS] = [NO_REGION; LAMP_REGIONS];
/// The key the lamps of [`EVERY_REGION`] are listed under in [`lists`] (`REGION_ANY` in
/// `meshlet.slang`).
pub const REGION_ANY: u32 = u32::MAX - 1;
/// The probes' region outside every house (`REGION_OUTSIDE` in `probes.slang`, D-068): a street
/// lamp's ([`Lamp::outside`]).
pub const REGION_OUTSIDE: u32 = 0;
/// The most entries of [`lists`]: every lamp in every region it lights.
pub const MAX_LIST: usize = LAMP_CAPACITY * LAMP_REGIONS;

/// A lamp, in the frame [`crate::MeshletScene::set_lamps`] is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    /// Its bulb's middle, metres.
    pub position: Vec3,
    /// Its bulb's radius, metres: the size of its shadows' penumbrae.
    pub radius: f32,
    /// Its luminous intensity, candelas, the same every way (a bare bulb: lumens / 4π).
    pub candela: f32,
    /// Its colour temperature, kelvin (a warm bulb 2700, daylight 6500).
    pub kelvin: f32,
    /// The probes' regions (D-068, [`crate::RegionVolume`]) whose surfaces it lights: its room's
    /// and those it shines into through a doorway, [`NO_REGION`] in the slots left over. A
    /// surface elsewhere takes nothing of it, without a shadow ray: the house's rooms are its
    /// light lists. All slots empty ([`EVERY_REGION`]): every surface.
    pub regions: [u32; LAMP_REGIONS],
    /// Which way its light goes (#288): every way, or in a cone. [`Self::candela`] is the
    /// intensity at its brightest, along a cone's axis.
    pub emission: Emission,
}

impl Lamp {
    /// Whether it is a street lamp: [`REGION_OUTSIDE`] its first region. It lights the street
    /// and not the rooms, the probes leave it out, and ReSTIR DI lights it
    /// ([`LampChoice::Streets`]). Which way its light goes is its [`Self::emission`].
    pub fn outside(&self) -> bool {
        self.regions[0] == REGION_OUTSIDE
    }

    /// Its light in lumens, per channel of its colour of luminance 1 (`lamp_rgb`): its peak
    /// candelas over the solid angle its emission fills.
    pub fn lumens(&self) -> Vec3 {
        lamp_rgb(self.kelvin) * (self.candela.max(0.0) * self.emission.solid_angle())
    }

    /// The room it lights, its first region; none for a street lamp or one lighting every
    /// surface.
    pub fn room(&self) -> Option<u32> {
        let r = self.regions[0];
        (r != NO_REGION && r != REGION_OUTSIDE).then_some(r)
    }
}

/// Each room's lamps that are on (#363): their lumens per channel summed by [`Lamp::room`], the
/// far rooms' light round their walls, each weighed `bounce` plus `unlit` times the share of it
/// the frame leaves unlit (`kept`, [`choose_shares`]'s): past the frame's lamps the far room
/// takes their direct light on its walls too.
pub fn room_lumens(
    lamps: &[Lamp],
    kept: &[(usize, f32)],
    [bounce, unlit]: [f32; 2],
) -> std::collections::BTreeMap<u32, [f32; 3]> {
    let mut share = vec![0.0_f32; lamps.len()];
    for &(i, s) in kept {
        if let Some(v) = share.get_mut(i) {
            *v = s;
        }
    }
    let mut rooms = std::collections::BTreeMap::new();
    for (i, l) in lamps.iter().enumerate().filter(|(_, l)| l.candela > 0.0) {
        if let Some(room) = l.room() {
            let sum: &mut [f32; 3] = rooms.entry(room).or_default();
            let add = l.lumens() * (bounce + unlit * (1.0 - share[i]));
            *sum = [sum[0] + add.x, sum[1] + add.y, sum[2] + add.z];
        }
    }
    rooms
}

/// Which way a lamp's light goes (#288): the share of its peak intensity ([`Lamp::candela`])
/// in each direction, the same in every path that lights with it (the resolve, the light lists'
/// picks, the probes' and the mirror rays' hits, ReSTIR DI's target), through `lamp_beam` in
/// `meshlet.slang`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Emission {
    /// The same every way: a bare bulb, a glass globe, a pendant's open shade.
    #[default]
    Every,
    /// About `axis` (unit, the way the light goes): all of it within the half-angle whose cosine is
    /// `inner_cos`, none beyond the one whose cosine is `outer_cos`, a smooth step between. A
    /// downlight, a flood, a spot, a street lamp's cut-off ([`Emission::STREET`]).
    Cone {
        /// The cone's axis, unit.
        axis: Vec3,
        /// The cosine of the half-angle within which the light is whole.
        inner_cos: f32,
        /// The cosine of the half-angle beyond which there is none (below `inner_cos`).
        outer_cos: f32,
    },
}

impl Emission {
    /// A street lamp's head, full cut-off: all of its light down to 60° off the vertical, none from
    /// 85°. Lit every way, its bulb lit the facade over it round its housing's shadow, a dark
    /// funnel over each lamp (the owner, 2026-10-18).
    pub const STREET: Emission = Emission::Cone {
        axis: Vec3::NEG_Y,
        inner_cos: 0.5,
        outer_cos: 0.087,
    };

    /// A light flush with a ceiling or a wall facing `axis` (a dome, a diffuser): none behind its
    /// mount, its intensity falling from the axis to nothing at 90°, near a diffuser's cosine
    /// (the smooth step from 0 to 1 is 0.5 at 60°, where the cosine is too). It sends a quarter
    /// of the light a lamp of the same peak sends every way: multiply the candela by 4 to keep
    /// the lumens.
    pub fn flush(axis: Vec3) -> Emission {
        Emission::Cone {
            axis: axis.normalize(),
            inner_cos: 1.0,
            outer_cos: 0.0,
        }
    }

    /// The share of the peak intensity towards the unit direction `out` from the lamp (the CPU's
    /// `lamp_beam`).
    pub fn share(&self, out: Vec3) -> f32 {
        match *self {
            Emission::Every => 1.0,
            Emission::Cone {
                axis,
                inner_cos,
                outer_cos,
            } => {
                let t = ((out.dot(axis) - outer_cos) / (inner_cos - outer_cos)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
        }
    }

    /// The solid angle its light fills, steradians, as the peak's: its lumens over its candelas.
    /// Every way 4π; a cone 2π of the cosine it spans whole and half the step's.
    pub fn solid_angle(&self) -> f32 {
        match *self {
            Emission::Every => 4.0 * std::f32::consts::PI,
            Emission::Cone {
                inner_cos,
                outer_cos,
                ..
            } => 2.0 * std::f32::consts::PI * ((1.0 - inner_cos) + 0.5 * (inner_cos - outer_cos)),
        }
    }

    /// Its two vectors of [`GpuLamp`]: the axis and the full light's cosine, then the cut-off's.
    /// Every way: a zero axis.
    fn gpu(&self) -> ([f32; 4], [f32; 4]) {
        match *self {
            Emission::Every => ([0.0; 4], [0.0; 4]),
            Emission::Cone {
                axis,
                inner_cos,
                outer_cos,
            } => (
                [axis.x, axis.y, axis.z, inner_cos],
                [outer_cos, 0.0, 0.0, 0.0],
            ),
        }
    }
}

/// Mirrors `Lamp` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuLamp {
    /// xyz: the bulb's middle in the scene's frame; w: its radius.
    pub position_radius: [f32; 4],
    /// rgb: its intensity per channel, candelas (its colour of luminance 1 times its candelas);
    /// w: its reach, metres.
    pub colour_reach: [f32; 4],
    /// [`Lamp::regions`].
    pub regions: [u32; LAMP_REGIONS],
    /// [`Lamp::emission`]: xyz its cone's axis (zero: every way), w the cosine within which its
    /// light is whole.
    pub beam: [f32; 4],
    /// x: the cosine beyond which a cone's light is none; yzw unused.
    pub beam_edge: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuLamp>() == 96);

/// How far a lamp's light counts, metres: where it falls to [`LEAST_LUX`].
pub fn reach(candela: f32) -> f32 {
    (candela.max(0.0) / LEAST_LUX).sqrt()
}

/// A blackbody's colour at `kelvin`, linear Rec. 709 of luminance 1: Tanner Helland's fit of
/// the CIE 1964 10° colour-matching functions' blackbody locus (sRGB-encoded, so decoded here).
pub fn kelvin_rgb(kelvin: f32) -> Vec3 {
    let t = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_73 * (t - 60.0).powf(-0.133_204_76)
    };
    let g = if t <= 66.0 {
        99.470_8 * t.ln() - 161.119_57
    } else {
        288.122_16 * (t - 60.0).powf(-0.075_514_85)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    let decode = |c: f32| {
        let c = (c / 255.0).clamp(0.0, 1.0);
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let rgb = Vec3::new(decode(r), decode(g), decode(b));
    rgb / rgb.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-6)
}

/// The colour temperature the lamps' colours are balanced for, kelvin: as a camera set for
/// tungsten light sees them, and the eye adapted to a lit room. At the sun's white point a 2 700 K
/// bulb turned a white room deep orange (2026-10-18); balanced, it stays warm. The sun and the
/// sky keep their colours.
pub const WHITE_BALANCE: f32 = 3200.0;

/// A lamp's colour at `kelvin` as [`WHITE_BALANCE`] sees it ([`kelvin_rgb`] over the balance's),
/// linear Rec. 709 of luminance 1.
pub fn lamp_rgb(kelvin: f32) -> Vec3 {
    let c = kelvin_rgb(kelvin) / kelvin_rgb(WHITE_BALANCE);
    c / c.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-6)
}

/// Which of the scene's lamps a frame lights with ([`choose`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LampChoice {
    /// Those whose light reaches within 50 m of the camera, the nearest first, [`MAX_LAMPS`] at
    /// most: the light lists' share of them. The street lamps ([`Lamp::outside`]) keep up to
    /// [`STREET_SHARE`] of them, the others the rest, and the slots one kind leaves go to the
    /// other (#295).
    Nearest,
    /// As `Nearest` for every lamp but the street lamps, and every street lamp, however far:
    /// ReSTIR DI lights those (D-065, the owner's choice of 2026-10-18), so a far street keeps
    /// its light.
    Streets,
    /// Every lamp that is on, however far, [`LAMP_CAPACITY`] at most: D-065's comparison of
    /// ReSTIR DI with the light lists, which takes them all.
    Every,
}

/// The street lamps' share of [`MAX_LAMPS`] in [`LampChoice::Nearest`] (#295): a few hundred
/// house lamps nearer the camera crowded them out, and a lit street went dark.
pub const STREET_SHARE: usize = 24;

/// A street lamp's lens luminance, cd/m², in its red channel (its colour 1, 0.78, 0.55;
/// `forge_geom::street::lamp`'s section 1): shielded LEDs at 2 700 K behind a frosted diffuser.
/// At 4 and 6 times it (7.5 and 12 lux under a lamp, LED street lighting's range) the owner
/// preferred this (2026-10-18).
pub const STREET_LENS: f32 = 1000.0;

/// The street lamps' lights (D-065, #296; city-plan's until then): one under each arm's lens
/// (`forge_geom::street`), its intensity the lens's luminance times its area, so the light on the
/// road matches the glow seen (about 1.9 lux under an 8 m lamp, eight times a full moon's). They
/// light the outside's region only (D-068), the street and not the rooms behind the windows,
/// where they took the house lamps' places. `lamps`: each lamp's foot, scale, turn about +Y and
/// whether it has its second arm.
pub fn street_lights(lamps: &[(Vec3, f32, f32, bool)]) -> Vec<Lamp> {
    use forge_geom::street::{LENS_SIZE, LENS_Z};
    let mut regions = [NO_REGION; LAMP_REGIONS];
    regions[0] = REGION_OUTSIDE;
    // The lens's luminance by its colour (1, 0.78, 0.55), over its area.
    let lens = LENS_SIZE[0] * LENS_SIZE[1] * STREET_LENS * (0.2126 + 0.7152 * 0.78 + 0.0722 * 0.55);
    // A bulb of 12 cm just under the lens (7.69 m on the 8 m model).
    let (radius, under) = (0.12, 7.56);
    let mut out = Vec::new();
    for &(at, scale, turn, double) in lamps {
        let arms = if double { &LENS_Z[..] } else { &LENS_Z[..1] };
        for &z in arms {
            out.push(Lamp {
                position: at
                    + glam::Quat::from_rotation_y(turn) * (Vec3::new(0.0, under, z) * scale),
                radius: radius * scale,
                candela: lens * scale * scale,
                kelvin: 2700.0,
                regions,
                emission: Emission::STREET,
            });
        }
    }
    out
}

/// How far before its cut a kept lamp fades out, as a share of the cut's distance, at least
/// [`LEAST_FADE`] metres (#295). The cut is the distance of the nearest lamp of its kind not kept,
/// or where its light stops counting (its reach and the margin). Kept or not changed at once
/// before, and lamps popped on and off as the camera moved; the cut moves with the camera
/// without jumps, and two lamps changing places at it are both faded out.
const FADE: f32 = 0.2;
/// The least fade, metres.
const LEAST_FADE: f32 = 2.0;

/// The lamps a frame lights with, of `lamps`, seen from `camera` (scene frame), by `choice`: the
/// nearest first within each kind, each faded towards its cut ([`FADE`]).
pub fn choose(lamps: &[Lamp], camera: Vec3, choice: LampChoice) -> Vec<GpuLamp> {
    choose_shares(lamps, camera, choice)
        .into_iter()
        .map(|(i, share)| gpu_lamp(&lamps[i], share))
        .collect()
}

/// The lamps [`choose`] keeps, as their indices in `lamps` and the share of their light each is
/// lit with (1, less its fade towards its cut), in the frame's order.
pub fn choose_shares(lamps: &[Lamp], camera: Vec3, choice: LampChoice) -> Vec<(usize, f32)> {
    match choice {
        LampChoice::Nearest => {
            let streets = candidates(lamps, camera, 50.0, Lamp::outside);
            let houses = candidates(lamps, camera, 50.0, |l| !l.outside());
            let mut s = streets.len().min(STREET_SHARE);
            let mut h = houses.len().min(MAX_LAMPS - STREET_SHARE);
            // The slots one kind leaves, to the other's nearest.
            while s + h < MAX_LAMPS {
                match (streets.get(s), houses.get(h)) {
                    (Some(a), Some(b)) if a.0 <= b.0 => s += 1,
                    (_, Some(_)) => h += 1,
                    (Some(_), None) => s += 1,
                    (None, None) => break,
                }
            }
            let mut out: Vec<(f32, usize, f32)> = faded(lamps, &streets, s, 50.0)
                .chain(faded(lamps, &houses, h, 50.0))
                .collect();
            out.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.into_iter().map(|(_, i, share)| (i, share)).collect()
        }
        LampChoice::Every => {
            let all = candidates(lamps, camera, f32::INFINITY, |_| true);
            let n = all.len().min(LAMP_CAPACITY);
            faded(lamps, &all, n, f32::INFINITY)
                .map(|(_, i, share)| (i, share))
                .collect()
        }
        LampChoice::Streets => {
            let houses = candidates(lamps, camera, 50.0, |l| !l.outside());
            let h = houses.len().min(MAX_LAMPS);
            let streets = candidates(lamps, camera, f32::INFINITY, Lamp::outside);
            let s = streets.len().min(LAMP_CAPACITY - h);
            faded(lamps, &houses, h, 50.0)
                .chain(faded(lamps, &streets, s, f32::INFINITY))
                .map(|(_, i, share)| (i, share))
                .collect()
        }
    }
}

/// The share of each of `count` lamps' light a frame leaves unlit, by their indices: 1 less the
/// share `kept` lights it with ([`choose_shares`]), 1 for a lamp not kept. A clear pane's room
/// glows with its lamp's ([`crate::Glass::set_unlit`], #284).
pub fn unlit(count: usize, kept: &[(usize, f32)]) -> Vec<f32> {
    let mut out = vec![1.0_f32; count];
    for &(i, share) in kept {
        if let Some(u) = out.get_mut(i) {
            *u = (1.0 - share).max(0.0);
        }
    }
    out
}

/// The lamps a frame lights with: those whose light reaches within 50 m of the camera at
/// `camera` (scene frame), [`MAX_LAMPS`] at most ([`LampChoice::Nearest`]).
pub fn nearest(lamps: &[Lamp], camera: Vec3) -> Vec<GpuLamp> {
    choose(lamps, camera, LampChoice::Nearest)
}

/// Every lamp that is on, however far, the nearest to `camera` first, [`LAMP_CAPACITY`] at
/// most ([`LampChoice::Every`]).
pub fn every(lamps: &[Lamp], camera: Vec3) -> Vec<GpuLamp> {
    choose(lamps, camera, LampChoice::Every)
}

/// The lamps of `lamps` that `pick` takes, are on and whose light reaches within `margin` metres
/// of `camera`, as their distances and indices, the nearest first.
fn candidates(
    lamps: &[Lamp],
    camera: Vec3,
    margin: f32,
    pick: impl Fn(&Lamp) -> bool,
) -> Vec<(f32, usize)> {
    let mut near: Vec<(f32, usize)> = lamps
        .iter()
        .enumerate()
        .filter(|(_, l)| l.candela > 0.0 && pick(l))
        .map(|(i, l)| (l.position.distance(camera), i))
        .filter(|&(d, i)| d < reach(lamps[i].candela) + margin)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near
}

/// The first `kept` of `near` (of `lamps`), with their distances, indices and shares, each faded
/// towards its cut: the next candidate's distance, and its own reach and `margin`.
fn faded<'a>(
    lamps: &'a [Lamp],
    near: &'a [(f32, usize)],
    kept: usize,
    margin: f32,
) -> impl Iterator<Item = (f32, usize, f32)> + 'a {
    let next = near.get(kept).map(|c| c.0);
    near[..kept].iter().map(move |&(d, i)| {
        let out = |cut: f32| ((cut - d) / (FADE * cut).max(LEAST_FADE)).clamp(0.0, 1.0);
        let edge = reach(lamps[i].candela) + margin;
        let share = next.map_or(1.0, out) * if edge.is_finite() { out(edge) } else { 1.0 };
        (d, i, share)
    })
}

/// `l` as the GPU takes it, its light times `share`.
pub(crate) fn gpu_lamp(l: &Lamp, share: f32) -> GpuLamp {
    let c = lamp_rgb(l.kelvin) * (l.candela * share);
    let (beam, beam_edge) = l.emission.gpu();
    GpuLamp {
        position_radius: [
            l.position.x,
            l.position.y,
            l.position.z,
            l.radius.max(0.005),
        ],
        colour_reach: [c.x, c.y, c.z, reach(l.candela)],
        regions: l.regions,
        beam,
        beam_edge,
    }
}

/// A lamp that gives no light: a [`LampTable`]'s free place. Every choice and the far lamps'
/// grid pass over it.
pub const DARK: Lamp = Lamp {
    position: Vec3::ZERO,
    radius: 0.0,
    candela: 0.0,
    kelvin: WHITE_BALANCE,
    regions: EVERY_REGION,
    emission: Emission::Every,
};

/// A set of lamps in a [`LampTable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LampSetId(u32);

/// The lamps of a scene that streams (#338), by ids that stay theirs while their set is in: a
/// cell's lamps added as a set when it comes ([`Self::add`]) and removed when it goes. Each
/// lamp's id is its index in [`Self::lamps`], the list the frames choose from
/// ([`crate::MeshletScene::set_lamps`]) and the clear panes name ([`crate::GlassPanes::lamp`]):
/// a removed set's places are left [`DARK`] until another set takes them.
///
/// The probes follow a lamp that comes or goes by themselves (#336's adaptive hysteresis): woken
/// young again, they ran their placement anew and settled elsewhere in the crowns, 0.4 % of a
/// street at night still off 850 frames after its lamps came back (38 pixels without).
pub struct LampTable {
    lamps: Vec<Lamp>,
    free: crate::meshlet::dynamic::Ranges,
    sets: Vec<Option<(u32, u32)>>,
    /// Whether a street lamp came, went or changed since [`Self::take_streets_changed`].
    streets_changed: bool,
}

impl LampTable {
    /// Room for `capacity` lamps, all dark.
    pub fn new(capacity: u32) -> Self {
        Self {
            lamps: vec![DARK; capacity as usize],
            free: crate::meshlet::dynamic::Ranges::new(capacity, 0),
            sets: Vec::new(),
            streets_changed: false,
        }
    }

    /// Adds `lamps` as a set: lamp `i` of them is id [`Self::first`]`(set) + i`. `None` without
    /// room for them in one run.
    pub fn add(&mut self, lamps: &[Lamp]) -> Option<LampSetId> {
        let n = lamps.len() as u32;
        let first = self.free.take(n)?;
        self.lamps[first as usize..(first + n) as usize].copy_from_slice(lamps);
        self.streets_changed |= lamps.iter().any(Lamp::outside);
        let id = match self.sets.iter().position(Option::is_none) {
            Some(free) => free,
            None => {
                self.sets.push(None);
                self.sets.len() - 1
            }
        };
        self.sets[id] = Some((first, n));
        Some(LampSetId(id as u32))
    }

    /// Removes set `set`: its places dark from now on. Nothing for a set not in.
    pub fn remove(&mut self, set: LampSetId) {
        let Some((first, n)) = self.sets.get_mut(set.0 as usize).and_then(Option::take) else {
            return;
        };
        let places = &mut self.lamps[first as usize..(first + n) as usize];
        self.streets_changed |= places.iter().any(Lamp::outside);
        places.fill(DARK);
        self.free.give(first, n);
    }

    /// The id of set `set`'s first lamp; `None` for a set not in.
    pub fn first(&self, set: LampSetId) -> Option<u32> {
        self.sets.get(set.0 as usize)?.map(|(first, _)| first)
    }

    /// Lamp `id`, to switch, dim or move it (a lamp turned off: its candela 0). Nothing for an id
    /// past the table.
    pub fn set(&mut self, id: u32, lamp: Lamp) {
        if let Some(l) = self.lamps.get_mut(id as usize) {
            self.streets_changed |= l.outside() || lamp.outside();
            *l = lamp;
        }
    }

    /// Every place, the dark ones too: the frames' list, by id.
    pub fn lamps(&self) -> &[Lamp] {
        &self.lamps
    }

    /// Whether a street lamp came, went or changed since the last call: the far lamps' grid is
    /// to be made again ([`crate::MeshletScene::set_far_lamps`]).
    pub fn take_streets_changed(&mut self) -> bool {
        std::mem::take(&mut self.streets_changed)
    }
}

/// A far lamps' grid column's side, metres (#284).
pub const FAR_CELL: f32 = 8.0;

/// Mirrors `FarLamps` in `meshlet.slang`: where [`FarLamps`]' tables stand and the hash's size.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuFarLamps {
    pub slots: u64,
    pub indices: u64,
    pub lamps: u64,
    /// The slots' count less one (a power of two less one).
    pub slot_mask: u32,
    pub cell: f32,
    pub count: u32,
    pub pad: u32,
}

/// The street lamps lit beyond the frame's set (#284): each one's light, less the share of it the
/// set lights ([`choose_shares`]), unshadowed and without the penumbra filter, found by the
/// resolve in a grid of [`FAR_CELL`] columns built once (`far_lamps_at` in `meshlet.slang`).
/// Only [`MAX_LAMPS`] are lit with shadow rays a frame, the nearest, and the far streets' pools
/// went dark while their lamp heads kept glowing; a lamp leaving the set fades into this as it
/// fades out of it, and back.
pub struct FarLamps {
    /// The lamps as the GPU takes them, their whole light.
    pub(crate) lamps: Vec<GpuLamp>,
    /// Their peak intensities as built, candelas: a frame's lamp of another weighs its share.
    candela: Vec<f32>,
    /// The columns each lamp's reach overlaps.
    pub(crate) columns: ColumnTables,
}

impl FarLamps {
    /// The grid of `lamps`' street lamps that are on ([`Lamp::outside`]), in the frame
    /// [`crate::MeshletScene::set_lamps`] is given; `None` without any.
    pub fn new(lamps: &[Lamp]) -> Option<Self> {
        let street: Vec<&Lamp> = lamps
            .iter()
            .filter(|l| l.outside() && l.candela > 0.0)
            .collect();
        if street.is_empty() {
            return None;
        }
        let mut columns: BTreeMap<(i32, i32), Vec<u32>> = BTreeMap::new();
        for (k, l) in street.iter().enumerate() {
            let r = reach(l.candela);
            let at = glam::Vec2::new(l.position.x, l.position.z);
            let (lo, hi) = (
                ((at - r) / FAR_CELL).floor().as_ivec2(),
                ((at + r) / FAR_CELL).floor().as_ivec2(),
            );
            for x in lo.x..=hi.x {
                for z in lo.y..=hi.y {
                    // The column's nearest point to the lamp, within its reach.
                    let corner = glam::Vec2::new(x as f32, z as f32) * FAR_CELL;
                    if at.clamp(corner, corner + FAR_CELL).distance(at) < r {
                        columns.entry((x, z)).or_default().push(k as u32);
                    }
                }
            }
        }
        Some(Self {
            lamps: street.iter().map(|l| gpu_lamp(l, 1.0)).collect(),
            candela: street.iter().map(|l| l.candela).collect(),
            columns: ColumnTables::new(&columns),
        })
    }

    /// How many lamps it holds.
    pub fn len(&self) -> usize {
        self.lamps.len()
    }

    /// Whether it holds none (never: [`Self::new`] gives `None` then).
    pub fn is_empty(&self) -> bool {
        self.lamps.is_empty()
    }

    /// Its lamp standing at `position` (within a centimetre), if any.
    fn find(&self, position: Vec3) -> Option<usize> {
        let (x, z) = (
            (position.x / FAR_CELL).floor() as i32,
            (position.z / FAR_CELL).floor() as i32,
        );
        self.columns
            .column(x, z)
            .iter()
            .map(|&k| k as usize)
            .find(|&k| {
                let p = self.lamps[k].position_radius;
                Vec3::new(p[0], p[1], p[2]).distance_squared(position) < 1e-4
            })
    }

    /// Each of its lamps' share of its light this frame: of `lamps` (the frame's, as given to
    /// [`crate::MeshletScene::set_lamps`]), the street lamps that are on, less the share `kept`
    /// lights them with ([`choose_shares`]); 0 for a lamp not among them (off).
    pub fn weights(&self, lamps: &[Lamp], kept: &[(usize, f32)]) -> Vec<f32> {
        let mut lit = vec![0.0_f32; lamps.len()];
        for &(i, share) in kept {
            lit[i] = share;
        }
        let mut out = vec![0.0_f32; self.lamps.len()];
        for (i, l) in lamps.iter().enumerate() {
            if !l.outside() || l.candela <= 0.0 {
                continue;
            }
            if let Some(k) = self.find(l.position) {
                out[k] = (1.0 - lit[i]).max(0.0) * l.candela / self.candela[k];
            }
        }
        out
    }
}

/// The frame's light lists (D-065): for each region its `kept` lamps light, the lamps' indices, as
/// pairs (region, lamp) sorted by region, then lamp; the lamps of [`EVERY_REGION`] under
/// [`REGION_ANY`]. A pixel finds its region's run by a binary search (`lamp_run` in
/// `meshlet.slang`) and loops over its few lamps, not over the frame's 64 (0.5 ms in a hall).
pub fn lists(kept: &[GpuLamp]) -> Vec<[u32; 2]> {
    let mut out = Vec::new();
    for (i, l) in kept.iter().enumerate() {
        let i = i as u32;
        if l.regions.iter().all(|&r| r == NO_REGION) {
            out.push([REGION_ANY, i]);
        } else {
            out.extend(
                l.regions
                    .iter()
                    .filter(|&&r| r != NO_REGION)
                    .map(|&r| [r, i]),
            );
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_lists_hold_each_lamp_under_each_of_its_regions() {
        let lamp = |regions: [u32; LAMP_REGIONS]| GpuLamp {
            regions,
            ..GpuLamp::default()
        };
        let mut two = EVERY_REGION;
        two[0] = 7;
        two[1] = 3;
        two[2] = 7;
        let mut one = EVERY_REGION;
        one[0] = 3;
        let pairs = lists(&[lamp(two), lamp(EVERY_REGION), lamp(one)]);
        assert_eq!(pairs, vec![[3, 0], [3, 2], [7, 0], [REGION_ANY, 1]]);
    }

    #[test]
    fn a_lamps_lumens_follow_the_solid_angle_its_light_fills() {
        let pi = std::f32::consts::PI;
        assert!((Emission::Every.solid_angle() - 4.0 * pi).abs() < 1e-5);
        // A flush light sends a quarter of a bare bulb's of the same peak (`Emission::flush`).
        assert!((Emission::flush(Vec3::NEG_Y).solid_angle() - pi).abs() < 1e-5);
        let bulb = Lamp {
            candela: 100.0,
            kelvin: 6500.0,
            ..DARK
        };
        let luminance = bulb.lumens().dot(Vec3::new(0.2126, 0.7152, 0.0722));
        assert!((luminance - 400.0 * pi).abs() < 0.5, "{luminance}");
    }

    #[test]
    fn a_rooms_lumens_weigh_the_share_the_frame_leaves_unlit() {
        let in_room = |room: u32, candela: f32| {
            let mut regions = EVERY_REGION;
            regions[0] = room;
            Lamp {
                candela,
                kelvin: 6500.0,
                regions,
                ..DARK
            }
        };
        let street = Lamp {
            candela: 50.0,
            regions: {
                let mut r = EVERY_REGION;
                r[0] = REGION_OUTSIDE;
                r
            },
            ..DARK
        };
        let lamps = [
            in_room(5, 100.0),
            in_room(5, 0.0),
            in_room(9, 100.0),
            street,
        ];
        let luma = |c: [f32; 3]| Vec3::from(c).dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let one = 400.0 * std::f32::consts::PI;
        // Room 5's lamp lit whole by the frame, room 9's left unlit: bounce 2, unlit 3.
        let rooms = room_lumens(&lamps, &[(0, 1.0)], [2.0, 3.0]);
        assert_eq!(
            rooms.len(),
            2,
            "the street lamp and the dark lamp take no room"
        );
        assert!((luma(rooms[&5]) - 2.0 * one).abs() < 1.0);
        assert!((luma(rooms[&9]) - 5.0 * one).abs() < 1.0);
    }

    #[test]
    fn a_warm_bulb_is_orange_and_daylight_white_both_of_luminance_one() {
        let warm = kelvin_rgb(2700.0);
        let day = kelvin_rgb(6500.0);
        let luminance = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        assert!((luminance(warm) - 1.0).abs() < 1e-4);
        assert!((luminance(day) - 1.0).abs() < 1e-4);
        assert!(warm.x > warm.y && warm.y > warm.z, "{warm}");
        // Daylight within a few percent of white.
        assert!((day.x - day.z).abs() < 0.1, "{day}");
        // Balanced for tungsten: its own temperature white, a warm bulb warm but its blue kept
        // (half its red, where the sun's white point leaves a tenth).
        let balanced = lamp_rgb(WHITE_BALANCE);
        assert!(
            (balanced - Vec3::ONE).abs().max_element() < 1e-4,
            "{balanced}"
        );
        let warm = lamp_rgb(2700.0);
        assert!((luminance(warm) - 1.0).abs() < 1e-4);
        assert!(warm.z / warm.x > 0.4 && kelvin_rgb(2700.0).z / kelvin_rgb(2700.0).x < 0.12);
    }

    #[test]
    fn the_frame_keeps_the_nearest_lamps_that_reach_the_camera() {
        let bulb = |x: f32, candela: f32| Lamp {
            position: Vec3::new(x, 2.0, 0.0),
            radius: 0.05,
            candela,
            kelvin: 2700.0,
            regions: EVERY_REGION,
            emission: Emission::Every,
        };
        // 100 cd reaches about 45 m.
        assert!((reach(100.0) - 44.72).abs() < 0.01);
        // A row a metre apart: those within the reach and 50 m of the camera (95 of them) count,
        // the 64 nearest are kept.
        let lamps: Vec<Lamp> = (0..100)
            .map(|i| bulb(i as f32, 100.0))
            .chain([bulb(5000.0, 100.0), bulb(1.0, 0.0)])
            .collect();
        let kept = nearest(&lamps, Vec3::ZERO);
        assert_eq!(kept.len(), MAX_LAMPS);
        assert_eq!(kept[0].position_radius[0], 0.0);
        assert!(
            kept.windows(2)
                .all(|w| w[0].position_radius[0] <= w[1].position_radius[0])
        );
        // Every lamp that is on, the far one too, the nearest first.
        let all = every(&lamps, Vec3::ZERO);
        assert_eq!(all.len(), 101);
        assert_eq!(all[100].position_radius[0], 5000.0);
    }

    #[test]
    fn street_lamps_keep_their_share_against_nearer_house_lamps() {
        let lamp = |at: Vec3, candela: f32, outside: bool| {
            let mut regions = EVERY_REGION;
            regions[0] = if outside { REGION_OUTSIDE } else { 7 };
            Lamp {
                position: at,
                radius: 0.05,
                candela,
                kelvin: 2700.0,
                regions,
                emission: Emission::Every,
            }
        };
        // A hundred house lamps within 10 m, thirty street lamps 60 to 89 m away (109 cd reach
        // 47 m, so they count to 97 m).
        let lamps: Vec<Lamp> = (0..100)
            .map(|i| {
                lamp(
                    Vec3::new((i % 10) as f32, 2.0, (i / 10) as f32),
                    20.0,
                    false,
                )
            })
            .chain((0..30).map(|i| lamp(Vec3::new(0.0, 7.5, 60.0 + i as f32), 109.0, true)))
            .collect();
        let kept = nearest(&lamps, Vec3::ZERO);
        let streets = kept
            .iter()
            .filter(|l| l.regions[0] == REGION_OUTSIDE)
            .count();
        assert_eq!(kept.len(), MAX_LAMPS);
        assert_eq!(streets, STREET_SHARE);
        // Without the houses, the streets take every slot they need.
        let alone: Vec<Lamp> = lamps[100..].to_vec();
        assert_eq!(nearest(&alone, Vec3::ZERO).len(), 30);
    }

    #[test]
    fn the_far_lamps_light_what_the_set_leaves_of_each_street_lamp() {
        let lamp = |at: Vec3, outside: bool| {
            let mut regions = EVERY_REGION;
            regions[0] = if outside { REGION_OUTSIDE } else { 7 };
            Lamp {
                position: at,
                radius: 0.12,
                candela: 109.0,
                kelvin: 2700.0,
                regions,
                emission: Emission::STREET,
            }
        };
        // A street of a hundred lamps 25 m apart and a house lamp; the camera at its start.
        let lamps: Vec<Lamp> = (0..100)
            .map(|i| lamp(Vec3::new(3.0, 7.5, 25.0 * i as f32), true))
            .chain([lamp(Vec3::new(0.0, 2.0, 0.0), false)])
            .collect();
        let far = FarLamps::new(&lamps).expect("street lamps");
        assert_eq!(far.len(), 100);
        // A lamp is listed under the column it stands in, not in one beyond its reach (47 m).
        assert!(far.columns.column(0, 50).contains(&16));
        assert!(!far.columns.column(0, 50).contains(&30));
        let kept = choose_shares(&lamps, Vec3::ZERO, LampChoice::Nearest);
        let weights = far.weights(&lamps, &kept);
        // Each street lamp's light is lit once: by the set, by the far lamps, or shared.
        let mut lit = [0.0_f32; 100];
        for &(i, share) in &kept {
            if i < 100 {
                lit[i] = share;
            }
        }
        for k in 0..100 {
            assert!(
                (lit[k] + weights[k] - 1.0).abs() < 1e-5,
                "{k}: {} {}",
                lit[k],
                weights[k]
            );
        }
        assert_eq!(weights[0], 0.0);
        assert_eq!(weights[99], 1.0);
        // A street lamp switched off is lit by neither.
        let off: Vec<Lamp> = lamps[1..].to_vec();
        assert_eq!(far.weights(&off, &[])[0], 0.0);
        // What the set leaves unlit of each lamp, for the clear glass's rooms.
        assert_eq!(unlit(4, &[(1, 0.25), (3, 1.0)]), vec![1.0, 0.75, 1.0, 0.0]);
    }

    #[test]
    fn no_lamp_switches_on_or_off_at_once_as_the_camera_moves() {
        // Two hundred lamps on a 2 m grid; the camera walks 40 m across it in 1 cm steps. Each
        // lamp's light (its red intensity) may change little from step to step, though dozens
        // enter and leave the 64 kept.
        let lamps: Vec<Lamp> = (0..200)
            .map(|i| Lamp {
                position: Vec3::new((i % 20) as f32 * 2.0, 2.5, (i / 20) as f32 * 2.0),
                radius: 0.05,
                candela: 50.0,
                kelvin: 2700.0,
                regions: EVERY_REGION,
                emission: Emission::Every,
            })
            .collect();
        let light = |camera: Vec3| {
            let mut out = vec![0.0_f32; lamps.len()];
            for l in nearest(&lamps, camera) {
                let i = lamps
                    .iter()
                    .position(|x| {
                        x.position.x == l.position_radius[0] && x.position.z == l.position_radius[2]
                    })
                    .unwrap();
                out[i] = l.colour_reach[0];
            }
            out
        };
        let full = lamp_rgb(2700.0).x * 50.0;
        let mut before = light(Vec3::new(-10.0, 1.7, 9.0));
        let (mut changes, mut worst) = (0, 0.0_f32);
        for step in 1..=4000 {
            let now = light(Vec3::new(-10.0 + step as f32 * 0.01, 1.7, 9.0));
            for (a, b) in before.iter().zip(&now) {
                let jump = (a - b).abs() / full;
                worst = worst.max(jump);
                changes += usize::from((a > &0.0) != (b > &0.0));
            }
            before = now;
        }
        assert!(changes > 20, "{changes} lamps entered or left");
        assert!(
            worst < 0.01,
            "a lamp's light jumped by {worst} of its full light in 1 cm"
        );
    }

    #[test]
    fn a_cone_lights_within_its_angles_and_the_street_cut_off_is_the_old_beam() {
        // Every way: all of it everywhere.
        assert_eq!(Emission::Every.share(Vec3::Y), 1.0);
        // The street lamp's cone is the beam it had by its region: smoothstep(0.087, 0.5, y) of the
        // direction from the surface to the lamp, the bit-exact same expression.
        for i in 0..=100 {
            let a = i as f32 * std::f32::consts::PI / 100.0;
            let to_lamp = Vec3::new(a.sin(), a.cos(), 0.0);
            let t = ((to_lamp.y - 0.087) / (0.5 - 0.087)).clamp(0.0, 1.0);
            let old = t * t * (3.0 - 2.0 * t);
            assert_eq!(Emission::STREET.share(-to_lamp), old, "{a}");
        }
        // A flush dome facing down: none behind its mount, half at 60°, all along its axis; a
        // quarter of an omni lamp's lumens at the same peak (∫ share dΩ = π against 4π).
        let dome = Emission::flush(Vec3::NEG_Y);
        assert_eq!(dome.share(Vec3::Y), 0.0);
        assert_eq!(dome.share(Vec3::X), 0.0);
        assert!((dome.share(Vec3::new(0.0, -0.5, 0.75_f32.sqrt())) - 0.5).abs() < 1e-6);
        assert_eq!(dome.share(Vec3::NEG_Y), 1.0);
        let n = 400;
        let mut flux = 0.0;
        for i in 0..n {
            // Equal-area bands in the cosine.
            let c = -1.0 + (i as f32 + 0.5) * 2.0 / n as f32;
            flux +=
                dome.share(Vec3::new((1.0 - c * c).sqrt(), -c, 0.0)) * 4.0 * std::f32::consts::PI
                    / n as f32;
        }
        assert!((flux - std::f32::consts::PI).abs() < 0.01, "{flux}");
        // On the GPU: a zero axis is every way.
        assert_eq!(Emission::Every.gpu(), ([0.0; 4], [0.0; 4]));
        assert_eq!(
            Emission::STREET.gpu(),
            ([0.0, -1.0, 0.0, 0.5], [0.087, 0.0, 0.0, 0.0])
        );
    }

    #[test]
    fn a_lamp_table_keeps_its_ids_and_leaves_a_removed_set_dark() {
        let lamp = |x: f32, street: bool| Lamp {
            position: Vec3::new(x, 3.0, 0.0),
            candela: 100.0,
            regions: if street {
                [REGION_OUTSIDE; LAMP_REGIONS]
            } else {
                EVERY_REGION
            },
            ..DARK
        };
        let mut table = LampTable::new(8);
        let a = table.add(&[lamp(1.0, false), lamp(2.0, false)]).unwrap();
        assert!(!table.take_streets_changed());
        let b = table.add(&[lamp(3.0, true)]).unwrap();
        assert!(table.take_streets_changed());
        assert_eq!((table.first(a), table.first(b)), (Some(0), Some(2)));
        // A set out: its places dark, the others' ids unchanged, nothing chosen there.
        table.remove(a);
        assert_eq!(table.first(a), None);
        assert_eq!(table.lamps()[0], DARK);
        assert_eq!(table.lamps()[2].position.x, 3.0);
        let chosen = choose_shares(table.lamps(), Vec3::ZERO, LampChoice::Every);
        assert_eq!(chosen.iter().map(|c| c.0).collect::<Vec<_>>(), vec![2]);
        // The room it left is taken again first; past the table's room, nothing.
        let c = table.add(&[lamp(4.0, false)]).unwrap();
        assert_eq!(table.first(c), Some(0));
        assert!(table.add(&[lamp(5.0, false); 6]).is_none());
        // A street lamp switched off asks for the far lamps' grid again.
        table.set(
            2,
            Lamp {
                candela: 0.0,
                ..table.lamps()[2]
            },
        );
        assert!(table.take_streets_changed());
        assert!(FarLamps::new(table.lamps()).is_none());
    }
}
