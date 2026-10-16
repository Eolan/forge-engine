//! A species as data: its size, its speeds, its group and how close it lets people, vehicles,
//! dogs and cats come. The behaviour (its day, where it rests and feeds) is its [`Behaviour`]'s,
//! in [`crate::sim`]. The village's eight live in `data/fauna.toml`; [`village`] reads them.

use serde::Deserialize;

/// The routine and the movement a species follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Behaviour {
    /// Flocks feeding on the ground in gardens and squares, resting in hedges and trees, roosting
    /// in a colony's tree.
    Sparrow,
    /// Aloft all day, screaming parties along the streets at dusk, nests under the roofs.
    Swift,
    /// Flocks on a square, circling over the roofs, roosting on ledges.
    Pigeon,
    /// Soaring over the harbour, standing on the quay and the roofs, floating on the sea.
    Gull,
    /// A home and its garden, a territory patrolled at dawn and dusk, walls to rest and escape on.
    Cat,
    /// Walks with its owner along the streets, morning and evening.
    Dog,
    /// A herd out of its fold onto the hillside, browsing and resting in the shade.
    Goat,
    /// Schools along the quay's wall.
    Fish,
}

impl Behaviour {
    /// Whether it flies.
    pub fn flies(self) -> bool {
        matches!(
            self,
            Behaviour::Sparrow | Behaviour::Swift | Behaviour::Pigeon | Behaviour::Gull
        )
    }
}

/// A species.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Species {
    /// Its key, `house-sparrow`.
    pub id: String,
    /// Its common name.
    pub name: String,
    /// Its binomial.
    pub latin: String,
    /// Its routine and movement.
    pub behaviour: Behaviour,
    /// Its body's length, metres.
    pub body: f32,
    /// Its groups' sizes, fewest and most.
    pub group: [u32; 2],
    /// Its pace on the ground, m/s.
    pub walk: f32,
    /// Its cruising speed: flying, swimming, trotting, m/s.
    pub cruise: f32,
    /// Its speed in flight from a threat, m/s.
    pub flee_speed: f32,
    /// Its fastest turn, degrees a second.
    pub turn: f32,
    /// Its fastest climb, m/s.
    pub climb: f32,
    /// How many of its group's nearest it follows (a topological neighbourhood); 0 alone.
    pub neighbours: u32,
    /// How close it lets a groupmate come before it turns away, metres.
    pub separation: f32,
    /// How close it lets a person on foot come, metres; 0 never flees.
    pub flee_person: f32,
    /// How close it lets a vehicle come on a street limited to 50 km/h, metres, scaled by the
    /// street's limit.
    pub flee_vehicle: f32,
    /// How close it lets a dog come, metres.
    pub flee_dog: f32,
    /// How close it lets a cat come, metres.
    pub flee_cat: f32,
    /// How far from its home it goes, metres.
    pub range: f32,
    /// Its leash, metres; 0 none.
    #[serde(default)]
    pub leash: f32,
    /// How many of its flight distances away it starts to walk away from a threat on foot
    /// before it flies; 0 never.
    #[serde(default)]
    pub walk_off: f32,
}

#[derive(Deserialize)]
struct File {
    species: Vec<Species>,
}

/// Parses a species file.
pub fn parse(text: &str) -> Result<Vec<Species>, toml::de::Error> {
    toml::from_str::<File>(text).map(|f| f.species)
}

/// The village's eight species, from `data/fauna.toml`.
pub fn village() -> Vec<Species> {
    parse(include_str!("../data/fauna.toml")).expect("the built-in species file parses")
}

/// The index of the species with this behaviour, the first one.
pub fn index_of(all: &[Species], b: Behaviour) -> Option<usize> {
    all.iter().position(|s| s.behaviour == b)
}
