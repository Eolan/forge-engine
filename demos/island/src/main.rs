//! `island` — Phase 2's demo (issue #96, `docs/demos/island.md`): a 16 km island generated
//! from a seed (`--island`, 7 by default), its ground cooked into cluster DAGs in tiles and
//! drawn at 2 m, the sea, the rivers and the lakes on the GPU. It shares `city-blocks`'
//! renderer and flags (`city_blocks::main_island`); `--shot` frames one of its golden shots
//! and `--tour` flies over it.

#![forbid(unsafe_code)]

fn main() -> anyhow::Result<()> {
    city_blocks::main_island()
}
