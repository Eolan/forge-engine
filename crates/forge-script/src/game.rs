//! Game scripts (#319, D-072): a script that answers the game's events with commands, outside
//! the predicted step.
//!
//! The script runs once to register what it answers on the `game` table the host gives it:
//! - **Events:** `game.on(name, handler)`. The host sends events ([`Game::send`]: an entity
//!   killed, anything it names), ticks ([`Game::tick`], the `"tick"` event, with the tick's
//!   number) and the player's actions ([`Game::input`], the `"input"` event, with the action's
//!   name and its value). Each event's handlers run in the order they were registered.
//! - **Timers:** `game.after(seconds, handler)` and `game.every(seconds, handler)` give a
//!   timer's number, and `game.cancel(number)` stops it. They count the host's ticks, never a
//!   clock (D-072: reproducible). Timers due on the same tick run in the order they were set,
//!   before that tick's handlers.
//! - **Entities:** handles ([`Entity`], userdata), never plain numbers. A script gets them from
//!   events and from `game.find(kind)`, and compares and prints them. `e:id()` gives the stable
//!   id the host knows them by.
//! - **Reading the world:** `game.get(entity, field)` and `game.find(kind)`, through the host's
//!   [`World`]. They answer only while an event or a tick runs.
//! - **Effects:** `game.command(name, ...)` queues a command stamped with the tick
//!   ([`Command`]). The host takes them ([`Game::commands`]) and its Rust systems apply them on
//!   every machine. A script never changes the world itself.
//!
//! Each event runs under the script's caps (its budget of interrupts reset at each), in Luau's
//! sandbox. The host can start the script again from a new source while the game runs
//! ([`Game::reload`]: hot reload, the old script kept when the new one fails to load), and read
//! the time each event's handlers took ([`Game::timings`]) for its profile.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use mlua::{Function, MultiValue, Table, Value};

use crate::{Data, Error, Limits, Script, to_data, to_lua};

/// An entity's handle in a script: its stable id, opaque to the script (`e:id()` reads it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity(pub u64);

impl mlua::UserData for Entity {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("id", |_, e, ()| Ok(e.0 as f64));
        methods.add_meta_method(
            mlua::MetaMethod::Eq,
            |_, e, other: mlua::UserDataRef<Entity>| Ok(e.0 == other.0),
        );
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, e, ()| {
            Ok(format!("entity {}", e.0))
        });
    }
}

/// What a script reads of the world (D-072's query API by stable ids).
pub trait World {
    /// Field `field` of entity `id` (`game.get(e, "health")`); `Nil` for none.
    fn get(&self, id: u64, field: &str) -> Data;
    /// The entities of `kind`, in the order of their ids (`game.find("zombie")`).
    fn find(&self, kind: &str) -> Vec<u64>;
}

/// A world with nothing in it.
impl World for () {
    fn get(&self, _: u64, _: &str) -> Data {
        Data::Nil
    }
    fn find(&self, _: &str) -> Vec<u64> {
        Vec::new()
    }
}

/// A script's effect, for the host's systems to apply.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// The tick it was given at.
    pub tick: u64,
    /// Its name.
    pub name: String,
    /// Its arguments (entities as [`Data::Entity`]).
    pub args: Vec<Data>,
}

/// The timers: by the tick they are due and their number, their period in ticks (none: once)
/// and their handler.
type Timers = BTreeMap<(u64, u64), (Option<u64>, Function)>;

/// The time a script's handlers took for one event's name, since [`Game::timings`] was last
/// called: for the host's profile (its F1 overlay).
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// The event (`"tick"`, `"input"`, the host's own), or `"timers"` for the timers run at
    /// each tick.
    pub event: String,
    /// The times it ran.
    pub runs: u32,
    /// Milliseconds in all.
    pub ms: f64,
}

/// A game script, its handlers and its timers.
pub struct Game {
    script: Script,
    name: String,
    limits: Limits,
    rate: u32,
    handlers: Table,
    timers: Rc<RefCell<Timers>>,
    tick: Rc<Cell<u64>>,
    commands: Rc<RefCell<Vec<Command>>>,
    timings: BTreeMap<String, (u32, f64)>,
}

impl Game {
    /// Runs the script `source` named `name` once under `limits`, with the `game` table, for it
    /// to register its handlers; `rate` ticks a second count its timers' seconds.
    pub fn new(limits: Limits, name: &str, source: &str, rate: u32) -> Result<Self, Error> {
        Self::start(limits, name, source, rate, 0)
    }

    /// [`Game::new`] at tick `tick`: its timers count from there.
    fn start(
        limits: Limits,
        name: &str,
        source: &str,
        rate: u32,
        tick: u64,
    ) -> Result<Self, Error> {
        let mut script = Script::new(limits.clone())?;
        let host = |e: mlua::Error| Error::Host(e.to_string());
        let lua = script.lua.clone();
        let game = lua.create_table().map_err(host)?;
        let handlers = lua.create_table().map_err(host)?;
        let timers: Rc<RefCell<Timers>> = Rc::default();
        let tick = Rc::new(Cell::new(tick));
        let commands: Rc<RefCell<Vec<Command>>> = Rc::default();
        let next = Rc::new(Cell::new(1_u64));
        let per_second = f64::from(rate.max(1));
        let list = handlers.clone();
        let on = lua
            .create_function(move |lua, (event, handler): (String, Function)| {
                let row: Table = match list.raw_get::<Option<Table>>(event.as_str())? {
                    Some(row) => row,
                    None => {
                        let row = lua.create_table()?;
                        list.raw_set(event.as_str(), row.clone())?;
                        row
                    }
                };
                row.raw_push(handler)
            })
            .map_err(host)?;
        let timer = |every: bool| {
            let (timers, tick, next) = (Rc::clone(&timers), Rc::clone(&tick), Rc::clone(&next));
            lua.create_function(move |_, (seconds, handler): (f64, Function)| {
                let ticks = ((seconds.max(0.0) * per_second).round() as u64).max(1);
                let id = next.get();
                next.set(id + 1);
                timers
                    .borrow_mut()
                    .insert((tick.get() + ticks, id), (every.then_some(ticks), handler));
                Ok(id as f64)
            })
        };
        let after = timer(false).map_err(host)?;
        let every = timer(true).map_err(host)?;
        let cancelled = Rc::clone(&timers);
        let cancel = lua
            .create_function(move |_, id: f64| {
                cancelled.borrow_mut().retain(|&(_, k), _| k != id as u64);
                Ok(())
            })
            .map_err(host)?;
        let (queue, now) = (Rc::clone(&commands), Rc::clone(&tick));
        let command = lua
            .create_function(move |_, (name, args): (String, MultiValue)| {
                let args = args
                    .into_iter()
                    .map(|v| to_data(v, 0, &mut HashSet::new()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(mlua::Error::runtime)?;
                queue.borrow_mut().push(Command {
                    tick: now.get(),
                    name,
                    args,
                });
                Ok(())
            })
            .map_err(host)?;
        for (key, f) in [
            ("on", on),
            ("after", after),
            ("every", every),
            ("cancel", cancel),
            ("command", command),
        ] {
            game.raw_set(key, f).map_err(host)?;
        }
        // The world's readers: what an event lends (in the registry) while it runs, else an
        // error. Luau resolves `game.find` once, when a handler loads (an import of its safe
        // environment), so the functions stay and what they call changes.
        for key in ["get", "find"] {
            let lent = format!("forge-script world {key}");
            let f = lua
                .create_function(move |lua, args: MultiValue| -> mlua::Result<MultiValue> {
                    match lua.named_registry_value::<Option<Function>>(&lent)? {
                        Some(reader) => reader.call(args),
                        None => Err(mlua::Error::runtime(format!(
                            "game.{key} reads the world only while an event or a tick runs"
                        ))),
                    }
                })
                .map_err(host)?;
            game.raw_set(key, f).map_err(host)?;
        }
        lua.globals().set("game", game.clone()).map_err(host)?;
        script.injected.insert("game".to_owned());
        script.run(name, source)?;
        Ok(Self {
            script,
            name: name.to_owned(),
            limits,
            rate,
            handlers,
            timers,
            tick,
            commands,
            timings: BTreeMap::new(),
        })
    }

    /// Starts the script again from `source` at the tick it is at: hot reload while the game
    /// runs. The new script's handlers and timers replace the old ones (its timers count from
    /// now); the commands the host has not taken yet stay, before any the new script gives as
    /// it loads. When `source` fails to load, the old script runs on and the error is returned.
    pub fn reload(&mut self, source: &str) -> Result<(), Error> {
        let mut next = Self::start(
            self.limits.clone(),
            &self.name,
            source,
            self.rate,
            self.tick.get(),
        )?;
        let mut kept = self.commands();
        kept.append(&mut next.commands.borrow_mut());
        *next.commands.borrow_mut() = kept;
        // The old script's times since the last call, then the new one's.
        for (event, (runs, ms)) in std::mem::take(&mut self.timings) {
            let t = next.timings.entry(event).or_default();
            t.0 += runs;
            t.1 += ms;
        }
        *self = next;
        Ok(())
    }

    /// Sends the built-in `"input"` event: the action's name (the game's, as its `bind` lines
    /// name it, D-069) and its value: 1 pressed and 0 released for a button, −1 to 1 for an axis.
    pub fn input(&mut self, action: &str, value: f64, world: &dyn World) -> Result<(), Error> {
        self.send(
            "input",
            &[Data::String(action.to_owned()), Data::Number(value)],
            world,
        )
    }

    /// The time each event's handlers took since the last call, by the event's name: for the
    /// host's profile.
    pub fn timings(&mut self) -> Vec<Timing> {
        std::mem::take(&mut self.timings)
            .into_iter()
            .map(|(event, (runs, ms))| Timing { event, runs, ms })
            .collect()
    }

    /// The tick it is at: the ticks run so far.
    pub fn now(&self) -> u64 {
        self.tick.get()
    }

    /// Sends event `event` with `args` to its handlers, in their order, `world` read meanwhile.
    pub fn send(&mut self, event: &str, args: &[Data], world: &dyn World) -> Result<(), Error> {
        let host = |e: mlua::Error| Error::Host(e.to_string());
        let row: Option<Table> = self.handlers.raw_get(event).map_err(host)?;
        let handlers: Vec<Function> = match row {
            Some(row) => row
                .sequence_values::<Function>()
                .collect::<mlua::Result<_>>()
                .map_err(host)?,
            None => Vec::new(),
        };
        self.run(event, world, |lua| {
            let values = args
                .iter()
                .map(|a| to_lua(lua, a))
                .collect::<mlua::Result<Vec<Value>>>()?;
            for h in &handlers {
                h.call::<()>(MultiValue::from_iter(values.iter().cloned()))?;
            }
            Ok(())
        })
    }

    /// One tick: the timers due run, in the order they were set, then the `"tick"` handlers
    /// with the tick's number.
    pub fn tick(&mut self, world: &dyn World) -> Result<(), Error> {
        let now = self.tick.get() + 1;
        self.tick.set(now);
        let timers = Rc::clone(&self.timers);
        self.run("timers", world, |_| {
            loop {
                let due = {
                    let mut t = timers.borrow_mut();
                    match t.first_key_value() {
                        Some((&(at, _), _)) if at <= now => t.pop_first(),
                        _ => None,
                    }
                };
                let Some(((_, id), (period, handler))) = due else {
                    break;
                };
                // A repeating timer set again before its handler runs, which may cancel it.
                if let Some(p) = period {
                    timers
                        .borrow_mut()
                        .insert((now + p, id), (Some(p), handler.clone()));
                }
                handler.call::<()>(())?;
            }
            Ok(())
        })?;
        self.send("tick", &[Data::Integer(now as i64)], world)
    }

    /// The commands given since the last call, in their order.
    pub fn commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands.borrow_mut())
    }

    /// The timers set and not yet run or cancelled.
    pub fn timers(&self) -> usize {
        self.timers.borrow().len()
    }

    /// Runs `f` with `game.get` and `game.find` reading `world`, under the script's caps afresh.
    fn run(
        &mut self,
        what: &str,
        world: &dyn World,
        f: impl FnOnce(&mlua::Lua) -> mlua::Result<()>,
    ) -> Result<(), Error> {
        self.script.spent.set(0);
        let began = std::time::Instant::now();
        self.script.began.set(began);
        let lua = self.script.lua.clone();
        let result = lua.scope(|scope| {
            let get =
                scope.create_function(|lua, (e, field): (mlua::UserDataRef<Entity>, String)| {
                    to_lua(lua, &world.get(e.0, &field))
                })?;
            let find = scope.create_function(|lua, kind: String| {
                let t = lua.create_table()?;
                for id in world.find(&kind) {
                    t.raw_push(Entity(id))?;
                }
                Ok(t)
            })?;
            lua.set_named_registry_value("forge-script world get", get)?;
            lua.set_named_registry_value("forge-script world find", find)?;
            let out = f(&lua);
            lua.unset_named_registry_value("forge-script world get")?;
            lua.unset_named_registry_value("forge-script world find")?;
            out
        });
        let timing = self.timings.entry(what.to_owned()).or_default();
        timing.0 += 1;
        timing.1 += began.elapsed().as_secs_f64() * 1e3;
        result.map_err(|e| self.script.error(&format!("{}: {what}", self.name), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three zombies (ids 7, 9, 12) with their health.
    struct Town;

    impl World for Town {
        fn get(&self, id: u64, field: &str) -> Data {
            match field {
                "health" => Data::Integer(id as i64 * 10),
                _ => Data::Nil,
            }
        }
        fn find(&self, kind: &str) -> Vec<u64> {
            if kind == "zombie" {
                vec![7, 9, 12]
            } else {
                Vec::new()
            }
        }
    }

    fn game(source: &str) -> Game {
        Game::new(Limits::default(), "test", source, 10).unwrap()
    }

    #[test]
    fn handlers_answer_events_in_order_with_commands_stamped_by_tick() {
        let mut g = game(
            r#"
            game.on("noise", function(at, loud) game.command("look", at, loud) end)
            game.on("noise", function(at) game.command("turn", at) end)
            game.on("tick", function(n) if n == 2 then game.command("second") end end)
            "#,
        );
        g.send("noise", &[Data::Integer(3), Data::Bool(true)], &Town)
            .unwrap();
        g.tick(&Town).unwrap();
        g.tick(&Town).unwrap();
        let c = g.commands();
        let names: Vec<(&str, u64)> = c.iter().map(|c| (c.name.as_str(), c.tick)).collect();
        assert_eq!(names, vec![("look", 0), ("turn", 0), ("second", 2)]);
        assert_eq!(c[0].args, vec![Data::Integer(3), Data::Bool(true)]);
        assert!(g.commands().is_empty());
    }

    #[test]
    fn timers_count_ticks_in_the_order_they_were_set_and_stop_when_cancelled() {
        // At 10 ticks a second: once after half a second, every fifth of a second, and one
        // cancelled.
        let mut g = game(
            r#"
            game.after(0.5, function() game.command("after") end)
            local every
            every = game.every(0.2, function()
                game.command("every")
            end)
            local never = game.after(0.3, function() game.command("never") end)
            game.cancel(never)
            game.on("tick", function(n) if n == 6 then game.cancel(every) end end)
            "#,
        );
        let mut log = Vec::new();
        for _ in 0..10 {
            g.tick(&()).unwrap();
            log.extend(g.commands().into_iter().map(|c| (c.tick, c.name)));
        }
        let expect: Vec<(u64, String)> = [(2, "every"), (4, "every"), (5, "after"), (6, "every")]
            .into_iter()
            .map(|(t, n)| (t, n.to_owned()))
            .collect();
        assert_eq!(log, expect);
        assert_eq!(g.timers(), 0);
    }

    #[test]
    fn entities_are_handles_read_through_the_world_only_during_events() {
        let mut g = game(
            r#"
            game.on("tick", function()
                local zs = game.find("zombie")
                for _, z in ipairs(zs) do
                    if game.get(z, "health") > 80 then game.command("flee", z) end
                end
                game.command("same", zs[1] == game.find("zombie")[1], tostring(zs[2]), zs[3]:id())
            end)
            "#,
        );
        g.tick(&Town).unwrap();
        let c = g.commands();
        assert_eq!(c[0].name, "flee");
        assert_eq!(c[0].args, vec![Data::Entity(9)]);
        assert_eq!(c[1].args, vec![Data::Entity(12)]);
        assert_eq!(
            c[2].args,
            vec![
                Data::Bool(true),
                Data::String("entity 9".to_owned()),
                Data::Integer(12)
            ]
        );
        // An entity sent with an event is a handle too; outside events the world is closed.
        g.send("hit", &[Data::Entity(7)], &Town).unwrap();
        let err = Game::new(Limits::default(), "early", "game.find('zombie')", 10)
            .err()
            .unwrap();
        assert!(err.to_string().contains("only while an event"), "{err}");
    }

    #[test]
    fn the_definitions_name_every_function_of_the_game_table() {
        let defs = include_str!("../defs/game.d.luau");
        let mut g = game("");
        let keys = g
            .script
            .run(
                "keys",
                "local ks = {} for k in pairs(game) do table.insert(ks, k) end table.sort(ks) return ks",
            )
            .unwrap();
        let Data::List(keys) = keys else {
            panic!("{keys:?}")
        };
        assert_eq!(keys.len(), 7, "{keys:?}");
        for key in keys {
            let Data::String(key) = key else {
                panic!("{key:?}")
            };
            assert!(
                defs.contains(&format!("\t{key}: (")),
                "{key} missing from game.d.luau"
            );
        }
        assert!(defs.contains("declare extern type Entity") && defs.contains("function id(self)"));
    }

    #[test]
    fn the_example_script_sends_the_healthy_after_a_noise_and_the_hurt_flee() {
        // Zombies 7 and 9 healthy, 12 hurt; then 7 hurt while it chases.
        struct Hurt(Cell<i64>);
        impl World for Hurt {
            fn get(&self, id: u64, _: &str) -> Data {
                Data::Integer(match id {
                    7 => self.0.get(),
                    9 => 80,
                    _ => 10,
                })
            }
            fn find(&self, _: &str) -> Vec<u64> {
                vec![7, 9, 12]
            }
        }
        let world = Hurt(Cell::new(90));
        let source = include_str!("../scripts/pursuit.luau");
        let mut g = Game::new(Limits::default(), "pursuit", source, 10).unwrap();
        g.send("noise", &[Data::Number(4.5), Data::Number(-2.0)], &world)
            .unwrap();
        let go: Vec<Vec<Data>> = g.commands().into_iter().map(|c| c.args).collect();
        assert_eq!(
            go,
            vec![
                vec![Data::Entity(7), Data::Number(4.5), Data::Integer(-2)],
                vec![Data::Entity(9), Data::Number(4.5), Data::Integer(-2)],
            ]
        );
        world.0.set(20);
        for _ in 0..10 {
            g.tick(&world).unwrap();
        }
        let flee: Vec<(u64, String, Vec<Data>)> = g
            .commands()
            .into_iter()
            .map(|c| (c.tick, c.name, c.args))
            .collect();
        assert_eq!(flee, vec![(10, "flee".to_owned(), vec![Data::Entity(7)])]);
    }

    #[test]
    fn a_reload_replaces_the_handlers_and_timers_at_the_same_tick_and_keeps_the_commands() {
        let mut g = game(
            r#"
            game.after(1.0, function() game.command("old timer") end)
            game.on("tick", function(n) game.command("old", n) end)
            "#,
        );
        for _ in 0..3 {
            g.tick(&()).unwrap();
        }
        // A script that does not load: the old one runs on.
        assert!(g.reload("game.on(").is_err());
        g.tick(&()).unwrap();
        g.reload(
            r#"
            game.command("loaded")
            game.after(0.5, function() game.command("new timer") end)
            game.on("tick", function(n) game.command("new", n) end)
            "#,
        )
        .unwrap();
        assert_eq!(g.now(), 4);
        for _ in 0..6 {
            g.tick(&()).unwrap();
        }
        let log: Vec<(u64, String)> = g.commands().into_iter().map(|c| (c.tick, c.name)).collect();
        let expect: Vec<(u64, String)> = [
            (1, "old"),
            (2, "old"),
            (3, "old"),
            (4, "old"),
            (4, "loaded"),
            (5, "new"),
            (6, "new"),
            (7, "new"),
            (8, "new"),
            (9, "new timer"),
            (9, "new"),
            (10, "new"),
        ]
        .into_iter()
        .map(|(t, n)| (t, n.to_owned()))
        .collect();
        // The old timer (due at tick 10) went with its script.
        assert_eq!(log, expect);
        assert_eq!(g.timers(), 0);
        // The ticks' times across the reload: the old script's 4 and the new one's 6.
        let ticks = g.timings().into_iter().find(|t| t.event == "tick");
        assert_eq!(ticks.map(|t| t.runs), Some(10));
    }

    #[test]
    fn input_is_an_event_and_each_events_time_is_measured() {
        let mut g = game(
            r#"
            game.on("input", function(action, value)
                if action == "fire" and value > 0.5 then game.command("shoot") end
            end)
            "#,
        );
        g.input("fire", 1.0, &()).unwrap();
        g.input("fire", 0.0, &()).unwrap();
        g.input("look_x", -0.25, &()).unwrap();
        g.tick(&()).unwrap();
        let names: Vec<String> = g.commands().into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["shoot".to_owned()]);
        let timings = g.timings();
        let runs: Vec<(&str, u32)> = timings.iter().map(|t| (t.event.as_str(), t.runs)).collect();
        assert_eq!(runs, vec![("input", 3), ("tick", 1), ("timers", 1)]);
        assert!(timings.iter().all(|t| t.ms >= 0.0));
        assert!(g.timings().is_empty());
    }

    #[test]
    fn a_handler_over_its_budget_ends_and_the_game_goes_on() {
        let limits = Limits {
            interrupts: 10_000,
            ..Limits::default()
        };
        let mut g = Game::new(
            limits,
            "busy",
            r#"
            game.on("spin", function() while true do end end)
            game.on("tick", function(n) game.command("alive", n) end)
            assert(os == nil)
            "#,
            10,
        )
        .unwrap();
        let err = g.send("spin", &[], &()).unwrap_err();
        assert!(matches!(err, Error::Budget { .. }), "{err}");
        g.tick(&()).unwrap();
        assert_eq!(g.commands()[0].args, vec![Data::Integer(1)]);
    }
}
