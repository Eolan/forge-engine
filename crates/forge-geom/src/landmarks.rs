//! Landmark buildings as part programs: the first is a covered market hall of iron and glass, the
//! kind the old towns of the Midi built on their squares in the nineteenth century. Every model
//! uses the kit's section layout ([`crate::modules`]), so the kit's material groups dress it, and
//! stands on `y = 0` round the origin, its length along `x`.

use glam::Vec3;

use crate::modules::{ALL, FABRIC, GLASS, METAL, NEG_Y, NO_BACK, POS_Y, STONE, WOOD, cuboid, quad};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::SECTIONS;

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
