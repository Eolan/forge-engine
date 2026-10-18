use glam::DVec3;

use super::*;

/// The lab's fetch mission's shape: climb, find the parcel the climb spawns, come back within
/// half a minute.
const FETCH: &str = r#"
Mission(
    id: "test:fetch",
    version: 2,
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
    steps: (id: "main", seq: [
        (id: "climb", objective: "climb", until: Inside("top")),
        (do: [Spawn("parcel")]),
        (id: "find", objective: "find", until: Holds("parcel")),
        (id: "back", objective: "return", until: Inside("start"), fail_if: After(30.0)),
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

fn var(r: &Runtime, name: &str) -> i64 {
    r.vars().find(|(n, _)| *n == name).unwrap().1
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
    // What `fail_if` names is no target.
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
fn the_return_fails_on_its_timer_and_fails_the_mission() {
    let mut r = fetch();
    let tick = walk(&mut r, 0, &[(TOP, 1), (PARCEL, 1)]);
    // 30 s at 60 ticks a second after the return began.
    let tick = walk(&mut r, tick, &[(AWAY, 30 * 60 - 1)]);
    assert_eq!(r.outcome(), Outcome::Running);
    r.tick(tick, AWAY);
    assert_eq!(r.outcome(), Outcome::Failed);
    assert_eq!(status(&r, "return"), Status::Failed);
    assert!(
        r.events()
            .contains(&MissionEvent::ObjectiveFailed("return".into()))
    );
    assert_eq!(r.step("main"), Some(StepStatus::Failed));
}

#[test]
fn a_tie_goes_to_failure() {
    // The player reaches the start on the tick the timer runs out.
    let mut r = fetch();
    let tick = walk(&mut r, 0, &[(TOP, 1), (PARCEL, 1), (AWAY, 30 * 60 - 1)]);
    r.tick(tick, START);
    assert_eq!(r.outcome(), Outcome::Failed);
}

/// A mission with two objectives, places `pa` and `pb`, an item `key` and the counters
/// `alarms` and `found`.
fn mission(steps: &str, places: &str) -> Runtime {
    Runtime::new(parse(&record(steps, places)).unwrap(), 60).unwrap()
}

fn record(steps: &str, places: &str) -> String {
    format!(
        r#"Mission(id: "test:m", version: 2,
            objectives: [(id: "a", text: "A"), (id: "b", text: "B")],
            places: [{places}],
            items: [(id: "key", at: (5.0, 0.0, 0.0))],
            vars: {{"alarms": 0, "found": 0}},
            steps: {steps})"#
    )
}

const AB: &str = r#"(id: "pa", shape: Sphere(centre: (1.0, 0.0, 0.0), radius: 0.5)),
                    (id: "pb", shape: Sphere(centre: (-1.0, 0.0, 0.0), radius: 0.5))"#;
const PA: DVec3 = DVec3::new(1.0, 0.0, 0.0);
const PB: DVec3 = DVec3::new(-1.0, 0.0, 0.0);

#[test]
fn all_waits_for_every_step() {
    let mut r = mission(
        r#"(all: [(objective: "a", until: Inside("pa")), (objective: "b", until: Inside("pb"))])"#,
        AB,
    );
    r.tick(0, PA);
    assert_eq!(status(&r, "a"), Status::Completed);
    assert_eq!(r.outcome(), Outcome::Running);
    r.tick(1, PB);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn an_all_misses_its_optional_steps_when_its_required_ones_are_done() {
    let mut r = mission(
        r#"(all: [
            (objective: "a", until: Inside("pa")),
            (objective: "b", optional: true, until: Inside("pb")),
        ])"#,
        AB,
    );
    r.tick(0, PA);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert_eq!(status(&r, "b"), Status::Missed);
    assert!(
        r.events()
            .contains(&MissionEvent::ObjectiveMissed("b".into()))
    );
}

#[test]
fn an_optional_step_that_fails_does_not_fail_its_sequence() {
    let mut r = mission(
        r#"(seq: [
            (objective: "b", optional: true, until: Never, fail_if: Var("alarms", AtLeast(1))),
            (objective: "a", until: Inside("pa")),
        ])"#,
        AB,
    );
    // The sequence waits on its optional step like any other...
    r.tick(0, AWAY);
    assert_eq!(status(&r, "b"), Status::Active);
    assert_eq!(status(&r, "a"), Status::Pending);
    // ...and when it fails, goes on.
    let mut r = mission(
        r#"(seq: [
            (do: [Add("alarms", 1)]),
            (objective: "b", optional: true, until: Never, fail_if: Var("alarms", AtLeast(1))),
            (objective: "a", until: Inside("pa")),
        ])"#,
        AB,
    );
    r.tick(0, AWAY);
    assert_eq!(status(&r, "b"), Status::Failed);
    assert_eq!(status(&r, "a"), Status::Active);
    r.tick(1, PA);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn a_race_is_decided_by_the_first_step_to_end_even_failing() {
    // The second arm fails first: the race fails, the first arm is missed.
    let mut r = mission(
        r#"(race: [
            (objective: "a", until: Inside("pa")),
            (objective: "b", until: Never, fail_if: Inside("pb")),
        ])"#,
        AB,
    );
    r.tick(0, PB);
    assert_eq!(r.outcome(), Outcome::Failed);
    assert_eq!(status(&r, "a"), Status::Missed);
    assert_eq!(status(&r, "b"), Status::Failed);
    // Both end in one tick: the written order decides.
    let mut r = mission(
        r#"(race: [
            (objective: "a", until: After(0.0)),
            (objective: "b", until: Never, fail_if: After(0.0)),
        ])"#,
        AB,
    );
    r.tick(0, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert_eq!(status(&r, "b"), Status::Missed);
}

#[test]
fn all_of_remembers_each_part() {
    // Visited one after the other, never both at once.
    let mut r = mission(
        r#"(objective: "a", until: AllOf([Inside("pa"), Inside("pb")]))"#,
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
        r#"(objective: "a", until: AnyOf([Inside("pa"), Event("door")]))"#,
        AB,
    );
    r.tick(0, AWAY);
    r.notify("door");
    r.notify("an event no condition uses");
    r.tick(1, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn not_is_tested_as_it_is_now() {
    // Leave the place after being in it: the Not under the AllOf is not held.
    let mut r = mission(
        r#"(objective: "a", until: AllOf([Inside("pa"), Not(Near("pa", 2.0))]))"#,
        AB,
    );
    r.tick(0, PA);
    assert_eq!(r.outcome(), Outcome::Running);
    r.tick(1, PA + DVec3::new(1.5, 0.0, 0.0));
    assert_eq!(r.outcome(), Outcome::Running);
    r.tick(2, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn near_an_item_counts_only_once_it_lies_there() {
    let mut r = mission(
        r#"(seq: [
            (objective: "a", until: Near("key", 3.0)),
        ])"#,
        AB,
    );
    r.tick(0, DVec3::new(3.0, 0.0, 0.0));
    assert_eq!(r.outcome(), Outcome::Running);
    let mut r = mission(
        r#"(seq: [(do: [Spawn("key")]), (objective: "a", until: Near("key", 3.0))])"#,
        AB,
    );
    r.tick(0, DVec3::new(3.0, 0.0, 0.0));
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn counters_change_in_order_and_later_steps_see_them_in_the_same_tick() {
    let mut r = mission(
        r#"(seq: [
            (do: [Set("found", 2), Add("found", 3), Emit("test:counted")]),
            (objective: "a", until: Var("found", Equals(5)), then: [Add("alarms", 1)]),
            (objective: "b", until: Var("alarms", AtLeast(1))),
        ])"#,
        AB,
    );
    r.tick(0, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert_eq!(var(&r, "found"), 5);
    assert_eq!(var(&r, "alarms"), 1);
    assert!(
        r.events()
            .contains(&MissionEvent::Emitted("test:counted".into()))
    );
}

#[test]
fn end_ends_the_mission_at_once_and_cancels_the_rest() {
    let mut r = mission(
        r#"(all: [
            (objective: "a", until: Inside("pa"), then: [End(Failed)]),
            (objective: "b", until: Inside("pb"), on_cancel: [Add("alarms", 1)]),
        ])"#,
        AB,
    );
    r.tick(0, PA);
    assert_eq!(r.outcome(), Outcome::Failed);
    assert_eq!(status(&r, "b"), Status::Missed);
    // The cancelled step's own action ran.
    assert_eq!(var(&r, "alarms"), 1);
}

#[test]
fn on_fail_runs_when_a_step_fails() {
    let mut r = mission(
        r#"(seq: [
            (objective: "b", optional: true, until: Never, fail_if: Inside("pb"),
             on_fail: [Add("alarms", 3)]),
            (objective: "a", until: Var("alarms", AtLeast(3))),
        ])"#,
        AB,
    );
    r.tick(0, PB);
    assert_eq!(r.outcome(), Outcome::Succeeded);
}

#[test]
fn an_event_counts_only_after_its_wait_began() {
    let mut r = mission(
        r#"(seq: [(objective: "a", until: Inside("pa")), (objective: "b", until: Event("door"))])"#,
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
    // The branch's objective is shown beside the sequence's and missed when the sequence ends.
    let mut r = mission(
        r#"(seq: [
            (branch: (objective: "b", until: Inside("pb"))),
            (objective: "a", until: Inside("pa")),
        ])"#,
        AB,
    );
    r.tick(0, AWAY);
    assert_eq!(status(&r, "a"), Status::Active);
    assert_eq!(status(&r, "b"), Status::Active);
    r.tick(1, PA);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert_eq!(status(&r, "b"), Status::Missed);
}

#[test]
fn a_branch_in_the_background_still_completes() {
    let mut r = mission(
        r#"(seq: [
            (branch: (objective: "b", until: Inside("pb"))),
            (objective: "a", until: Inside("pa")),
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
        r#"(seq: [(do: [Spawn("key")]), (until: After(0.0)), (objective: "a", until: After(0.0))])"#,
        AB,
    );
    r.tick(0, AWAY);
    assert_eq!(r.outcome(), Outcome::Succeeded);
    assert_eq!(r.items().next().unwrap().1, ItemState::Present);
}

#[test]
fn steps_are_named_by_their_id_or_their_place() {
    let m = parse(&record(
        r#"(id: "main", seq: [(until: Inside("pa")), (id: "two", all: [(until: Inside("pb"))])])"#,
        AB,
    ))
    .unwrap();
    assert_eq!(step_ids(&m.steps), ["main", "main/0", "two", "two/0"]);
    let m = parse(&record(r#"(seq: [(until: Inside("pa"))])"#, AB)).unwrap();
    assert_eq!(step_ids(&m.steps), ["root", "root/0"]);
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
    assert_eq!(
        stage.ended.keys().collect::<Vec<_>>(),
        ["climb", "find", "main/1"]
    );
    assert_eq!(stage.picked, ["parcel"]);
    // The record edited (hot reload): a new first step, the return's step renamed and its
    // timer changed.
    let edited = FETCH
        .replace(
            r#"(id: "climb", text: "Climb the stairs"),"#,
            r#"(id: "look", text: "Look around"), (id: "climb", text: "Climb the stairs"),"#,
        )
        .replace(
            r#"steps: (id: "main", seq: ["#,
            r#"steps: (id: "main", seq: [(id: "look", objective: "look", until: After(1.0)),"#,
        )
        .replace(
            r#"(do: [Spawn("parcel")])"#,
            r#"(id: "spawn", do: [Spawn("parcel")])"#,
        )
        .replace(r#"id: "back""#, r#"id: "home""#)
        .replace("After(30.0)", "After(45.0)")
        .replace(
            "    steps:",
            "    renamed: {\"back\": \"home\", \"main/1\": \"spawn\"},\n    steps:",
        );
    r.reload(parse(&edited).unwrap()).unwrap();
    r.tick(100, AWAY);
    // The new step runs; the climb, the spawn and the find stay done, the parcel picked.
    assert_eq!(status(&r, "look"), Status::Active);
    assert_eq!(status(&r, "climb"), Status::Completed);
    walk(&mut r, 101, &[(AWAY, 60)]);
    assert_eq!(status(&r, "find"), Status::Completed);
    assert_eq!(status(&r, "return"), Status::Active);
    assert_eq!(r.step("spawn"), Some(StepStatus::Succeeded));
    assert!(
        r.items()
            .all(|(_, s)| matches!(s, ItemState::Picked { .. }))
    );
}

#[test]
fn a_stage_reload_runs_no_ended_step_again() {
    // A counter added to by a step before the one in progress: reloaded, it is not added again.
    let text = record(
        r#"(id: "main", seq: [
            (id: "count", do: [Add("found", 1)]),
            (id: "wait", objective: "a", until: Inside("pa")),
        ])"#,
        AB,
    );
    let mut r = Runtime::new(parse(&text).unwrap(), 60).unwrap();
    r.tick(0, AWAY);
    assert_eq!(var(&r, "found"), 1);
    r.reload(parse(&text).unwrap()).unwrap();
    r.tick(1, AWAY);
    assert_eq!(var(&r, "found"), 1);
    assert_eq!(status(&r, "a"), Status::Active);
    // A branch whose step was running in the background starts it again.
    let text = record(
        r#"(id: "main", seq: [
            (id: "bg", branch: (id: "side", objective: "b", until: Inside("pb"))),
            (id: "wait", objective: "a", until: Inside("pa")),
        ])"#,
        AB,
    );
    let mut r = Runtime::new(parse(&text).unwrap(), 60).unwrap();
    r.tick(0, AWAY);
    r.reload(parse(&text).unwrap()).unwrap();
    r.tick(1, PB);
    assert_eq!(status(&r, "b"), Status::Completed);
    assert_eq!(status(&r, "a"), Status::Active);
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
    let invalid = |r: Result<Mission, Error>| matches!(r, Err(Error::Invalid(_)));
    assert!(invalid(bad("test:fetch", "fetch")));
    assert!(invalid(bad("version: 2", "version: 1")));
    assert!(invalid(bad(r#"Inside("top")"#, r#"Inside("roof")"#)));
    assert!(invalid(bad(r#"Spawn("parcel")"#, r#"Spawn("box")"#)));
    assert!(invalid(bad(
        r#"(id: "find", text"#,
        r#"(id: "climb", text"#
    )));
    assert!(invalid(bad("radius: 1.0", "radius: -1.0")));
    assert!(invalid(bad("After(30.0)", "After(-1.0)")));
    assert!(matches!(
        bad("version: 2,", "version: 2, colour: 3,"),
        Err(Error::Parse(_))
    ));
    // An objective named by two steps.
    assert!(invalid(bad(
        r#"objective: "find""#,
        r#"objective: "climb""#
    )));
    // Two steps with one id, a step of two kinds, a step of none.
    assert!(invalid(bad(r#"id: "find""#, r#"id: "climb""#)));
    assert!(invalid(bad(
        r#"until: Holds("parcel")"#,
        r#"until: Holds("parcel"), seq: []"#
    )));
    assert!(invalid(bad(r#"until: Holds("parcel")"#, "")));
    // A counter not declared; a rename onto no step.
    assert!(invalid(bad(r#"Spawn("parcel")"#, r#"Add("tries", 1)"#)));
    assert!(invalid(bad(
        "    steps:",
        "    renamed: {\"old\": \"nowhere\"},\n    steps:"
    )));
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
