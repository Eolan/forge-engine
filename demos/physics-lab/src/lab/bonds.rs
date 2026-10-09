//! Joints that break: the wall's mortar (#142) and the bridge's (#147). Bodies laid unturned are
//! held to their neighbours (or to the world) by fixed joints, and after each step the joints that
//! carried more than they hold, or whose bodies moved out of where they were laid by more than they
//! give, are broken. Jolt saves whether each joint holds with the world, so what broke restores,
//! replays and goes through `--net` like the rest.

use forge_physics::{BodyId, JointId, JointLoad, World};
use glam::DVec3;

/// What a joint holds before it breaks: a force, N, and a torque, N·m; and how far it gives: its
/// bodies moved apart by `stretch` metres, or turned by `twist` (the sine of half the angle).
#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub force: f32,
    pub torque: f32,
    pub stretch: f64,
    pub twist: f32,
}

/// A joint: the bodies it holds (indices into the bonds' bodies, `None` for the world) and where
/// the second was laid from the first.
#[derive(Clone, Copy, Debug)]
struct Bond {
    joint: JointId,
    a: Option<u32>,
    b: u32,
    offset: DVec3,
}

/// Bodies and the joints between them that break.
#[derive(Clone, Debug)]
pub(super) struct Bonds {
    pub joints: Vec<JointId>,
    bonds: Vec<Bond>,
    bodies: Vec<BodyId>,
    laid: Vec<DVec3>,
    limits: Limits,
}

impl Bonds {
    /// No joints yet between `bodies`, laid unturned at `laid`.
    pub(super) fn new(bodies: Vec<BodyId>, laid: Vec<DVec3>, limits: Limits) -> Self {
        Self {
            joints: Vec::new(),
            bonds: Vec::new(),
            bodies,
            laid,
            limits,
        }
    }

    /// Joins body `b` to body `a` (or to the world) where they lie, with the solver's velocity and
    /// position steps `steps` for the joint.
    pub(super) fn join(&mut self, world: &mut World, a: Option<u32>, b: u32, steps: (u32, u32)) {
        let joint = world.join_fixed(
            a.map(|a| self.bodies[a as usize]),
            self.bodies[b as usize],
            steps,
        );
        let offset = self.laid[b as usize] - a.map_or(DVec3::ZERO, |a| self.laid[a as usize]);
        self.joints.push(joint);
        self.bonds.push(Bond {
            joint,
            a,
            b,
            offset,
        });
    }

    /// After a step of `dt`: breaks the joints that carried more than they hold, or that their
    /// bodies pulled or turned out of where they were laid by more than they give (a solver's
    /// joints yield a little under a blow and pass on less of it than rigid ones would), in the
    /// joints' order (the same on every machine). How many broke.
    pub(super) fn crack(&self, world: &mut World, dt: f32) -> usize {
        let mut holding = Vec::new();
        world.holding(&self.joints, &mut holding);
        let mut loads: Vec<JointLoad> = Vec::new();
        world.joint_loads(&self.joints, &mut loads);
        let mut at = Vec::new();
        world.transforms(&self.bodies, &mut at);
        let limits = self.limits;
        let (force, torque) = (limits.force * dt, limits.torque * dt);
        let strained = |bond: &Bond| {
            let b = at[bond.b as usize];
            let (offset, turn) = match bond.a {
                Some(a) => {
                    let a = at[a as usize];
                    let back = a.rotation.inverse();
                    (
                        back.as_dquat() * (b.position - a.position),
                        back * b.rotation,
                    )
                }
                None => (b.position, b.rotation),
            };
            offset.distance(bond.offset) > limits.stretch || turn.xyz().length() > limits.twist
        };
        let broken: Vec<JointId> = self
            .bonds
            .iter()
            .zip(holding.iter().zip(&loads))
            .filter(|(bond, (h, l))| {
                **h && (l.position > force || l.rotation > torque || strained(bond))
            })
            .map(|(bond, _)| bond.joint)
            .collect();
        world.set_holding(&broken, false);
        broken.len()
    }

    /// The joints still holding.
    pub(super) fn holding(&self, world: &World) -> usize {
        let mut holding = Vec::new();
        world.holding(&self.joints, &mut holding);
        holding.iter().filter(|&&h| h).count()
    }
}
