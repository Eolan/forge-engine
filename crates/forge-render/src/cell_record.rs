//! A streamed cell's record (#338 step 3, piece 6): what a cell adds beside its scene edit (its
//! rooms as the probes' regions, its clear panes, its lamps), added and removed as one.
//!
//! A cell comes in as a [`crate::SceneEditor`]'s edit (its meshes and instances) and the sets
//! here; the frames take the edit ([`crate::MeshletScene::apply`]) and [`CellSystems::add`] the
//! sets in the same update, so that all of it lands on the frame the edit is published: the
//! next frame begun publishes the edit, and the sets switch on with it. It leaves the same way,
//! the editor's removal applied and [`CellSystems::remove`] called in one update.
//!
//! The lamps go first: the panes name the cell's own lamps, which become the table's ids. A set
//! that fails takes those already added out again, so a cell is in whole or not at all. A street
//! lamp that comes or goes changes the far lamps' grid, which the caller makes again
//! ([`crate::lamps::LampTable::take_streets_changed`], [`crate::MeshletScene::set_far_lamps`]).

use std::sync::Arc;

use forge_gpu::{Device, GpuError, Result};

use crate::glass::{Glass, GlassPanes, NO_LAMP, PaneSetId};
use crate::lamps::{Lamp, LampSetId, LampTable};
use crate::probes::{Probes, RegionSetId, RegionVolume};

/// What a cell adds beside its scene edit.
#[derive(Clone, Debug, Default)]
pub struct CellSets {
    /// Its rooms ([`Probes::add_regions`]): their region ids are the scene's, as its lamps name
    /// them.
    pub regions: Vec<RegionVolume>,
    /// Its clear panes ([`Glass::add_panes`]), their lamps indices into `lamps`.
    pub panes: GlassPanes,
    /// Its lamps ([`LampTable::add`]).
    pub lamps: Vec<Lamp>,
    /// The key its panes' look is drawn from: the same each time the cell comes back.
    pub key: u32,
}

/// A cell's sets in the frame's systems, to take it out with ([`CellSystems::remove`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellRecord {
    /// Its rooms' set, when it has rooms and the scene probes.
    pub regions: Option<RegionSetId>,
    /// Its panes' set, when it has panes and the scene has glass.
    pub panes: Option<PaneSetId>,
    /// Its lamps' set, when it has lamps.
    pub lamps: Option<LampSetId>,
}

/// The frame's systems a cell's sets go to: those the scene has.
pub struct CellSystems<'a> {
    /// The probes, which take the cell's rooms.
    pub probes: Option<&'a mut Probes>,
    /// The clear glass, which takes its panes.
    pub glass: Option<&'a mut Glass>,
    /// The frames' lamps by stable ids.
    pub lamps: &'a mut LampTable,
}

impl CellSystems<'_> {
    /// Adds `sets` from the next frame, in the update that applies the cell's edit. Fails, with
    /// nothing added, when a system has no room for its set.
    pub fn add(&mut self, device: &Arc<Device>, sets: &CellSets) -> Result<CellRecord> {
        let mut record = CellRecord::default();
        let first = if sets.lamps.is_empty() {
            0
        } else {
            let id = self.lamps.add(&sets.lamps).ok_or_else(|| {
                GpuError::Unsupported(format!("no room for a cell's {} lamps", sets.lamps.len()))
            })?;
            record.lamps = Some(id);
            self.lamps.first(id).expect("a set just added")
        };
        if let Some(glass) = self.glass.as_deref_mut().filter(|_| !sets.panes.is_empty()) {
            let mut panes = sets.panes.clone();
            for l in &mut panes.lamp {
                if *l != NO_LAMP {
                    *l += first;
                }
            }
            match glass.add_panes(device, &panes, sets.key) {
                Ok(id) => record.panes = Some(id),
                Err(e) => {
                    self.remove(&record);
                    return Err(e);
                }
            }
        }
        if let Some(probes) = self
            .probes
            .as_deref_mut()
            .filter(|_| !sets.regions.is_empty())
        {
            match probes.add_regions(device, &sets.regions) {
                Ok(id) => record.regions = Some(id),
                Err(e) => {
                    self.remove(&record);
                    return Err(e);
                }
            }
        }
        Ok(record)
    }

    /// Takes the cell's sets out from the next frame, in the update that applies the edit removing
    /// its instances and meshes.
    pub fn remove(&mut self, record: &CellRecord) {
        if let Some(id) = record.lamps {
            self.lamps.remove(id);
        }
        if let (Some(glass), Some(id)) = (self.glass.as_deref_mut(), record.panes) {
            glass.remove_panes(id);
        }
        if let (Some(probes), Some(id)) = (self.probes.as_deref_mut(), record.regions)
            && let Err(e) = probes.remove_regions(id)
        {
            tracing::warn!("a cell's rooms left the probes: {e}");
        }
    }
}
