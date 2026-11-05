//! Lamps (D-065 step 1, #270): small sphere lights the resolve lights the scene with, one shadow
//! ray each towards a point of the sphere that changes every frame, so TAA averages their
//! penumbrae as it did the sun's single ray (#54).
//!
//! A scene gives its lamps each frame ([`crate::MeshletScene::set_lamps`]): where each is, its
//! radius, its luminous intensity in candelas and its colour temperature in kelvin. Those whose
//! light reaches within 50 m of the camera are kept, the nearest first, [`MAX_LAMPS`] at most
//! ([`nearest`]); each pixel of the resolve takes every one whose [`reach`] it is in (`lamps_at` in
//! `meshlet.slang`). D-065's clustered lists (Olsson, Billeter and Assarsson 2012) come when a
//! pixel's place holds more lamps than it can afford; the probes lit by the lamps after.

use crate::columns::ColumnTables;
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};
use std::collections::BTreeMap;

/// The most lamps a frame lights with: all its buffers hold (#372, the owner's go of 2026-10-31).
/// The 64 nearest before left the Bistro's far street and arch black at night, their lamps past
/// the 64 (every lamp lit, 0.987 of Forge's path tracer with them; the 64, 0.797), for 6.6 ms of
/// its 132 lamps lit every way (the light lists' rays, filter and resolve, `docs/PROFILE.md`);
/// ReSTIR DI lit them for less but shimmered (D-065's amendment).
pub const MAX_LAMPS: usize = LAMP_CAPACITY;
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
    /// Its bulb's middle, metres (a flat piece's: its rectangle's, just in front of it,
    /// [`Emission::Flat`]).
    pub position: Vec3,
    /// Its bulb's radius, metres: the size of its shadows' penumbrae (a flat piece's: its
    /// highlight's, the disc of its area; its rectangle makes its penumbrae).
    pub radius: f32,
    /// Its luminous intensity, candelas, the same every way (a bare bulb: lumens / 4π).
    pub candela: f32,
    /// Its colour temperature, kelvin (a warm bulb 2700, daylight 6500).
    pub kelvin: f32,
    /// Its colour over its temperature's, of luminance 1 (white: [`Vec3::ONE`]): a coloured bulb
    /// or glass, as the Bistro's string lights (D-087). Their product is not brought back to
    /// luminance 1: give a tinted lamp [`WHITE_BALANCE`]'s temperature, whose colour is white,
    /// or its candelas no longer measure its luminance.
    pub tint: Vec3,
    /// The probes' regions (D-068, [`crate::RegionVolume`]) whose surfaces it lights: its room's
    /// and those it shines into through a doorway, [`NO_REGION`] in the slots left over. A
    /// surface elsewhere takes nothing of it, without a shadow ray: the house's rooms are its
    /// light lists. All slots empty ([`EVERY_REGION`]): every surface.
    pub regions: [u32; LAMP_REGIONS],
    /// Which way its light goes (#288): every way, in a cone, or from a flat face.
    /// [`Self::candela`] is the intensity at its brightest, along a cone's or a face's axis.
    pub emission: Emission,
    /// Whether its light casts shadows (#386): every lamp's does but an authored scene's whose
    /// data marks it without (a shipped city's raster mode shadows a twelfth of its lights).
    /// Without, no shadow ray is traced to it: its light reaches every surface in its reach and
    /// its regions, through what stands between.
    pub shadows: bool,
    /// The share of its light the fog scatters (D-099, [`crate::FogVolume`]): 1 for most, 0 for
    /// a lamp an authored scene keeps out of its fog (its `scaleVolFog`).
    pub fog: f32,
    /// An authored light's own range, metres: its light fades to nothing there (the shaders'
    /// window), never past its [`reach`]. None: its reach alone. A shipped city's lights end at
    /// their attenuation radius, 2 to 8 m for most of its bar's, where the reach to [`LEAST_LUX`]
    /// runs 20 m and more.
    pub range: Option<f32>,
}

impl Lamp {
    /// How far from its position its light counts, metres: its [`reach`], its [`Self::range`] if
    /// shorter (a range that is not a number counts as none); a tube's from its line, so its half
    /// length more from its position ([`Emission::extent`]).
    pub fn reach(&self) -> f32 {
        let reach = reach(self.candela);
        let own = match self.range {
            Some(r) if !r.is_nan() => reach.min(r.max(0.0)),
            _ => reach,
        };
        // A tube's runs from its line (#396, `lamp_tube_window`), so from its middle past its
        // ends. A lamp that lights nothing reaches nothing.
        if own > 0.0 {
            own + self.emission.extent()
        } else {
            0.0
        }
    }

    /// Whether it is a street lamp: [`REGION_OUTSIDE`] its first region. It lights the street
    /// and not the rooms, the probes leave it out, and ReSTIR DI lights it
    /// ([`LampChoice::Streets`]). Which way its light goes is its [`Self::emission`].
    pub fn outside(&self) -> bool {
        self.regions[0] == REGION_OUTSIDE
    }

    /// Its light in lumens, per channel of its colour of luminance 1 (`lamp_rgb` times its
    /// tint): its peak candelas over the solid angle its emission fills.
    pub fn lumens(&self) -> Vec3 {
        lamp_rgb(self.kelvin) * self.tint * (self.candela.max(0.0) * self.emission.solid_angle())
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
    /// A flat glowing piece facing `axis` (a panel, a quad of light; [`Emission::flat`]):
    /// Lambert's emitter, its intensity the cosine from the axis, none behind. It is the rectangle
    /// of half sizes `half` round [`Lamp::position`], just in front of the piece, across the axis
    /// and turned by `turn` ([`Emission::rectangle_axes`]). The shadow rays aim at points of it
    /// and end 2 cm short of them, so what covers the piece (a curtain, a window's recess) shades
    /// its light (#370). Its unshadowed light is the rectangle's exact diffuse light, not its
    /// middle's (#369). A sphere's disc faces each surface instead; [`Lamp::radius`] stays the
    /// highlight's size. ReSTIR DI (`LampRestir`) samples the rectangle by area (#372).
    Flat {
        /// The way its face looks, unit.
        axis: Vec3,
        /// The turn of its first side about the axis, radians, from the first of
        /// [`Emission::rectangle_axes`]' basis.
        turn: f32,
        /// Its half sizes along its first side and its second, metres (both over 0).
        half: Vec2,
    },
    /// A lantern: glass round its upright sides, between a cap and a foot that may be glazed
    /// (#372, the Bistro's street lanterns). Its intensity at the elevation φ over its side's (its
    /// candela, the peak, level) is that of a prism of upright panes, cos φ, and of its glass seen
    /// from above and below: `up` × sin φ upwards, `down` × −sin φ downwards, the shares of its
    /// glass's area seen from straight above and below over that seen from the side
    /// (`Glow::beams` in physics-lab). The cone before it ([`Emission::Cone`] down, whole from 30°
    /// above the horizon) sent the side's whole intensity straight down, where the Bistro's
    /// lanterns show 0.32 of it.
    Prism {
        /// Its glass's area seen from above over that seen from the side.
        up: f32,
        /// Its glass's area seen from below over that seen from the side.
        down: f32,
    },
    /// A tube or a strip of light (#396): a segment `2 × half_length` long along `axis` (unit)
    /// through [`Lamp::position`], [`Lamp::radius`] thick, glowing as a Lambertian cylinder. Its
    /// intensity is its candela across its axis times the sine of the angle from it, its flux π²
    /// times that candela (π × its luminance × its side's area). Its diffuse light is its line's,
    /// in closed form, clipped at a surface's horizon (`lamp_tube_share`); its highlight is lit
    /// from the point of it nearest the mirror ray (after Karis 2013 and Picott 1992); its shadow
    /// rays aim at points along it. Its light still falls off as one over the distance from its
    /// line: laid a few centimetres before a surface, it burns a band along itself.
    Tube {
        /// The way it runs, unit.
        axis: Vec3,
        /// Half its length, metres.
        half_length: f32,
    },
}

impl Emission {
    /// A tube or strip along `axis` (any length but not zero), `half_length` metres each side of
    /// its middle ([`Emission::Tube`]).
    pub fn tube(axis: Vec3, half_length: f32) -> Emission {
        debug_assert!(
            axis != Vec3::ZERO && half_length > 0.0,
            "a tube needs an axis and a length: {axis} {half_length}"
        );
        // Two millimetres at least: a tube of none would light nothing by its line and its
        // highlight and ReSTIR's ribbon still would.
        Emission::Tube {
            axis: axis.normalize(),
            half_length: half_length.max(1e-3),
        }
    }

    /// How far its light reaches beyond its middle's: a tube's half length, else 0.
    pub fn extent(&self) -> f32 {
        match *self {
            Emission::Tube { half_length, .. } => half_length.max(0.0),
            _ => 0.0,
        }
    }

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

    /// A flat piece facing `axis` (any length but not zero), its rectangle turned by `turn` and of
    /// half sizes `half` ([`Emission::Flat`]).
    pub fn flat(axis: Vec3, turn: f32, half: Vec2) -> Emission {
        debug_assert!(
            axis != Vec3::ZERO && half.min_element() > 0.0,
            "a flat piece needs an axis and both half sizes: {axis} {half}"
        );
        Emission::Flat {
            axis: axis.normalize(),
            turn,
            half,
        }
    }

    /// The basis a flat piece's turn counts from: two unit axes across the unit `axis`, as
    /// `lamp_disc_axes` in `meshlet.slang` makes them.
    pub fn rectangle_axes(axis: Vec3) -> (Vec3, Vec3) {
        let up = if axis.y.abs() < 0.99 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let t = up.cross(axis).normalize();
        (t, axis.cross(t))
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
            Emission::Flat { axis, .. } => out.dot(axis).max(0.0),
            Emission::Prism { up, down } => {
                let s = out.y.clamp(-1.0, 1.0);
                (1.0 - s * s).sqrt() + up * s.max(0.0) + down * (-s).max(0.0)
            }
            Emission::Tube { axis, .. } => {
                let c = out.dot(axis).clamp(-1.0, 1.0);
                (1.0 - c * c).sqrt()
            }
        }
    }

    /// The solid angle its light fills, steradians, as the peak's: its lumens over its candelas.
    /// Every way 4π; a cone 2π of the cosine it spans whole and half the step's; a flat piece π; a
    /// lantern π² for its sides and π for each share above and below.
    pub fn solid_angle(&self) -> f32 {
        match *self {
            Emission::Every => 4.0 * std::f32::consts::PI,
            Emission::Cone {
                inner_cos,
                outer_cos,
                ..
            } => 2.0 * std::f32::consts::PI * ((1.0 - inner_cos) + 0.5 * (inner_cos - outer_cos)),
            Emission::Flat { .. } => std::f32::consts::PI,
            Emission::Prism { up, down } => {
                let pi = std::f32::consts::PI;
                pi * pi + pi * (up + down)
            }
            Emission::Tube { .. } => std::f32::consts::PI * std::f32::consts::PI,
        }
    }

    /// Its two vectors of [`GpuLamp`]: the axis and the full light's cosine, then the cut-off's
    /// and a flat piece's turn and half sizes. Every way: a zero axis.
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
            Emission::Flat { axis, turn, half } => {
                ([axis.x, axis.y, axis.z, 1.0], [0.0, turn, half.x, half.y])
            }
            Emission::Prism { up, down } => ([0.0, 1.0, 0.0, 0.0], [up, down, -1.0, 0.0]),
            Emission::Tube { axis, half_length } => (
                [axis.x, axis.y, axis.z, 0.0],
                [half_length.max(0.0), 0.0, -2.0, 0.0],
            ),
        }
    }
}

/// A tube's diffuse light on a surface at `world` facing `n` (unit), per candela of its peak
/// intensity (#396): the integral along it of the sine from its axis times the surface's cosine
/// over the distance squared, over its length, the segment clipped at the surface's horizon, in
/// closed form. The CPU mirror of `lamp_tube_share` in `meshlet.slang`. `middle`, `axis` (unit),
/// `half_length` and `radius` (the nearest a surface takes its line, metres) are the tube's.
pub fn tube_share(
    middle: Vec3,
    axis: Vec3,
    half_length: f32,
    radius: f32,
    world: Vec3,
    n: Vec3,
) -> f32 {
    let c = middle - world;
    let along = c.dot(axis);
    let across = c - axis * along;
    let h = across.length().max(radius.max(1e-3));
    // On its axis none, as the GPU's.
    let normal = if across.length() > 1e-6 {
        across / across.length()
    } else {
        Vec3::ZERO
    };
    let (nn, nt) = (n.dot(normal), n.dot(axis));
    let (mut s1, mut s2) = (along - half_length, along + half_length);
    if nt.abs() > 1e-6 {
        let s0 = -h * nn / nt;
        if nt > 0.0 {
            s1 = s1.max(s0);
        } else {
            s2 = s2.min(s0);
        }
    } else if nn <= 0.0 {
        return 0.0;
    }
    if s2 <= s1 {
        return 0.0;
    }
    let g = |s: f32| {
        let q = h * h + s * s;
        0.5 * (nn * (s / q + (s / h).atan() / h) - nt * h / q)
    };
    ((g(s2) - g(s1)) / (2.0 * half_length.max(1e-6))).max(0.0)
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
    /// x: the cosine beyond which a cone's light is none; for a flat piece ([`Emission::Flat`]):
    /// y its turn, zw its half sizes (z over 0 marks it flat); for a lantern
    /// ([`Emission::Prism`]) its shares up and down, z -1; for a tube ([`Emission::Tube`]) x its
    /// half length, z -2; else yzw 0.
    pub beam_edge: [f32; 4],
    /// x: [`LAMP_SHADOWLESS`] when it casts no shadow ([`Lamp::shadows`]); y: one less
    /// [`Lamp::fog`], as float bits (a zeroed record lights the fog whole); zw 0.
    pub flags: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuLamp>() == 112);

/// [`GpuLamp::flags`]: a lamp without shadows (`LAMP_SHADOWLESS` in `meshlet.slang`).
pub const LAMP_SHADOWLESS: u32 = 1;

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
    /// As `Nearest` for every lamp but the street lamps, [`STREETS_HOUSES`] at most, and every
    /// street lamp, however far: ReSTIR DI lights those (D-065, the owner's choice of
    /// 2026-10-18), so a far street keeps its light.
    Streets,
    /// Every lamp that is on, however far, [`LAMP_CAPACITY`] at most: D-065's comparison of
    /// ReSTIR DI with the light lists, which takes them all.
    Every,
}

/// The street lamps' share of [`MAX_LAMPS`] in [`LampChoice::Nearest`] (#295): a few hundred
/// house lamps nearer the camera crowded them out, and a lit street went dark.
pub const STREET_SHARE: usize = 24;

/// The house lamps' share in [`LampChoice::Streets`], the rest of [`LAMP_CAPACITY`] the street
/// lamps': with [`MAX_LAMPS`] the buffers' whole (#372), a few hundred house lamps near the camera
/// left the streets none.
pub const STREETS_HOUSES: usize = 64;

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
                tint: Vec3::ONE,
                regions,
                emission: Emission::STREET,
                shadows: true,
                fog: 1.0,
                range: None,
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
            let h = houses.len().min(STREETS_HOUSES);
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
        // A lamp of no reach (a range of 0) lights nothing: it takes no place.
        .filter(|(_, l)| l.candela > 0.0 && l.reach() > 0.0 && pick(l))
        .map(|(i, l)| (l.position.distance(camera), i))
        .filter(|&(d, i)| d < lamps[i].reach() + margin)
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
        let edge = lamps[i].reach() + margin;
        let share = next.map_or(1.0, out) * if edge.is_finite() { out(edge) } else { 1.0 };
        (d, i, share)
    })
}

/// `l` as the GPU takes it, its light times `share`.
pub(crate) fn gpu_lamp(l: &Lamp, share: f32) -> GpuLamp {
    let c = lamp_rgb(l.kelvin) * l.tint * (l.candela * share);
    let (beam, beam_edge) = l.emission.gpu();
    GpuLamp {
        position_radius: [
            l.position.x,
            l.position.y,
            l.position.z,
            l.radius.max(0.005),
        ],
        colour_reach: [c.x, c.y, c.z, l.reach()],
        regions: l.regions,
        beam,
        beam_edge,
        // y: the share of its light the fog does not scatter (`1 − fog`), so a record left zero
        // lights the fog whole.
        flags: [
            if l.shadows { 0 } else { LAMP_SHADOWLESS },
            (1.0 - l.fog.clamp(0.0, 4.0)).to_bits(),
            0,
            0,
        ],
    }
}

/// A lamp that gives no light: a [`LampTable`]'s free place. Every choice and the far lamps'
/// grid pass over it.
pub const DARK: Lamp = Lamp {
    position: Vec3::ZERO,
    radius: 0.0,
    candela: 0.0,
    kelvin: WHITE_BALANCE,
    tint: Vec3::ONE,
    regions: EVERY_REGION,
    emission: Emission::Every,
    shadows: true,
    fog: 1.0,
    range: None,
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
/// Only the lamps whose light reaches within 50 m of the camera are lit with shadow rays (when
/// this was made, the 64 nearest), and the far streets' pools went dark while their lamp heads
/// kept glowing; a lamp leaving the set fades into this as it fades out of it, and back.
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
            let r = l.reach();
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
/// `meshlet.slang`) and loops over its few lamps, not over the frame's every lamp (64 cost
/// 0.5 ms in a hall).
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

/// A column's side in [`NearLamps`], metres (#386): as [`FAR_CELL`].
pub const NEAR_CELL: f32 = 8.0;
/// [`NearLamps`]' columns along x and along z round the camera: 256 m of [`NEAR_CELL`]. A point
/// beyond weighs every lamp, as before the grid.
pub const NEAR_SIDE: u32 = 32;

/// Mirrors `NearLamps` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub(crate) struct GpuNearLamps {
    pub columns: u64,
    pub indices: u64,
    pub origin: [f32; 2],
    pub cell: f32,
    pub side: u32,
    pub count: u32,
    pub pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<GpuNearLamps>() == 48);

/// The frame's lamps, all of every region ([`EVERY_REGION`]), by the columns of a grid of
/// [`NEAR_SIDE`] × [`NEAR_SIDE`] columns of [`NEAR_CELL`] metres round the camera (#386): each
/// column lists, in increasing order, the lamps whose reach (`colour_reach.w`, as the shader lets
/// a lamp go) meets it across the ground. Made only when no lamp of the frame has regions, so
/// every lamp is under the lists' REGION_ANY: a mirror ray's hit and a pixel's lamp place (the
/// light lists, ReSTIR's candidates) take their column's lamps rather than the frame's every one,
/// an authored district's hundreds, most of them beyond reach. A lamp beyond its
/// reach weighs nothing, and the distance across the ground is never more than the distance, so
/// a column leaves out no lamp that lights a point in it, and the sums and picks are the same.
pub(crate) struct NearLamps {
    /// The grid's corner (x, z) in the scene frame, a whole number of columns.
    pub origin: Vec2,
    /// Per column, rows of increasing z: (first, count) in [`Self::indices`].
    pub columns: Vec<[u32; 2]>,
    /// The lamps' indices in the frame's list, column after column.
    pub indices: Vec<u32>,
    /// The lamps gridded: the frame's lamps of every region.
    pub count: u32,
}

impl NearLamps {
    /// The grid of `kept`'s lamps round `camera` (the scene frame); `None` when one of them has
    /// regions (the light lists then), or there are none.
    pub(crate) fn new(kept: &[GpuLamp], camera: Vec3) -> Option<Self> {
        if kept.is_empty() || kept.iter().any(|l| l.regions != EVERY_REGION) {
            return None;
        }
        let half = NEAR_SIDE as f32 * 0.5;
        let origin = ((Vec2::new(camera.x, camera.z) / NEAR_CELL).floor() - half) * NEAR_CELL;
        let side = NEAR_SIDE as i32;
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); (NEAR_SIDE * NEAR_SIDE) as usize];
        let mut count = 0;
        for (i, l) in kept.iter().enumerate() {
            count += 1;
            if l.colour_reach[3] <= 0.0 {
                continue;
            }
            // A hair over its reach, the bounds as the test: the shader's test is on the
            // distance squared, rounded in the scene frame.
            let reach = l.colour_reach[3] * 1.001 + 0.01;
            let at = Vec2::new(l.position_radius[0], l.position_radius[2]) - origin;
            let lo = ((at - reach) / NEAR_CELL)
                .floor()
                .as_ivec2()
                .max(glam::IVec2::ZERO);
            let hi = ((at + reach) / NEAR_CELL)
                .floor()
                .as_ivec2()
                .min(glam::IVec2::splat(side - 1));
            for z in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    // The column's nearest point to the lamp, within its reach.
                    let corner = Vec2::new(x as f32, z as f32) * NEAR_CELL;
                    if at.clamp(corner, corner + NEAR_CELL).distance(at) <= reach {
                        lists[(z * side + x) as usize].push(i as u32);
                    }
                }
            }
        }
        let mut columns = Vec::with_capacity(lists.len());
        let mut indices = Vec::new();
        for list in &lists {
            columns.push([indices.len() as u32, list.len() as u32]);
            indices.extend_from_slice(list);
        }
        Some(Self {
            origin,
            columns,
            indices,
            count,
        })
    }

    /// The lamps listed for the column of `point` (the scene frame); `None` beyond the grid.
    #[cfg(test)]
    fn at(&self, point: Vec3) -> Option<&[u32]> {
        let c = ((Vec2::new(point.x, point.z) - self.origin) / NEAR_CELL).floor();
        if c.min_element() < 0.0 || c.max_element() >= NEAR_SIDE as f32 {
            return None;
        }
        let [first, n] = self.columns[(c.y as u32 * NEAR_SIDE + c.x as u32) as usize];
        Some(&self.indices[first as usize..(first + n) as usize])
    }
}

#[cfg(test)]
mod tube_tests {
    use super::*;

    /// The tube's light by a midpoint sum along it: the sine from its axis times the surface's
    /// cosine over the distance squared, per candela, over its length.
    fn summed(middle: Vec3, axis: Vec3, half: f32, world: Vec3, n: Vec3) -> f64 {
        let steps = 20_000;
        let ds = 2.0 * f64::from(half) / f64::from(steps);
        let mut sum = 0.0f64;
        for k in 0..steps {
            let s = -f64::from(half) + (f64::from(k) + 0.5) * ds;
            let p = middle.as_dvec3() + axis.as_dvec3() * s;
            let d = p - world.as_dvec3();
            let dist = d.length();
            let cos_r = n.as_dvec3().dot(d) / dist;
            if cos_r <= 0.0 {
                continue;
            }
            let sin_e = (1.0 - (d.dot(axis.as_dvec3()) / dist).powi(2))
                .max(0.0)
                .sqrt();
            sum += sin_e * cos_r / (dist * dist) * ds;
        }
        sum / (2.0 * f64::from(half))
    }

    #[test]
    fn a_tubes_closed_form_matches_its_integral() {
        let middle = Vec3::new(0.0, 3.0, 0.0);
        let cases = [
            // Under its middle, facing it; under an end; beside it, a wall; tilted; half under
            // the horizon (a wall at the tube's end, the segment crossing its plane).
            (Vec3::X, Vec3::ZERO, Vec3::Y),
            (Vec3::X, Vec3::new(1.2, 0.0, 0.0), Vec3::Y),
            (Vec3::X, Vec3::new(0.3, 2.5, 0.8), Vec3::NEG_Z),
            (
                Vec3::new(1.0, 0.3, 0.2).normalize(),
                Vec3::new(-0.5, 1.0, 0.4),
                Vec3::new(0.2, 0.9, -0.3).normalize(),
            ),
            (Vec3::X, Vec3::new(0.6, 3.1, 0.0), Vec3::NEG_X),
        ];
        for (axis, world, n) in cases {
            let half = 1.5;
            let closed = f64::from(tube_share(middle, axis, half, 0.01, world, n));
            let sum = summed(middle, axis, half, world, n);
            assert!(
                (closed - sum).abs() <= 2e-3 * sum.max(1e-3),
                "{world} {n}: {closed} against {sum}"
            );
        }
        // Far away, a point of its candela: across its axis, facing it.
        let far = tube_share(
            Vec3::ZERO,
            Vec3::X,
            0.5,
            0.01,
            Vec3::new(0.0, -100.0, 0.0),
            Vec3::Y,
        );
        assert!((far - 1e-4).abs() < 2e-7, "{far}");
        // Behind the surface: none.
        assert_eq!(
            tube_share(middle, Vec3::X, 1.0, 0.01, Vec3::ZERO, Vec3::NEG_Y),
            0.0
        );
    }

    #[test]
    fn a_tube_keeps_its_flux_reach_and_record() {
        let pi = std::f32::consts::PI;
        let tube = Emission::tube(Vec3::Z * 2.0, 4.5);
        assert!((tube.solid_angle() - pi * pi).abs() < 1e-5);
        assert_eq!(tube.share(Vec3::Z), 0.0);
        assert!((tube.share(Vec3::X) - 1.0).abs() < 1e-6);
        assert_eq!(tube.extent(), 4.5);
        let lamp = Lamp {
            emission: tube,
            candela: 100.0,
            range: Some(25.0),
            ..DARK
        };
        // Its range past its line, and so without one its light's reach.
        assert_eq!(lamp.reach(), 29.5);
        let unranged = Lamp {
            range: None,
            ..lamp
        };
        assert!((unranged.reach() - (reach(100.0) + 4.5)).abs() < 1e-4);
        let g = gpu_lamp(&lamp, 1.0);
        assert_eq!(g.beam, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(g.beam_edge, [4.5, 0.0, -2.0, 0.0]);
        // Its range from its line, as `lamp_tube_window` reads it back.
        assert_eq!(g.colour_reach[3] - g.beam_edge[0], 25.0);
        // Any other lamp's reach is its own.
        assert_eq!(
            Lamp {
                candela: 100.0,
                ..DARK
            }
            .reach(),
            reach(100.0)
        );
        // A tube that lights nothing reaches nothing.
        let unlit = Lamp {
            emission: tube,
            candela: 100.0,
            range: Some(0.0),
            ..DARK
        };
        assert_eq!(unlit.reach(), 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_lists_every_lamp_of_every_region_that_reaches_it_in_order() {
        // Lamps scattered round the camera, some with regions (left to the lists), one dark.
        let mut kept = Vec::new();
        let mut h = 0x9E37_79B9_u32;
        let mut next = || {
            h ^= h << 13;
            h ^= h >> 17;
            h ^= h << 5;
            h as f32 / u32::MAX as f32
        };
        for i in 0..300 {
            let mut regions = EVERY_REGION;
            if i % 7 == 0 {
                regions[0] = 3;
            }
            let reach = if i % 50 == 1 {
                0.0
            } else {
                2.0 + next() * 60.0
            };
            kept.push(GpuLamp {
                position_radius: [
                    next() * 400.0 - 200.0,
                    next() * 30.0,
                    next() * 400.0 - 200.0,
                    0.05,
                ],
                colour_reach: [1.0, 1.0, 1.0, reach],
                regions,
                ..GpuLamp::default()
            });
        }
        let camera = Vec3::new(13.0, 2.0, -40.0);
        assert!(
            NearLamps::new(&kept, camera).is_none(),
            "lamps with regions: the lists"
        );
        kept.retain(|l| l.regions == EVERY_REGION);
        let grid = NearLamps::new(&kept, camera).expect("every lamp of every region");
        assert_eq!(
            grid.count,
            kept.iter().filter(|l| l.regions == EVERY_REGION).count() as u32
        );
        // Points over the grid and past it: every lamp of every region within its reach of a
        // point is in the point's column, in increasing order; beyond the grid, none.
        for _ in 0..2000 {
            let point = Vec3::new(
                camera.x + next() * 320.0 - 160.0,
                next() * 30.0,
                camera.z + next() * 320.0 - 160.0,
            );
            let Some(list) = grid.at(point) else {
                let c = (Vec2::new(point.x, point.z) - grid.origin) / NEAR_CELL;
                assert!(c.min_element() < 0.0 || c.max_element() >= NEAR_SIDE as f32);
                continue;
            };
            assert!(list.windows(2).all(|w| w[0] < w[1]));
            for (i, l) in kept.iter().enumerate() {
                let lit = l.regions == EVERY_REGION
                    && Vec3::from_slice(&l.position_radius[..3]).distance_squared(point)
                        < l.colour_reach[3] * l.colour_reach[3];
                if lit {
                    assert!(
                        list.contains(&(i as u32)),
                        "lamp {i} lights {point} but is not listed"
                    );
                }
                if l.regions != EVERY_REGION {
                    assert!(!list.contains(&(i as u32)));
                }
            }
        }
        // The camera's column is in the middle of the grid.
        assert!(grid.at(camera).is_some());
    }

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
    fn a_range_shortens_the_reach_and_never_lengthens_it() {
        let lamp = |range: Option<f32>| Lamp {
            range,
            candela: 100.0,
            ..DARK
        };
        // 100 cd reaches about 45 m.
        assert!((lamp(None).reach() - 44.72).abs() < 0.01);
        assert_eq!(lamp(Some(4.0)).reach(), 4.0);
        assert!((lamp(Some(100.0)).reach() - 44.72).abs() < 0.01);
        assert_eq!(lamp(Some(-1.0)).reach(), 0.0);
        assert!((lamp(Some(f32::NAN)).reach() - 44.72).abs() < 0.01);
        assert_eq!(gpu_lamp(&lamp(Some(4.0)), 1.0).colour_reach[3], 4.0);
        // A lamp of no reach takes no place among the frame's.
        let kept = nearest(&[lamp(Some(0.0)), lamp(Some(4.0))], Vec3::ZERO);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].colour_reach[3], 4.0);
    }

    #[test]
    fn the_frame_keeps_the_nearest_lamps_that_reach_the_camera() {
        let bulb = |x: f32, candela: f32| Lamp {
            position: Vec3::new(x, 2.0, 0.0),
            radius: 0.05,
            candela,
            kelvin: 2700.0,
            tint: Vec3::ONE,
            regions: EVERY_REGION,
            emission: Emission::Every,
            shadows: true,
            fog: 1.0,
            range: None,
        };
        // 100 cd reaches about 45 m.
        assert!((reach(100.0) - 44.72).abs() < 0.01);
        // A row a metre apart: those within the reach and 50 m of the camera (95 of them) are
        // kept, the nearest first.
        let lamps: Vec<Lamp> = (0..100)
            .map(|i| bulb(i as f32, 100.0))
            .chain([bulb(5000.0, 100.0), bulb(1.0, 0.0)])
            .collect();
        let kept = nearest(&lamps, Vec3::ZERO);
        assert_eq!(kept.len(), 95);
        // Six hundred in a 10 cm row: the nearest MAX_LAMPS.
        let crowd: Vec<Lamp> = (0..600).map(|i| bulb(i as f32 * 0.1, 100.0)).collect();
        assert_eq!(nearest(&crowd, Vec3::ZERO).len(), MAX_LAMPS);
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
                tint: Vec3::ONE,
                regions,
                emission: Emission::Every,
                shadows: true,
                fog: 1.0,
                range: None,
            }
        };
        // Six hundred house lamps within 10 m (more than MAX_LAMPS), thirty street lamps 60 to
        // 89 m away (109 cd reach 47 m, so they count to 97 m).
        let lamps: Vec<Lamp> = (0..600)
            .map(|i| {
                lamp(
                    Vec3::new((i % 25) as f32 * 0.4, 2.0, (i / 25) as f32 * 0.4),
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
        // With ReSTIR DI for the streets (`--street-restir`), the houses keep their 64 and every
        // street lamp is kept.
        let restir = choose(&lamps, Vec3::ZERO, LampChoice::Streets);
        let streets = restir
            .iter()
            .filter(|l| l.regions[0] == REGION_OUTSIDE)
            .count();
        assert_eq!((restir.len(), streets), (STREETS_HOUSES + 30, 30));
        // Without the houses, the streets take every slot they need.
        let alone: Vec<Lamp> = lamps[600..].to_vec();
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
                tint: Vec3::ONE,
                regions,
                emission: Emission::STREET,
                shadows: true,
                fog: 1.0,
                range: None,
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
        // A thousand lamps on a 2 m grid; the camera walks 40 m across it in 1 cm steps. Each
        // lamp's light (its red intensity) may change little from step to step, though dozens
        // enter and leave the MAX_LAMPS kept.
        let lamps: Vec<Lamp> = (0..1000)
            .map(|i| Lamp {
                position: Vec3::new((i % 50) as f32 * 2.0, 2.5, (i / 50) as f32 * 2.0),
                radius: 0.05,
                candela: 50.0,
                kelvin: 2700.0,
                tint: Vec3::ONE,
                regions: EVERY_REGION,
                emission: Emission::Every,
                shadows: true,
                fog: 1.0,
                range: None,
            })
            .collect();
        let light = |camera: Vec3| {
            let mut out = vec![0.0_f32; lamps.len()];
            for l in nearest(&lamps, camera) {
                let column = (l.position_radius[0] / 2.0).round() as usize;
                let row = (l.position_radius[2] / 2.0).round() as usize;
                out[column + row * 50] = l.colour_reach[0];
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
        // A flat piece facing down (#369, #370): Lambert's cosine, the same lumens as the dome's,
        // its rectangle's turn and half sizes on the GPU (which mark it flat).
        let panel = Emission::flat(Vec3::NEG_Y * 2.0, 0.5, Vec2::new(0.6, 0.4));
        assert_eq!(panel.share(Vec3::Y), 0.0);
        assert_eq!(panel.share(Vec3::NEG_Y), 1.0);
        assert!((panel.share(Vec3::new(0.0, -0.5, 0.75_f32.sqrt())) - 0.5).abs() < 1e-6);
        let flux: f32 = (0..n)
            .map(|i| {
                let c = -1.0 + (i as f32 + 0.5) * 2.0 / n as f32;
                panel.share(Vec3::new((1.0 - c * c).sqrt(), -c, 0.0)) * 4.0 * std::f32::consts::PI
                    / n as f32
            })
            .sum();
        assert!((flux - panel.solid_angle()).abs() < 0.01, "{flux}");
        assert_eq!(panel.gpu(), ([0.0, -1.0, 0.0, 1.0], [0.0, 0.5, 0.6, 0.4]));
        // A lantern (#372): its side's whole light level, its glass's shares straight up and down,
        // its lumens the solid angle's; on the GPU its axis up and -1 marking it.
        let lantern = Emission::Prism {
            up: 0.0,
            down: 0.32,
        };
        assert!((lantern.share(Vec3::X) - 1.0).abs() < 1e-6);
        assert!(lantern.share(Vec3::Y).abs() < 1e-6);
        assert!((lantern.share(Vec3::NEG_Y) - 0.32).abs() < 1e-6);
        let flux: f32 = (0..n)
            .map(|i| {
                let c = -1.0 + (i as f32 + 0.5) * 2.0 / n as f32;
                lantern.share(Vec3::new((1.0 - c * c).sqrt(), -c, 0.0)) * 4.0 * std::f32::consts::PI
                    / n as f32
            })
            .sum();
        assert!((flux - lantern.solid_angle()).abs() < 0.01, "{flux}");
        assert_eq!(
            lantern.gpu(),
            ([0.0, 1.0, 0.0, 0.0], [0.0, 0.32, -1.0, 0.0])
        );
        // Its basis is the shader's: across the axis, right-handed about it.
        for axis in [
            Vec3::NEG_Y,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 2.0, -3.0).normalize(),
        ] {
            let (t, b) = Emission::rectangle_axes(axis);
            assert!(t.dot(axis).abs() < 1e-6 && b.dot(axis).abs() < 1e-6 && t.dot(b).abs() < 1e-6);
            assert!((t.cross(b) - axis).length() < 1e-5, "{axis}");
        }
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
