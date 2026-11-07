//! Materials that change with the scene's clock (D-104): texture coordinates that scroll or step
//! through a flipbook's frames, and a glow that blinks or flickers.
//!
//! A row's motion ([`MaterialMotion`], [`crate::MeshletSceneBuilder::set_material_motion`]) is
//! worked out on the CPU each frame, in f64, at the clock the application gives
//! ([`crate::MeshletScene::set_material_clock`]). The moving rows go into the frame slot's own copy
//! of the material table, which that frame's passes read. No shader reads anything new, and a scene
//! without motions keeps its single table.
//!
//! - **What moves:** a row's UV transform (`uv_u`, `uv_v`) for a scroll or a flipbook, and its
//!   emissive colour for its glow. The texture gradients follow the transform, so a flipbook's
//!   frame takes the mip level its smaller cell asks for.
//! - **What doesn't:** the drawn geometry. The motion vectors stay zero, and the upscaler takes a
//!   scrolling picture as changed shading. The probes and the caches take a glow's changes in at
//!   their own pace. A lamp's light is the application's to switch, apart from its glowing part.
//! - **Cut-out rows can't move their coordinates:** their opacity micromaps and the rays' alpha
//!   are the row's as built.
//!
//! A row can also show pictures in turn ([`Slideshow`],
//! [`crate::MeshletSceneBuilder::set_material_slides`]): an advert board's. Each picture swaps
//! the row's colour and glow maps and its transform; a motion then applies over the picture.

use super::{GpuMaterial, MATERIAL_MULTILAYER, MATERIAL_ROOM_ATLAS, MATERIAL_UV};

/// How a material row changes with the scene's clock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaterialMotion {
    /// Texture coordinates a second, added after the row's own transform and the flipbook's frame.
    /// They wrap at one repeat, so the scroll is for a repeating sampler.
    pub scroll: [f32; 2],
    /// The row's coordinates cover one frame of a grid of them, stepping at a rate.
    pub flipbook: Option<Flipbook>,
    /// Its emissive colour's strength over time; full without one.
    pub glow: Option<Glow>,
}

/// A grid of frames in one image, played in order: along a row from the left, then the rows from
/// the image's top (v = 0) down. The row's own coordinates cover the whole image (0 to 1), and the
/// flipbook shrinks them to one frame's cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flipbook {
    /// Frames across the image.
    pub columns: u32,
    /// Frames down the image.
    pub rows: u32,
    /// Frames played, the grid's first ones; 0 plays them all.
    pub frames: u32,
    /// Frames a second.
    pub rate: f32,
}

/// A glow's strength over time, between a low level and full (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Glow {
    /// On for `duty` of each period, at `low` for the rest.
    Blink {
        /// Seconds a period.
        period: f32,
        /// The share of a period it is on, 0 to 1.
        duty: f32,
        /// Where in its period it starts, a share of the period: signs side by side apart.
        phase: f32,
        /// Its level when off.
        low: f32,
        /// The share of a period it takes to ease on and to ease off; 0 switches at once.
        smooth: f32,
    },
    /// A light that falters: `rate` times a second it may dip, with chance `chance`, to a level
    /// between `low` and full. The same `seed` falters alike on every run.
    Flicker {
        /// Steps a second.
        rate: f32,
        /// The chance a step dips, 0 to 1.
        chance: f32,
        /// The lowest level a dip reaches.
        low: f32,
        /// Its own draw: lamps with different seeds falter apart.
        seed: u32,
    },
}

/// Pictures a material row shows in turn: an advert board's slideshow. The row's colour map and its
/// glow map (where it has them) take each picture, and its transform the picture's.
#[derive(Clone, Debug, PartialEq)]
pub struct Slideshow {
    /// The pictures, in order.
    pub pictures: Vec<Slide>,
    /// Seconds each shows.
    pub seconds: f32,
    /// Seconds into the show at the clock's 0: boards side by side change apart.
    pub offset: f32,
}

/// A picture of a [`Slideshow`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    /// The picture, a sampled image's index in the bindless set
    /// ([`crate::material::TextureSet::sampled`]).
    pub picture: u32,
    /// Where the row's coordinates fall in it: `u' = dot(uv[0], (u, v, 1))` and
    /// `v' = dot(uv[1], (u, v, 1))`.
    pub uv: [[f32; 3]; 2],
}

impl Slideshow {
    /// The picture it shows at `seconds` (none without pictures).
    pub fn at(&self, seconds: f64) -> Option<&Slide> {
        if self.pictures.is_empty() || self.seconds <= 0.0 {
            return self.pictures.first();
        }
        let step = ((seconds + f64::from(self.offset)) / f64::from(self.seconds)).floor();
        let k = step.rem_euclid(self.pictures.len() as f64) as usize;
        self.pictures.get(k)
    }
}

impl MaterialMotion {
    /// Whether it moves the row's texture coordinates (a scroll or a flipbook).
    pub fn moves_coordinates(&self) -> bool {
        self.scroll != [0.0; 2] || self.flipbook.is_some()
    }

    /// The UV transform's rows at `seconds`, from the row's own: `u' = dot(u, (u, v, 1))` and
    /// `v' = dot(v, (u, v, 1))`. First the row's transform, then the flipbook's frame, then the
    /// scroll.
    pub fn transform(&self, (u, v): ([f32; 3], [f32; 3]), seconds: f64) -> ([f32; 3], [f32; 3]) {
        let (mut u, mut v) = (u, v);
        if let Some(book) = self.flipbook {
            let (columns, rows) = (book.columns.max(1), book.rows.max(1));
            let k = book.frame(seconds);
            let (column, row) = ((k % columns) as f32, (k / columns) as f32);
            let (su, sv) = (1.0 / columns as f32, 1.0 / rows as f32);
            u = [u[0] * su, u[1] * su, (u[2] + column) * su];
            v = [v[0] * sv, v[1] * sv, (v[2] + row) * sv];
        }
        // In f64 and wrapped to one repeat: an hour in, an f32 product would step by a texel.
        let shift = |speed: f32| (f64::from(speed) * seconds).rem_euclid(1.0) as f32;
        u[2] += shift(self.scroll[0]);
        v[2] += shift(self.scroll[1]);
        (u, v)
    }

    /// Its glow's strength at `seconds`, 0 to 1 (1 without one).
    pub fn glow_at(&self, seconds: f64) -> f32 {
        self.glow.map_or(1.0, |g| g.at(seconds))
    }
}

impl Flipbook {
    /// Frames it plays: `frames`, or the whole grid.
    pub fn count(&self) -> u32 {
        let grid = self.columns.max(1) * self.rows.max(1);
        if self.frames == 0 {
            grid
        } else {
            self.frames.min(grid)
        }
    }

    /// Its frame at `seconds`, from 0.
    pub fn frame(&self, seconds: f64) -> u32 {
        let step = (f64::from(self.rate) * seconds).floor();
        step.rem_euclid(f64::from(self.count())) as u32
    }
}

impl Glow {
    /// Its strength at `seconds`, 0 to 1.
    pub fn at(&self, seconds: f64) -> f32 {
        match *self {
            Glow::Blink {
                period,
                duty,
                phase,
                low,
                smooth,
            } => {
                if period <= 0.0 {
                    return 1.0;
                }
                let p = (seconds / f64::from(period) + f64::from(phase)).rem_euclid(1.0) as f32;
                low + (1.0 - low) * blink(p, duty.clamp(0.0, 1.0), smooth.max(0.0))
            }
            Glow::Flicker {
                rate,
                chance,
                low,
                seed,
            } => {
                let step = (f64::from(rate) * seconds).floor() as i64;
                let [a, b, _] = forge_core::hash::pcg3d([seed, step as u32, (step >> 32) as u32]);
                // The top 24 bits, a share in [0, 1).
                let unit = |x: u32| (x >> 8) as f32 / (1u32 << 24) as f32;
                if unit(a) < chance {
                    low + (1.0 - low) * unit(b)
                } else {
                    1.0
                }
            }
        }
    }
}

/// How on a blink is, `p` of the way through its period: on over `[0, duty)`, easing in over
/// `smooth` after the start and out over `smooth` before the end (a smoothstep each).
fn blink(p: f32, duty: f32, smooth: f32) -> f32 {
    if p >= duty {
        return 0.0;
    }
    if smooth <= 0.0 {
        return 1.0;
    }
    let s = smooth.min(duty * 0.5);
    let ease = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    ease(p / s).min(ease((duty - p) / s))
}

impl GpuMaterial {
    /// Why this row can't take `motion`, if it can't: coordinates move only on a row that
    /// reads the mesh's UVs through its transform, and not on a cut-out row.
    pub(crate) fn refuses(&self, motion: &MaterialMotion) -> Option<&'static str> {
        if !motion.moves_coordinates() {
            return None;
        }
        self.refuses_coordinates()
    }

    /// Why this row can't move its coordinates or change its pictures, if it can't.
    pub(crate) fn refuses_coordinates(&self) -> Option<&'static str> {
        if self.flags & MATERIAL_UV == 0 {
            Some("it doesn't read the mesh's UVs")
        } else if self.flags & (MATERIAL_ROOM_ATLAS | MATERIAL_MULTILAYER) != 0 {
            Some("a layered row or a room atlas keeps other things in its transform")
        } else if self.ray_masked() {
            Some("its cut-out is built once, for the rays and their opacity micromaps")
        } else {
            None
        }
    }

    /// This row showing `slide`: its colour and glow maps (where it has them) the picture, its
    /// transform the picture's.
    pub(crate) fn showing(&self, slide: &Slide) -> GpuMaterial {
        let mut row = *self;
        if row.albedo_texture != super::TEXTURE_NONE {
            row.albedo_texture = slide.picture;
        }
        if row.emissive_texture != super::TEXTURE_NONE {
            row.emissive_texture = slide.picture;
        }
        row.uv_u = slide.uv[0];
        row.uv_v = slide.uv[1];
        row
    }

    /// This row as `motion` makes it at `seconds`: its transform moved and its glow scaled.
    pub(crate) fn moved(&self, motion: &MaterialMotion, seconds: f64) -> GpuMaterial {
        let mut row = *self;
        if motion.moves_coordinates() {
            (row.uv_u, row.uv_v) = motion.transform((self.uv_u, self.uv_v), seconds);
        }
        let glow = motion.glow_at(seconds);
        row.emissive = self.emissive.map(|c| c * glow);
        row
    }

    /// A row that reads the mesh's UVs as they are and glows `emissive`, for the tests.
    #[cfg(test)]
    pub(crate) fn uv_test_row(emissive: [f32; 3]) -> GpuMaterial {
        GpuMaterial {
            flags: MATERIAL_UV,
            uv_u: [1.0, 0.0, 0.0],
            uv_v: [0.0, 1.0, 0.0],
            emissive,
            ..bytemuck::Zeroable::zeroed()
        }
    }

    /// The UV transform's rows and the emissive colour, for the tests.
    #[cfg(test)]
    pub(crate) fn motion_fields(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        (self.uv_u, self.uv_v, self.emissive)
    }

    /// The colour and glow maps, for the tests.
    #[cfg(test)]
    pub(crate) fn pictures(&self) -> (u32, u32) {
        (self.albedo_texture, self.emissive_texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: ([f32; 3], [f32; 3]) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    /// A flipbook steps at its rate, along a row and then down, and starts over after its frames.
    #[test]
    fn a_flipbook_steps_along_its_rows_then_down() {
        let book = Flipbook {
            columns: 4,
            rows: 2,
            frames: 6,
            rate: 2.0,
        };
        let frames: Vec<u32> = (0..14).map(|k| book.frame(f64::from(k) * 0.5)).collect();
        assert_eq!(frames, [0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 0, 1]);
        // Frame 5 is the second row's second cell: a quarter wide, half high.
        let motion = MaterialMotion {
            flipbook: Some(book),
            ..MaterialMotion::default()
        };
        let (u, v) = motion.transform(IDENTITY, 2.5);
        assert!(close(u, [0.25, 0.0, 0.25]), "{u:?}");
        assert!(close(v, [0.0, 0.5, 0.5]), "{v:?}");
        // No frame count plays the whole grid; before the clock's start counts back.
        let all = Flipbook { frames: 0, ..book };
        assert_eq!(all.count(), 8);
        assert_eq!(all.frame(-0.5), 7);
    }

    /// The flipbook works on the row's own transform: a row that mirrors u keeps it mirrored
    /// within the cell.
    #[test]
    fn a_flipbook_follows_the_rows_own_transform() {
        let motion = MaterialMotion {
            flipbook: Some(Flipbook {
                columns: 2,
                rows: 1,
                frames: 0,
                rate: 1.0,
            }),
            ..MaterialMotion::default()
        };
        let mirrored = ([-1.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        let (u, _) = motion.transform(mirrored, 1.0);
        // u = 0 lands at the second cell's right edge, u = 1 at its left.
        let at = |x: f32| u[0] * x + u[2];
        assert!((at(0.0) - 1.0).abs() < 1e-6 && (at(1.0) - 0.5).abs() < 1e-6);
    }

    /// A scroll moves by its speed and wraps at one repeat, as exactly an hour in as at the start.
    #[test]
    fn a_scroll_wraps_and_keeps_its_precision() {
        // Speeds an f32 holds exactly, so that the hour's offsets are the same to the bit.
        let motion = MaterialMotion {
            scroll: [0.375, -0.25],
            ..MaterialMotion::default()
        };
        let (u, v) = motion.transform(IDENTITY, 2.0);
        assert!(
            close(u, [1.0, 0.0, 0.75]) && close(v, [0.0, 1.0, 0.5]),
            "{u:?} {v:?}"
        );
        // An hour and two seconds in: the same offsets (0.375 × 3602 = 1350.75).
        let (u2, v2) = motion.transform(IDENTITY, 3602.0);
        assert!(close(u2, u) && close(v2, v), "{u2:?} {v2:?}");
    }

    /// A blink is on for its duty, at its low level otherwise, eases at its edges, and its phase
    /// shifts it.
    #[test]
    fn a_blink_holds_its_duty_and_eases_its_edges() {
        let blink = |phase: f32, smooth: f32| Glow::Blink {
            period: 2.0,
            duty: 0.25,
            phase,
            low: 0.2,
            smooth,
        };
        let hard = blink(0.0, 0.0);
        assert_eq!(hard.at(0.1), 1.0);
        assert_eq!(hard.at(0.6), 0.2);
        assert_eq!(hard.at(2.1), 1.0);
        let shifted = blink(0.5, 0.0);
        assert_eq!(shifted.at(1.1), 1.0);
        assert_eq!(shifted.at(0.1), 0.2);
        let soft = blink(0.0, 0.05);
        // Low at the start, full in the middle, low again at the end.
        assert!((soft.at(0.0) - 0.2).abs() < 1e-6);
        assert_eq!(soft.at(0.25), 1.0);
        assert!(soft.at(0.49) < 0.25);
        // Continuous: no step larger than the ease allows.
        let steps: Vec<f32> = (0..=400).map(|k| soft.at(f64::from(k) * 0.005)).collect();
        assert!(steps.windows(2).all(|w| (w[1] - w[0]).abs() < 0.2));
    }

    /// A flicker dips at about its chance, never under its low level, and alike on every run.
    #[test]
    fn a_flicker_dips_by_its_chance_and_repeats() {
        let flicker = |seed: u32| Glow::Flicker {
            rate: 10.0,
            chance: 0.25,
            low: 0.3,
            seed,
        };
        let glow = flicker(2136);
        // Step k, sampled well inside it.
        let levels_at = |glow: Glow, into: f64, n: u32| -> Vec<f32> {
            (0..n)
                .map(|k| glow.at((f64::from(k) + into) * 0.1))
                .collect()
        };
        let levels = levels_at(glow, 0.3, 4000);
        let dips = levels.iter().filter(|&&l| l < 1.0).count();
        assert!((800..1200).contains(&dips), "{dips} dips of 4000");
        assert!(levels.iter().all(|&l| (0.3..=1.0).contains(&l)));
        // The same steps give the same levels; within a step the level holds.
        assert_eq!(levels, levels_at(glow, 0.7, 4000));
        assert_ne!(levels[..100], levels_at(flicker(7), 0.3, 100)[..]);
    }

    /// A slideshow shows each picture for its seconds, in order and round again, from its offset;
    /// a row showing one takes its maps and its transform, and keeps a missing map missing.
    #[test]
    fn a_slideshow_shows_its_pictures_in_turn() {
        let slide = |picture: u32| Slide {
            picture,
            uv: [[0.5, 0.0, 0.25 * picture as f32], [0.0, 0.5, 0.0]],
        };
        let show = Slideshow {
            pictures: vec![slide(1), slide(2), slide(3)],
            seconds: 8.0,
            offset: 4.0,
        };
        let at = |t: f64| show.at(t).map(|s| s.picture);
        assert_eq!(
            [at(0.0), at(3.9), at(4.0), at(12.0), at(20.0), at(28.0)],
            [1, 1, 2, 3, 1, 2].map(Some)
        );
        assert_eq!(at(-4.5), Some(3));
        let row = GpuMaterial::uv_test_row([1.0; 3]);
        let shown = row.showing(&slide(2));
        assert_eq!(shown.pictures(), (2, 2));
        assert_eq!(shown.motion_fields().0, [0.5, 0.0, 0.5]);
        let plain = GpuMaterial {
            emissive_texture: super::super::TEXTURE_NONE,
            ..row
        };
        assert_eq!(
            plain.showing(&slide(3)).pictures(),
            (3, super::super::TEXTURE_NONE)
        );
        let empty = Slideshow {
            pictures: Vec::new(),
            seconds: 8.0,
            offset: 0.0,
        };
        assert_eq!(empty.at(1.0), None);
    }

    /// A row takes a moving transform only where it reads the mesh's UVs and isn't cut out; a glow
    /// alone suits any row.
    #[test]
    fn coordinates_move_only_on_rows_that_take_them() {
        let scroll = MaterialMotion {
            scroll: [1.0, 0.0],
            ..MaterialMotion::default()
        };
        let glow = MaterialMotion {
            glow: Some(Glow::Flicker {
                rate: 1.0,
                chance: 0.5,
                low: 0.0,
                seed: 1,
            }),
            ..MaterialMotion::default()
        };
        let row = GpuMaterial::uv_test_row([1.0; 3]);
        assert_eq!(row.refuses(&scroll), None);
        let plain = GpuMaterial { flags: 0, ..row };
        assert!(plain.refuses(&scroll).is_some());
        assert_eq!(plain.refuses(&glow), None);
        let cut = GpuMaterial {
            flags: MATERIAL_UV | super::super::MATERIAL_MASKED,
            ..row
        };
        assert!(cut.refuses(&scroll).is_some());
        // Moved: the transform shifted, the glow scaled, the rest kept.
        let both = MaterialMotion {
            scroll: [0.5, 0.0],
            glow: Some(Glow::Blink {
                period: 1.0,
                duty: 0.5,
                phase: 0.0,
                low: 0.25,
                smooth: 0.0,
            }),
            flipbook: None,
        };
        let (u, v, e) = row.moved(&both, 0.75).motion_fields();
        assert!(
            close(u, [1.0, 0.0, 0.375]) && close(v, [0.0, 1.0, 0.0]),
            "{u:?}"
        );
        assert!(close(e, [0.25; 3]), "{e:?}");
    }
}
