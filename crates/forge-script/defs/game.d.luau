--!strict
-- The `game` table a game script gets (#319, D-072; `forge_script::Game`), for luau-lsp:
--   luau-lsp analyze --definitions=crates/forge-script/defs/game.d.luau my_script.luau
-- Written by hand from `crates/forge-script/src/game.rs`; kept beside it until D-072's
-- definition files are generated from the Rust API.

-- An entity's handle: its stable id, opaque to the script. Handles compare equal when their ids
-- do, and print as "entity <id>".
declare class Entity
	-- The stable id the host knows the entity by.
	function id(self): number
end

declare game: {
	-- Answers event `event` (the host's names; "tick" each tick, with the tick's number). An
	-- event's handlers run in the order they were registered.
	on: (event: string, handler: (...any) -> ()) -> (),
	-- Runs `handler` once, `seconds` from now (counted in the host's ticks, never a clock); the
	-- timer's number.
	after: (seconds: number, handler: () -> ()) -> number,
	-- Runs `handler` every `seconds` (in ticks), until cancelled; the timer's number.
	every: (seconds: number, handler: () -> ()) -> number,
	-- Stops timer `timer`.
	cancel: (timer: number) -> (),
	-- Field `field` of `entity` in the world, nil for none. Only while an event or a tick runs.
	get: (entity: Entity, field: string) -> any,
	-- The entities of `kind`, in the order of their ids. Only while an event or a tick runs.
	find: (kind: string) -> { Entity },
	-- Asks the host to apply command `name` with its arguments, stamped with the tick. A script
	-- never changes the world itself.
	command: (name: string, ...any) -> (),
}
