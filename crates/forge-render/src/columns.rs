//! Columns of a world grid in an open-addressed hash, built once on the CPU and read per pixel on
//! the GPU (`column_run` in `meshlet.slang`): each non-empty column of a grid of `cell` metres
//! (`x`, `z`; a column holds every height) keeps a list of items, and a slot of the hash names
//! the column and where its list stands. The decals (D-074, [`crate::decals`]) and the street
//! lamps lit beyond the frame's set (#284, [`crate::lamps::FarLamps`]) are found this way.

use std::collections::BTreeMap;

/// A column's slot in the hash: `x` and `z` hashed, linear probing.
pub(crate) fn slot_of(x: i32, z: i32, mask: u32) -> u32 {
    ((x as u32).wrapping_mul(73_856_093) ^ (z as u32).wrapping_mul(19_349_663)) & mask
}

/// The hash's slots (`x`, `z`, first, count; count 0 an empty slot) and the columns' lists one
/// after the other.
pub(crate) struct ColumnTables {
    pub slots: Vec<[u32; 4]>,
    pub indices: Vec<u32>,
    /// The slots' count less one (a power of two less one).
    pub mask: u32,
}

impl ColumnTables {
    /// The tables of `columns`, each (`x`, `z`) cell's list of items; at most half the slots
    /// taken, so a missed column ends its probe soon.
    pub(crate) fn new(columns: &BTreeMap<(i32, i32), Vec<u32>>) -> Self {
        let size = (columns.len() * 2).next_power_of_two().max(2);
        let mask = size as u32 - 1;
        let mut slots = vec![[0_u32; 4]; size];
        let mut indices: Vec<u32> = Vec::new();
        for (&(x, z), list) in columns.iter().filter(|(_, l)| !l.is_empty()) {
            let mut s = slot_of(x, z, mask);
            while slots[s as usize][3] != 0 {
                s = (s + 1) & mask;
            }
            slots[s as usize] = [x as u32, z as u32, indices.len() as u32, list.len() as u32];
            indices.extend_from_slice(list);
        }
        Self {
            slots,
            indices,
            mask,
        }
    }

    /// The list of column (`x`, `z`), as the shader finds it; empty where there is none.
    pub(crate) fn column(&self, x: i32, z: i32) -> &[u32] {
        let mut s = slot_of(x, z, self.mask);
        for _ in 0..=self.mask {
            let slot = self.slots[s as usize];
            if slot[3] == 0 {
                break;
            }
            if slot[0] == x as u32 && slot[1] == z as u32 {
                return &self.indices[slot[2] as usize..(slot[2] + slot[3]) as usize];
            }
            s = (s + 1) & self.mask;
        }
        &[]
    }
}

/// `data` appended to `bytes` at the next 16-byte boundary; returns where it starts.
pub(crate) fn place(bytes: &mut Vec<u8>, data: &[u8]) -> u64 {
    while !bytes.len().is_multiple_of(16) {
        bytes.push(0);
    }
    let at = bytes.len() as u64;
    bytes.extend_from_slice(data);
    at
}
