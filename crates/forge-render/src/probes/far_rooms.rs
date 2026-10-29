//! The far rooms' daylight (#356): a room beyond the room grids' reach (D-078) takes a share of
//! the open sky's light, what its windows let in. A fixed share (`far_room_sky`) could not follow
//! the hour: against grids out to 150 m the village's far rooms read 0.19 EV too bright at noon,
//! when the sky is brightest and the high sun reaches no room, and 0.15 EV too dark at 0.75, when
//! the low sun shines in through the windows facing it (`reports/2026-10-28-356`).
//!
//! Each room's share is its windows' daylight over its surfaces, each frame on the GPU (`gi/far
//! rooms`, `probe_update.slang`): per window, its area times what the sky and the ground send a
//! wall facing its way and what the sun sends its glass; over the room's walls, floor and
//! ceiling; over what the sky sends a roof, the open sky's light the share multiplies; times
//! [`super::ProbeParams::far_room_daylight`], what bounces round the room. The rooms and their
//! windows come from the region sets (D-068): a room's windows are the recesses of its region,
//! each facing away from its room.

use glam::Vec2;

use super::{RegionKind, RegionVolume};

/// A window of a far room: the way it faces (out of the room, level, unit) and its glass's area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FarWindow {
    pub normal: Vec2,
    pub area: f32,
}

/// A room as the far rooms' daylight reads it: its region, its walls', floor's and ceiling's
/// area (m²) and its windows.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FarRoom {
    pub region: u32,
    pub surface: f32,
    pub windows: Vec<FarWindow>,
}

/// The rooms of a set's `volumes` with a window, in the order of their regions' first volume. A
/// room of several volumes sums their surfaces; a recess counts for its own region's room.
pub(super) fn far_rooms(volumes: &[RegionVolume]) -> Vec<FarRoom> {
    // Each room region: its surfaces and the mean of its volumes' corners.
    let mut rooms: Vec<(u32, f32, Vec2, f32)> = Vec::new();
    for v in volumes.iter().filter(|v| v.kind == RegionKind::Room) {
        let c = v.corners;
        let edges = [c[1] - c[0], c[2] - c[1], c[3] - c[2], c[0] - c[3]];
        let perimeter: f32 = edges.iter().map(|e| e.length()).sum();
        let floor_area = 0.5 * ((c[2] - c[0]).perp_dot(c[3] - c[1])).abs();
        let surface = 2.0 * floor_area + perimeter * (v.ceiling - v.floor).max(0.0);
        let centre = (c[0] + c[1] + c[2] + c[3]) * 0.25;
        match rooms.iter_mut().find(|r| r.0 == v.region) {
            Some(r) => {
                r.1 += surface;
                r.2 += centre;
                r.3 += 1.0;
            }
            None => rooms.push((v.region, surface, centre, 1.0)),
        }
    }
    let mut out: Vec<FarRoom> = rooms
        .iter()
        .map(|&(region, surface, _, _)| FarRoom {
            region,
            surface,
            windows: Vec::new(),
        })
        .collect();
    for v in volumes.iter().filter(|v| v.kind == RegionKind::Recess) {
        let Some(i) = rooms.iter().position(|r| r.0 == v.region) else {
            continue;
        };
        let room_centre = rooms[i].2 / rooms[i].3;
        // A reveal is a thin box: its glass across the short side, its width the long one.
        let c = v.corners;
        let (a, b) = (c[1] - c[0], c[3] - c[0]);
        let (along, across) = if a.length() >= b.length() {
            (a, b)
        } else {
            (b, a)
        };
        let height = (v.ceiling - v.floor).max(0.0);
        let area = along.length() * height;
        let Some(axis) = across.try_normalize() else {
            continue;
        };
        if area <= 0.0 {
            continue;
        }
        let centre = (c[0] + c[1] + c[2] + c[3]) * 0.25;
        let normal = if (centre - room_centre).dot(axis) >= 0.0 {
            axis
        } else {
            -axis
        };
        out[i].windows.push(FarWindow { normal, area });
    }
    out.retain(|r| !r.windows.is_empty() && r.surface > 0.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(x0: f32, z0: f32, x1: f32, z1: f32) -> [Vec2; 4] {
        [
            Vec2::new(x0, z0),
            Vec2::new(x1, z0),
            Vec2::new(x1, z1),
            Vec2::new(x0, z1),
        ]
    }

    /// Two rooms 3.9 m by 4 m and 3 m high, a window in the first's outer wall at z = 0 (its
    /// reveal 1 m wide, its glass from 0.9 to 2.1 m), and a room of two volumes with one window
    /// each way: each window faces out of its room, with its glass's area; the windowless room
    /// is left out.
    #[test]
    fn a_rooms_windows_face_out_of_it_with_their_glass() {
        let volume = |corners, floor, ceiling, region, kind| RegionVolume {
            corners,
            floor,
            ceiling,
            region,
            other: region,
            kind,
        };
        let v = vec![
            volume(
                outline(-0.3, -0.3, 8.3, 4.3),
                -0.5,
                3.3,
                0,
                RegionKind::Solid,
            ),
            volume(outline(0.0, 0.0, 3.9, 4.0), 0.0, 3.0, 1, RegionKind::Room),
            volume(outline(4.1, 0.0, 8.0, 4.0), 0.0, 3.0, 2, RegionKind::Room),
            volume(
                outline(1.0, -0.3, 2.0, 0.01),
                0.9,
                2.1,
                1,
                RegionKind::Recess,
            ),
            // Room 3, two volumes side by side along x, a window out of each end.
            volume(outline(10.0, 0.0, 12.0, 3.0), 0.0, 3.0, 3, RegionKind::Room),
            volume(outline(12.0, 0.0, 14.0, 3.0), 0.0, 3.0, 3, RegionKind::Room),
            // Given the other way round, glass side last.
            volume(
                [
                    Vec2::new(14.3, 2.0),
                    Vec2::new(13.99, 2.0),
                    Vec2::new(13.99, 1.0),
                    Vec2::new(14.3, 1.0),
                ],
                1.0,
                2.0,
                3,
                RegionKind::Recess,
            ),
            volume(
                outline(9.7, 1.0, 10.01, 2.5),
                1.0,
                2.0,
                3,
                RegionKind::Recess,
            ),
        ];
        let rooms = far_rooms(&v);
        assert_eq!(rooms.len(), 2, "{rooms:?}");
        let first = &rooms[0];
        assert_eq!(first.region, 1);
        assert!(
            (first.surface - (2.0 * 15.6 + 15.8 * 3.0)).abs() < 1e-3,
            "{}",
            first.surface
        );
        assert_eq!(first.windows.len(), 1);
        assert!((first.windows[0].normal - Vec2::new(0.0, -1.0)).length() < 1e-6);
        assert!((first.windows[0].area - 1.2).abs() < 1e-5);
        let third = &rooms[1];
        assert_eq!(third.region, 3);
        assert!((third.surface - 2.0 * (2.0 * 6.0 + 10.0 * 3.0)).abs() < 1e-3);
        let normals: Vec<Vec2> = third.windows.iter().map(|w| w.normal).collect();
        assert!((normals[0] - Vec2::X).length() < 1e-6, "{normals:?}");
        assert!((normals[1] + Vec2::X).length() < 1e-6, "{normals:?}");
        assert!((third.windows[1].area - 1.5).abs() < 1e-5);
    }
}
