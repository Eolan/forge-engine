use glam::DVec3;

use super::*;

/// The lab's fetch mission's shape: climb, find the parcel the climb spawns, come back within
/// half a minute.
const FETCH: &str = r#"
Mission(
    id: "test:fetch",
    version: 1,
    objectives: [
        (id: "climb", text: "Climb the stairs"),
        (id: "find", text: "Find the parcel"),
        (id: "return", text: "Back to the start within 30 s"),
    ],
    places: [
        (id: "top", shape: Box(min: (6.0, 1.0, -1.5), max: (6.6, 3.0, 1.5))),
        (id: "start", shape: Sphere(centre: (0.0, 0.0, 1.0), radius: 1.0)),
    ],
    items: [(id: "parcel", at: (-4.0, 0.0, -1.5))],
    steps: Sequence([
        Objective("climb", Wait(Entered("top"))),
        Spawn("parcel"),
        Objective("find", Wait(PickedUp("parcel"))),
        Race([
            Objective("return", Wait(Entered("start"))),
            Sequence([Wait(After(30.0)), Fail]),
        ]),
    ]),
)
"#;

const TOP: DVec3 = DVec3::new(6.3, 1.2, 0.0);
const PARCEL: DVec3 = DVec3::new(-4.0, 0.0, -1.5);
const START: DVec3 = DVec3::new(0.0, 0.0, 1.0);
const AWAY: DVec3 = DVec3::new(20.0, 0.0, 20.0);

fn fetch() -> Runtime {
    Runtime::new(parse(FETCH).unwrap(), 60).unwrap()
}

fn status(r: &Runtime, id: &str) -> Status {
    r.objectives().find(|(o, _)| o.id == id).unwrap().1
}

/// The player's feet at each tick of a walk: `ticks` at each place in turn.
fn walk(r: &mut Runtime, from: u64, legs: &[(DVec3, u64)]) -> u64 {
    let mut tick = from;
    for &(at, ticks) in legs {
        for _ in 0..ticks {
            r.tick(tick, at);
            tick += 1;
        }
    }
    tick
}

#[test]
fn the_fetch_mission_runs_to_its_end() {
    let mut r = fetch();
    r.tick(0, AWAY);
    assert_eq!(r.events(), [MissionEvent::ObjectiveStarted("climb".into())]);
    assert_eq!(status(&r, "find"), Status::Pending);
    r.tick(1, TOP);
    // Climbed, the parcel spawned and the next objective shown in the same tick.
    assert_eq!(
        r.events(),
        [
            MissionEvent::ObjectiveCompleted("climb".into()),
            MissionEvent::ItemSpawned("parcel".into()),
            MissionEvent::ObjectiveStarted("find".into()),
        ]
    );
    assert_eq!(r.targets(), [PARCEL]);
    r.tick(2, PARCEL + DVec3::new(0.9, 0.0, 0.0));
    assert!(
        r.events()
            .contains(&MissionEvent::ItemPicked("parcel".into()))
    );
    assert_eq!(status(&r, "find"), Status::Completed);
    assert_eq!(status(&r, "return"), Status::Active);
    assert_eq!(r.targets(), [START]);
    r.tick(3, START);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert!(r.events().contains(&MissionEvent::Succeeded));
    // Nothing moves after the end.
    let digest = r.digest();
    r.tick(4, TOP);
    assert_eq!(r.digest(), digest);
}

#[test]
fn the_race_is_lost_to_its_timer() {
    let mut r = fetch();
    let tick = walk(&mut r, 0, &[(TOP, 1), (PARCEL, 1)]);
    // 30 s at 60 ticks a second after the race began, the mission fails.
    let tick = walk(&mut r, tick, &[(AWAY, 30 * 60 - 1)]);
    assert_eq!(r.outcome(), Outcome::Running);
    r.tick(tick, AWAY);
    assert_eq!(r.outcome(), Outcome::Failed);
    assert_eq!(status(&r, "return"), Status::Cancelled);
    assert!(
        r.events()
            .contains(&MissionEvent::ObjectiveCancelled("return".into()))
    );
}

fn mission(steps: &str, places: &str) -> Runtime {
    let text = format!(
        r#"Mission(id: "test:m", version: 1,
            objectives: [(id: "a", text: "A"), (id: "b", text: "B")],
            places: [{places}],
            items: [(id: "key", at: (5.0, 0.0, 0.0))],
            steps: {steps})"#
    );
    Runtime::new(parse(&text).unwrap(), 60).unwrap()
}

const AB: &str = r#"(id: "pa", shape: Sphere(centre: (1.0, 0.0, 0.0), radius: 0.5)),
                    (id: "pb", shape: Sphere(centre: (-1.0, 0.0, 0.0), radius: 0.5))"#;
const PA: DVec3 = DVec3::new(1.0, 0.0, 0.0);
const PB: DVec3 = DVec3::new(-1.0, 0.0, 0.0);

#[test]
fn all_waits_for_every_step() {
    let mut r = mission(
        r#"All([Objective("a", Wait(Entered("pa"))), Objective("b", Wait(Entered("pb")))])"#,
        AB,
    );
    r.tick(0, PA);
    assert_eq!(status(&r, "a"), Status::Completed);
    assert_eq!(r.outcome(), Outcome::Running);
    r.tick(1, PB);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn all_of_remembers_each_part() {
    // Visited one after the other, never both at once.
    let mut r = mission(
        r#"Objective("a", Wait(AllOf([Entered("pa"), Entered("pb")])))"#,
        AB,
    );
    r.tick(0, PA);
    assert_eq!(r.targets(), [PB]);
    r.tick(1, AWAY);
    r.tick(2, PB);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn any_of_takes_the_first() {
    let mut r = mission(
        r#"Objective("a", Wait(AnyOf([Entered("pa"), Event("door")])))"#,
        AB,
    );
    r.tick(0, AWAY);
    r.notify("door");
    r.notify("an event no trigger uses");
    r.tick(1, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn an_event_counts_only_after_its_wait_began() {
    let mut r = mission(
        r#"Sequence([Objective("a", Wait(Entered("pa"))), Objective("b", Wait(Event("door")))])"#,
        AB,
    );
    r.notify("door");
    r.tick(0, AWAY);
    r.tick(1, PA);
    assert_eq!(status(&r, "b"), Status::Active);
    r.tick(2, AWAY);
    assert_eq!(status(&r, "b"), Status::Active);
    r.notify("door");
    r.tick(3, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn a_branch_runs_until_its_scope_ends() {
    // The branch's objective is shown beside the sequence's and cancelled when the sequence
    // ends.
    let mut r = mission(
        r#"Sequence([
            Branch(Objective("b", Wait(Entered("pb")))),
            Objective("a", Wait(Entered("pa"))),
        ])"#,
        AB,
    );
    r.tick(0, AWAY);
    assert_eq!(status(&r, "a"), Status::Active);
    assert_eq!(status(&r, "b"), Status::Active);
    r.tick(1, PA);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert_eq!(status(&r, "b"), Status::Cancelled);
}

#[test]
fn a_branch_in_the_background_still_completes() {
    let mut r = mission(
        r#"Sequence([
            Branch(Objective("b", Wait(Entered("pb")))),
            Objective("a", Wait(Entered("pa"))),
        ])"#,
        AB,
    );
    r.tick(0, PB);
    assert_eq!(status(&r, "b"), Status::Completed);
    assert_eq!(status(&r, "a"), Status::Active);
}

#[test]
fn instant_steps_chain_in_one_tick() {
    let mut r = mission(
        r#"Sequence([Spawn("key"), Wait(After(0.0)), Objective("a", Wait(After(0.0)))])"#,
        AB,
    );
    r.tick(0, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert_eq!(r.items().next().unwrap().1, ItemState::Present);
}

#[test]
fn a_snapshot_restores_to_the_bit() {
    // Two runs from one snapshot, mid-mission, tick for tick the same digests.
    let legs = [(AWAY, 5), (TOP, 1), (AWAY, 40), (PARCEL, 1), (AWAY, 30)];
    let mut whole = fetch();
    let mut tick = walk(&mut whole, 0, &legs[..2]);
    let mut saved = Vec::new();
    whole.save(&mut saved);
    let mut digests = Vec::new();
    for &(at, ticks) in &legs[2..] {
        for _ in 0..ticks {
            whole.tick(tick, at);
            digests.push(whole.digest());
            tick += 1;
        }
    }
    let mut resumed = fetch();
    assert_eq!(resumed.restore(&saved).unwrap(), saved.len());
    let mut tick = 6;
    let mut again = Vec::new();
    for &(at, ticks) in &legs[2..] {
        for _ in 0..ticks {
            resumed.tick(tick, at);
            again.push(resumed.digest());
            tick += 1;
        }
    }
    assert_eq!(digests, again);
    assert_eq!(resumed.stage(), whole.stage());
}

#[test]
fn a_snapshot_of_another_record_is_refused() {
    let mut saved = Vec::new();
    fetch().save(&mut saved);
    let mut other = Runtime::new(parse(&FETCH.replace("30 s", "20 s")).unwrap(), 60).unwrap();
    assert!(matches!(other.restore(&saved), Err(Error::State(_))));
}

#[test]
fn a_stage_loads_into_a_changed_record() {
    let mut r = fetch();
    walk(&mut r, 0, &[(TOP, 1), (PARCEL, 1)]);
    let stage = r.stage();
    assert_eq!(stage.completed, ["climb", "find"]);
    assert_eq!(stage.picked, ["parcel"]);
    // The record edited (hot reload): a new first objective, the return's text changed.
    let edited = FETCH
        .replace(
            r#"(id: "climb", text: "Climb the stairs"),"#,
            r#"(id: "look", text: "Look around"), (id: "climb", text: "Climb the stairs"),"#,
        )
        .replace(
            r#"steps: Sequence(["#,
            r#"steps: Sequence([Objective("look", Wait(After(1.0))),"#,
        )
        .replace("30 s", "45 s");
    r.reload(parse(&edited).unwrap()).unwrap();
    r.tick(100, AWAY);
    // The new objective runs; the climb and the find are done, the parcel stays picked.
    assert_eq!(status(&r, "look"), Status::Active);
    walk(&mut r, 101, &[(AWAY, 60)]);
    assert_eq!(status(&r, "climb"), Status::Completed);
    assert_eq!(status(&r, "find"), Status::Completed);
    assert_eq!(status(&r, "return"), Status::Active);
    assert!(
        r.items()
            .all(|(_, s)| matches!(s, ItemState::Picked { .. }))
    );
}

#[test]
fn a_stage_of_another_mission_is_refused() {
    let stage = Stage {
        mission: "test:other".into(),
        ..Stage::default()
    };
    assert!(Runtime::with_stage(parse(FETCH).unwrap(), 60, &stage).is_err());
}

#[test]
fn two_runs_agree_to_the_bit() {
    let legs = [(AWAY, 3), (TOP, 2), (PARCEL, 2), (START, 1)];
    let digests = |r: &mut Runtime| {
        let mut out = Vec::new();
        let mut tick = 0;
        for &(at, ticks) in &legs {
            for _ in 0..ticks {
                r.tick(tick, at);
                out.push(r.digest());
                tick += 1;
            }
        }
        out
    };
    assert_eq!(digests(&mut fetch()), digests(&mut fetch()));
}

#[test]
fn bad_records_are_refused() {
    let bad = |from: &str, to: &str| parse(&FETCH.replace(from, to));
    assert!(matches!(bad("test:fetch", "fetch"), Err(Error::Invalid(_))));
    assert!(matches!(
        bad("version: 1", "version: 2"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        bad(r#"Entered("top")"#, r#"Entered("roof")"#),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        bad(r#"Spawn("parcel")"#, r#"Spawn("box")"#),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        bad(r#"(id: "find", text"#, r#"(id: "climb", text"#),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        bad("radius: 1.0", "radius: -1.0"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        bad("After(30.0)", "After(-1.0)"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        bad("version: 1,", "version: 1, colour: 3,"),
        Err(Error::Parse(_))
    ));
    // An objective named by two steps.
    assert!(matches!(
        bad(r#"Objective("find", Wait"#, r#"Objective("climb", Wait"#),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn shapes_hold_their_edges() {
    let b = Shape::Box {
        min: [0.0, 0.0, 0.0],
        max: [1.0, 1.0, 1.0],
    };
    assert!(b.contains([1.0, 0.0, 0.5]));
    assert!(!b.contains([1.0 + 1e-9, 0.5, 0.5]));
    assert_eq!(b.centre(), [0.5, 0.5, 0.5]);
    let s = Shape::Sphere {
        centre: [0.0, 0.0, 0.0],
        radius: 2.0,
    };
    assert!(s.contains([0.0, 2.0, 0.0]));
    assert!(!s.contains([1.5, 1.5, 0.0]));
}
