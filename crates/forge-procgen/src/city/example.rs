//! A generic example city for the engine's tests, tools and docs: a square frame with a sea to
//! the south-east, a meandering river into it, an old town on the river, perimeter blocks
//! round it, a waterfront, an eco-quarter, a river park and two tram lines. Invented; no real
//! place.

use forge_core::{Seed, dmath};
use glam::DVec2;

use super::{
    BoundaryInfluence, CityParams, DistrictKind, DistrictSpec, LandmarkSpec, Pattern, SEA, Site,
    TramLineSpec, Waterway,
};

/// The example's site: `size` metres square, its masks in cells of `cell` metres.
pub fn site(size: f64, cell: f64) -> Site {
    let mut site = Site::flat(size, cell);
    let n = site.water.size;
    // The coast: a gentle curve across the south-east corner.
    for y in 0..n {
        for x in 0..n {
            let p = DVec2::new(x as f64 * cell, y as f64 * cell) / size;
            let coast = 1.55 - 0.08 * dmath::sin(p.y * 7.0);
            if p.x + p.y > coast {
                site.water.set(x, y, SEA);
            }
        }
    }
    // The river from the west edge to the sea, meandering.
    let centreline: Vec<DVec2> = (0..=60)
        .map(|i| {
            let t = i as f64 / 60.0;
            let x = t * size * 0.86;
            let y = size * (0.42 + 0.30 * t + 0.05 * dmath::sin(t * 9.0));
            DVec2::new(x, y)
        })
        .collect();
    site.rivers.push(Waterway {
        name: "river".into(),
        centreline,
        width: 90.0,
    });
    site.paint_rivers();
    site
}

/// The example's parameters for a site of `size` metres.
pub fn params(size: f64) -> CityParams {
    let at = |x: f64, y: f64| DVec2::new(x * size, y * size);
    let district = |name: &str,
                    kind: DistrictKind,
                    anchor: DVec2,
                    radius: f64,
                    pattern: Pattern,
                    angle: f64,
                    spacing: f64,
                    lot: f64,
                    floors: (u8, u8)| DistrictSpec {
        name: name.into(),
        kind,
        anchor,
        radius,
        pattern,
        grid_angle: angle,
        street_spacing: spacing,
        lot_area: lot,
        floors,
    };
    let r = size;
    CityParams {
        seed: Seed::new(20_261_011),
        districts: vec![
            district(
                "old town",
                DistrictKind::OldTown,
                at(0.42, 0.50),
                0.13 * r,
                Pattern::Organic(1.4),
                0.35,
                70.0,
                220.0,
                (3, 5),
            ),
            district(
                "ring north",
                DistrictKind::Perimeter,
                at(0.40, 0.30),
                0.22 * r,
                Pattern::Grid,
                0.12,
                115.0,
                450.0,
                (6, 8),
            ),
            district(
                "ring south",
                DistrictKind::Perimeter,
                at(0.48, 0.75),
                0.22 * r,
                Pattern::Grid,
                0.62,
                115.0,
                450.0,
                (6, 8),
            ),
            district(
                "waterfront",
                DistrictKind::Waterfront,
                at(0.78, 0.62),
                0.16 * r,
                Pattern::Grid,
                -0.75,
                130.0,
                700.0,
                (5, 9),
            ),
            district(
                "eco-quarter",
                DistrictKind::Eco,
                at(0.14, 0.24),
                0.18 * r,
                Pattern::Grid,
                0.0,
                160.0,
                900.0,
                (5, 8),
            ),
            district(
                "river park",
                DistrictKind::Park,
                at(0.12, 0.50),
                0.12 * r,
                Pattern::Grid,
                0.3,
                200.0,
                1000.0,
                (0, 0),
            ),
            district(
                "courtyards",
                DistrictKind::Courtyard,
                at(0.20, 0.80),
                0.20 * r,
                Pattern::Grid,
                0.5,
                125.0,
                500.0,
                (5, 7),
            ),
        ],
        main_spacing: 0.11 * r,
        boundary: BoundaryInfluence {
            strength: 2.0,
            reach: 0.03 * r,
        },
        max_bridge: 260.0,
        centre: at(0.44, 0.52),
        tram_lines: vec![
            TramLineSpec {
                name: "T1".into(),
                colour: [220, 40, 60],
                waypoints: vec![at(0.10, 0.20), at(0.44, 0.52), at(0.80, 0.60)],
            },
            TramLineSpec {
                name: "T2".into(),
                colour: [30, 110, 200],
                waypoints: vec![at(0.45, 0.08), at(0.44, 0.52), at(0.30, 0.90)],
            },
        ],
        stop_spacing: 450.0,
        boat_gap: 700.0,
        landmarks: vec![
            LandmarkSpec {
                name: "market hall".into(),
                anchor: at(0.42, 0.48),
            },
            LandmarkSpec {
                name: "harbour house".into(),
                anchor: at(0.84, 0.70),
            },
        ],
        park_reach: 300.0,
        park_min_area: 10_000.0,
        park_share_cap: 0.22,
    }
}
