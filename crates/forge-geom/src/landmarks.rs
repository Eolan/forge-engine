//! Landmark buildings as part programs: a covered market hall of iron and glass, the
//! kind the old towns of the Midi built on their squares in the nineteenth century; a long, low
//! hall whose roof steps down in public terraces; a shipyard gantry crane kept. Every model
//! uses the kit's section layout ([`crate::modules`]), so the kit's material groups dress it, and
//! stands on `y = 0` round the origin, its length along `x`.

use glam::Vec3;

use crate::modules::{
    ALL, FABRIC, GLASS, METAL, NEG_Y, NEG_Z, NO_BACK, POS_X, POS_Y, POS_Z, SHUTTER, STONE, WOOD,
    cuboid, quad,
};
use crate::procedural::TriMesh;

/// A covered market hall `length` by `width` metres: a stone plinth, iron columns every bay with
/// their girders at the eaves (6 m), a pitched glass roof on iron rafters rising to a ridge with
/// a raised glazed lantern along it, and rows of market stalls under it, their awnings striped.
/// The sides stay open.
pub fn market_hall(length: f32, width: f32) -> TriMesh {
    let mut m = TriMesh::default();
    let (hl, hw) = (length * 0.5, width * 0.5);
    let eaves = 6.0;
    let rise = width * 0.22;
    let ridge = eaves + rise;
    // The plinth, two steps up.
    cuboid(
        &mut m,
        Vec3::new(-hl - 0.6, 0.0, -hw - 0.6),
        Vec3::new(hl + 0.6, 0.18, hw + 0.6),
        STONE,
        ALL & !NEG_Y,
    );
    cuboid(
        &mut m,
        Vec3::new(-hl, 0.18, -hw),
        Vec3::new(hl, 0.36, hw),
        STONE,
        ALL & !NEG_Y,
    );
    // Columns round the edge every bay, their girders along the eaves.
    let bays = (length / 6.0).round().max(2.0) as u32;
    let across = (width / 6.0).round().max(2.0) as u32;
    let column = |m: &mut TriMesh, x: f32, z: f32| {
        cuboid(
            m,
            Vec3::new(x - 0.15, 0.36, z - 0.15),
            Vec3::new(x + 0.15, eaves, z + 0.15),
            METAL,
            ALL & !NEG_Y & !POS_Y,
        );
        // Its base in stone.
        cuboid(
            m,
            Vec3::new(x - 0.3, 0.36, z - 0.3),
            Vec3::new(x + 0.3, 0.9, z + 0.3),
            STONE,
            ALL & !NEG_Y,
        );
    };
    for i in 0..=bays {
        let x = -hl + length * i as f32 / bays as f32;
        column(&mut m, x, -hw);
        column(&mut m, x, hw);
    }
    for j in 1..across {
        let z = -hw + width * j as f32 / across as f32;
        column(&mut m, -hl, z);
        column(&mut m, hl, z);
    }
    for z in [-hw, hw] {
        cuboid(
            &mut m,
            Vec3::new(-hl - 0.2, eaves - 0.5, z - 0.2),
            Vec3::new(hl + 0.2, eaves, z + 0.2),
            METAL,
            ALL,
        );
    }
    for x in [-hl, hl] {
        cuboid(
            &mut m,
            Vec3::new(x - 0.2, eaves - 0.5, -hw - 0.2),
            Vec3::new(x + 0.2, eaves, hw + 0.2),
            METAL,
            ALL,
        );
    }
    // The roof: two glass slopes and glass gables, seen from both sides; rafters every bay.
    let over = 0.8;
    let (zl, zr) = (-hw - over, hw + over);
    let fall = rise * over / hw;
    let slope = |z_edge: f32| {
        [
            Vec3::new(-hl - over, eaves - fall, z_edge),
            Vec3::new(hl + over, eaves - fall, z_edge),
            Vec3::new(hl + over, ridge, 0.0),
            Vec3::new(-hl - over, ridge, 0.0),
        ]
    };
    for z_edge in [zl, zr] {
        let p = slope(z_edge);
        quad(&mut m, p, GLASS);
        quad(&mut m, [p[1], p[0], p[3], p[2]], GLASS);
    }
    for x in [-hl, hl] {
        let g = [
            Vec3::new(x, eaves, -hw),
            Vec3::new(x, eaves, hw),
            Vec3::new(x, ridge, 0.0),
            Vec3::new(x, ridge, 0.0),
        ];
        quad(&mut m, g, GLASS);
        quad(&mut m, [g[1], g[0], g[3], g[2]], GLASS);
    }
    for i in 0..=bays {
        let x = -hl + length * i as f32 / bays as f32;
        for side in [-1.0_f32, 1.0] {
            let (a, b) = (Vec3::new(x, eaves, side * hw), Vec3::new(x, ridge, 0.0));
            // A run of short boxes under the glass, each a step of the slope.
            let d = b - a;
            let steps = 10;
            let half_z = d.z.abs() / steps as f32 * 0.5;
            for k in 0..steps {
                let p = a + d * (k as f32 + 0.5) / steps as f32 - Vec3::Y * 0.1;
                cuboid(
                    &mut m,
                    p - Vec3::new(0.08, 0.07, half_z),
                    p + Vec3::new(0.08, 0.07, half_z),
                    METAL,
                    ALL,
                );
            }
        }
    }
    // The lantern along the ridge: glass sides under a small roof.
    let lw = (width * 0.12).max(1.2);
    cuboid(
        &mut m,
        Vec3::new(-hl * 0.8, ridge - 0.3, -lw),
        Vec3::new(hl * 0.8, ridge + 1.1, lw),
        GLASS,
        ALL & !NEG_Y,
    );
    cuboid(
        &mut m,
        Vec3::new(-hl * 0.8 - 0.3, ridge + 1.1, -lw - 0.4),
        Vec3::new(hl * 0.8 + 0.3, ridge + 1.25, lw + 0.4),
        METAL,
        ALL,
    );
    // The stalls in two rows down the hall, their striped awnings.
    let stalls = (length / 4.5).floor().max(1.0) as u32;
    for row in [-1.0_f32, 1.0] {
        let z = row * hw * 0.45;
        for k in 0..stalls {
            let x = -hl + 2.0 + (length - 4.0) * (k as f32 + 0.5) / stalls as f32;
            cuboid(
                &mut m,
                Vec3::new(x - 1.6, 0.36, z - 0.8),
                Vec3::new(x + 1.6, 1.25, z + 0.8),
                WOOD,
                NO_BACK | POS_Y,
            );
            let top = 2.6;
            let canopy = [
                Vec3::new(x - 1.8, top - 0.4, z - row * 1.3),
                Vec3::new(x + 1.8, top - 0.4, z - row * 1.3),
                Vec3::new(x + 1.8, top, z + row * 0.9),
                Vec3::new(x - 1.8, top, z + row * 0.9),
            ];
            quad(&mut m, canopy, FABRIC);
            quad(&mut m, [canopy[1], canopy[0], canopy[3], canopy[2]], FABRIC);
            for dx in [-1.6, 1.6] {
                cuboid(
                    &mut m,
                    Vec3::new(x + dx - 0.04, 1.25, z + row * 0.75 - 0.04),
                    Vec3::new(x + dx + 0.04, top, z + row * 0.75 + 0.04),
                    METAL,
                    ALL & !NEG_Y & !POS_Y,
                );
            }
        }
    }
    m
}

/// A long, low civic hall `length` by `width` metres whose roof is a public terrace stepping
/// down towards `+x` in four steps (the end that faces the water): each step a white volume
/// wrapped in a glazed band, a glass balustrade along each terrace's edge, a glazed ground floor.
pub fn terraced_hall(length: f32, width: f32) -> TriMesh {
    let mut m = TriMesh::default();
    let (hl, hw) = (length * 0.5, width * 0.5);
    let steps = 4;
    let rise = 3.2;
    let ground = 4.5;
    // The glazed ground floor under the first step, set back a pace behind its columns.
    cuboid(
        &mut m,
        Vec3::new(-hl + 1.0, 0.0, -hw + 1.0),
        Vec3::new(hl - 1.0, ground, hw - 1.0),
        GLASS,
        ALL & !NEG_Y & !POS_Y,
    );
    let mut x = -hl;
    while x <= hl + 1e-3 {
        for z in [-hw + 0.3, hw - 0.3] {
            cuboid(
                &mut m,
                Vec3::new(x - 0.2, 0.0, z - 0.2),
                Vec3::new(x + 0.2, ground, z + 0.2),
                STONE,
                ALL & !NEG_Y & !POS_Y,
            );
        }
        x += length / 10.0;
    }
    for k in 0..steps {
        // Step k runs from the land end to a quarter less each time, a storey higher.
        let x1 = hl - length * k as f32 / steps as f32;
        let (y0, y1) = (ground + rise * k as f32, ground + rise * (k + 1) as f32);
        cuboid(
            &mut m,
            Vec3::new(-hl, y0, -hw),
            Vec3::new(x1, y1, hw),
            STONE,
            ALL & !NEG_Y,
        );
        // Its glazed band, a little proud of the walls.
        let (b0, b1) = (y0 + 0.6, y1 - 0.5);
        cuboid(
            &mut m,
            Vec3::new(-hl + 0.6, b0, -hw - 0.03),
            Vec3::new(x1 - 0.6, b1, hw + 0.03),
            GLASS,
            POS_Z | NEG_Z,
        );
        cuboid(
            &mut m,
            Vec3::new(x1 - 0.1, b0, -hw + 0.6),
            Vec3::new(x1 + 0.03, b1, hw - 0.6),
            GLASS,
            POS_X,
        );
        // The terrace's glass balustrade along its edge and sides.
        let top = y1;
        let next = hl - length * (k + 1) as f32 / steps as f32;
        let (bx0, bx1) = (next.max(-hl), x1);
        cuboid(
            &mut m,
            Vec3::new(bx1 - 0.08, top, -hw),
            Vec3::new(bx1, top + 1.1, hw),
            GLASS,
            ALL,
        );
        for z in [-hw, hw - 0.08] {
            cuboid(
                &mut m,
                Vec3::new(bx0, top, z),
                Vec3::new(bx1, top + 1.1, z + 0.08),
                GLASS,
                ALL,
            );
        }
        // The terrace's benches, one a bay.
        let bays = ((bx1 - bx0) / 6.0).floor() as u32;
        for b in 0..bays {
            let cx = bx0 + 3.0 + b as f32 * 6.0;
            cuboid(
                &mut m,
                Vec3::new(cx - 1.0, top, -0.3),
                Vec3::new(cx + 1.0, top + 0.45, 0.3),
                WOOD,
                ALL & !NEG_Y,
            );
        }
    }
    m
}

/// A shipyard gantry crane kept as a landmark, `span` metres between its legs along `z` and
/// `height` to its girder: two A-framed legs on their rail bogies, the girder across, the
/// trolley and its glazed cab, and a jib reaching out over the water along `+x`. Its steel is
/// [`SHUTTER`]'s section (the paint), its bogies and cables [`METAL`].
pub fn gantry_crane(span: f32, height: f32) -> TriMesh {
    let mut m = TriMesh::default();
    let hs = span * 0.5;
    let bar = |m: &mut TriMesh, a: Vec3, b: Vec3, w: f32, section: u8| {
        let d = b - a;
        let r = glam::Quat::from_rotation_arc(Vec3::Y, d.normalize());
        let mut part = TriMesh::default();
        cuboid(
            &mut part,
            Vec3::new(-w, -d.length() * 0.5, -w),
            Vec3::new(w, d.length() * 0.5, w),
            section,
            ALL,
        );
        let base = m.positions.len() as u32;
        let c = (a + b) * 0.5;
        for (p, n) in part.positions.iter().zip(&part.normals) {
            m.positions.push((c + r * Vec3::from(*p)).to_array());
            m.normals.push((r * Vec3::from(*n)).to_array());
        }
        m.indices.extend(part.indices.iter().map(|i| base + i));
        m.sections.extend(part.sections);
    };
    let girder = height;
    for z in [-hs, hs] {
        // The bogie on its rail, and the leg's A-frame up to the girder.
        cuboid(
            &mut m,
            Vec3::new(-4.0, 0.0, z - 0.8),
            Vec3::new(4.0, 1.4, z + 0.8),
            METAL,
            ALL & !NEG_Y,
        );
        for x in [-3.2, 3.2] {
            bar(
                &mut m,
                Vec3::new(x, 1.4, z),
                Vec3::new(x * 0.25, girder, z),
                0.45,
                SHUTTER,
            );
        }
        bar(
            &mut m,
            Vec3::new(-2.0, girder * 0.45, z),
            Vec3::new(2.0, girder * 0.45, z),
            0.3,
            SHUTTER,
        );
    }
    // The girder, a box across the span and on along the jib over the water.
    let reach = span * 0.9;
    cuboid(
        &mut m,
        Vec3::new(-1.2, girder, -hs - 2.0),
        Vec3::new(1.2, girder + 2.6, hs + 2.0),
        SHUTTER,
        ALL,
    );
    bar(
        &mut m,
        Vec3::new(0.0, girder + 1.3, 0.0),
        Vec3::new(reach, girder + 1.0, 0.0),
        0.9,
        SHUTTER,
    );
    // The jib's stays from a mast over the middle.
    bar(
        &mut m,
        Vec3::new(0.0, girder + 2.6, 0.0),
        Vec3::new(0.0, girder + 12.0, 0.0),
        0.5,
        SHUTTER,
    );
    for target in [reach * 0.5, reach] {
        bar(
            &mut m,
            Vec3::new(0.0, girder + 12.0, 0.0),
            Vec3::new(target, girder + 1.8, 0.0),
            0.08,
            METAL,
        );
    }
    // The trolley on the jib, its cab under it, the hook's cables.
    let tx = reach * 0.6;
    cuboid(
        &mut m,
        Vec3::new(tx - 2.0, girder - 1.2, -1.6),
        Vec3::new(tx + 2.0, girder + 0.1, 1.6),
        SHUTTER,
        ALL,
    );
    cuboid(
        &mut m,
        Vec3::new(tx - 1.2, girder - 3.6, -1.1),
        Vec3::new(tx + 1.2, girder - 1.2, 1.1),
        GLASS,
        ALL,
    );
    for z in [-0.4, 0.4] {
        bar(
            &mut m,
            Vec3::new(tx, girder - 1.2, z),
            Vec3::new(tx, girder * 0.35, z),
            0.03,
            METAL,
        );
    }
    cuboid(
        &mut m,
        Vec3::new(tx - 0.6, girder * 0.35 - 1.0, -0.6),
        Vec3::new(tx + 0.6, girder * 0.35, 0.6),
        METAL,
        ALL,
    );
    m
}

/// A belvedere's pavilion, `radius` metres round the origin: a stepped octagonal platform, eight
/// stone columns carrying a ring of entablature, and a dome of eight ribbed segments over it, a
/// lantern on top; open on every side to the view.
pub fn pavilion(radius: f32) -> TriMesh {
    use std::f32::consts::TAU;
    let mut m = TriMesh::default();
    let sides = 8;
    let corner = |r: f32, k: u32, y: f32| {
        let a = (k as f32 + 0.5) / sides as f32 * TAU;
        Vec3::new(a.cos() * r, y, a.sin() * r)
    };
    // The platform: two octagonal steps.
    for (r, y0, y1) in [(radius + 1.2, 0.0, 0.3), (radius + 0.6, 0.3, 0.6)] {
        for k in 0..sides {
            let (a, b) = (corner(r, k, y0), corner(r, k + 1, y0));
            let (c, d) = (corner(r, k + 1, y1), corner(r, k, y1));
            quad(&mut m, [b, a, d, c], STONE);
            quad(
                &mut m,
                [Vec3::new(0.0, y1, 0.0), c, d, Vec3::new(0.0, y1, 0.0)],
                STONE,
            );
        }
    }
    // The columns at the corners.
    let (base, top) = (0.6, 4.6);
    for k in 0..sides {
        let c = corner(radius, k, 0.0);
        cuboid(
            &mut m,
            Vec3::new(c.x - 0.22, base, c.z - 0.22),
            Vec3::new(c.x + 0.22, top, c.z + 0.22),
            STONE,
            ALL & !NEG_Y & !POS_Y,
        );
        cuboid(
            &mut m,
            Vec3::new(c.x - 0.32, top, c.z - 0.32),
            Vec3::new(c.x + 0.32, top + 0.2, c.z + 0.32),
            STONE,
            ALL,
        );
    }
    // The entablature: an octagonal ring, faced outside and in, its underside.
    let (e0, e1) = (top + 0.2, top + 0.9);
    let (ro, ri) = (radius + 0.45, radius - 0.45);
    for k in 0..sides {
        let (a, b) = (corner(ro, k, e0), corner(ro, k + 1, e0));
        quad(&mut m, [b, a, a.with_y(e1), b.with_y(e1)], STONE);
        let (c, d) = (corner(ri, k, e0), corner(ri, k + 1, e0));
        quad(&mut m, [c, d, d.with_y(e1), c.with_y(e1)], STONE);
        quad(&mut m, [a, b, d, c], STONE);
    }
    // The dome: segments from the ring to the crown, curved in six steps, then the lantern.
    let crown = e1 + radius * 0.8;
    let ring = |t: f32, k: u32| {
        let r = ro * (1.0 - t * t).sqrt().max(0.05);
        corner(r, k, e1 + (crown - e1) * t)
    };
    let steps = 6;
    for k in 0..sides {
        for s in 0..steps {
            let (t0, t1) = (s as f32 / steps as f32, (s + 1) as f32 / steps as f32);
            let p = [ring(t0, k), ring(t0, k + 1), ring(t1, k + 1), ring(t1, k)];
            quad(&mut m, [p[1], p[0], p[3], p[2]], METAL);
            quad(&mut m, p, METAL);
        }
    }
    cuboid(
        &mut m,
        Vec3::new(-0.4, crown - 0.1, -0.4),
        Vec3::new(0.4, crown + 1.0, 0.4),
        GLASS,
        ALL,
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::SECTIONS;

    #[test]
    fn the_terraced_hall_and_the_crane_stand_within_their_sizes() {
        for (m, half, top) in [
            (terraced_hall(60.0, 24.0), Vec3::new(30.5, 0.0, 12.5), 20.0),
            (gantry_crane(30.0, 32.0), Vec3::new(28.0, 0.0, 17.5), 47.0),
            (pavilion(5.0), Vec3::new(6.3, 0.0, 6.3), 11.0),
        ] {
            assert!(m.triangle_count() > 100);
            assert!(m.sections.iter().all(|&s| s < SECTIONS));
            for p in &m.positions {
                let p = Vec3::from(*p);
                assert!(p.x.abs() <= half.x && p.z.abs() <= half.z, "{p}");
                assert!(p.y >= -0.01 && p.y <= top, "{p}");
            }
        }
    }

    #[test]
    fn the_market_hall_stands_within_its_size() {
        let m = market_hall(42.0, 22.0);
        assert!(m.triangle_count() > 500);
        assert!(m.sections.iter().all(|&s| s < SECTIONS));
        for p in &m.positions {
            let p = Vec3::from(*p);
            assert!(
                p.x.abs() <= 22.0 && p.z.abs() <= 12.0 && p.y >= 0.0 && p.y < 14.0,
                "{p}"
            );
        }
        assert!(
            m.normals
                .iter()
                .all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-3)
        );
    }
}
