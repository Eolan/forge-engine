//! Game scripts (#319, D-072): a script that answers the game's events with commands, outside
//! the predicted step.
//!
//! The script runs once to register what it answers on the `game` table the host gives it:
//! - **Events:** `game.on(name, handler)`. The host sends events ([`Game::send`]: an input, an
//!   entity killed, anything it names) and ticks ([`Game::tick`], the `"tick"` event, with the
//!   tick's number). Each event's handlers run in the order they were registered.
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
//! sandbox.

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

/// A game script, its handlers and its timers.
pub struct Game {
    script: Script,
    name: String,
    handlers: Table,
    timers: Rc<RefCell<Timers>>,
    tick: Rc<Cell<u64>>,
    commands: Rc<RefCell<Vec<Command>>>,
}

impl Game {
    /// Runs the script `source` named `name` once under `limits`, with the `game` table, for it
    /// to register its handlers; `rate` ticks a second count its timers' seconds.
    pub fn new(limits: Limits, name: &str, source: &str, rate: u32) -> Result<Self, Error> {
        let mut script = Script::new(limits)?;
        let host = |e: mlua::Error| Error::Host(e.to_string());
        let lua = script.lua.clone();
        let game = lua.create_table().map_err(host)?;
        let handlers = lua.create_table().map_err(host)?;
        let timers: Rc<RefCell<Timers>> = Rc::default();
        let tick = Rc::new(Cell::new(0_u64));
        let commands: Rc<RefCell<Vec<Command>>> = Rc::default();
        let next = Rc::new(Cell::new(1_u64));
        let rate = f64::from(rate.max(1));
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
                let ticks = ((seconds.max(0.0) * rate).round() as u64).max(1);
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
            handlers,
            timers,
            tick,
            commands,
        })
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
        self.script.began.set(std::time::Instant::now());
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
