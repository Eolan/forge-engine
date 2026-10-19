--!strict
-- An example game script (#319, D-072): zombies that hear a noise go for it, and those hurt
-- flee. It reads the world through `game.get` and `game.find` and acts only through commands;
-- the host's Rust systems apply them. `tools/luau-check.sh` type-checks it against
-- `crates/forge-script/defs/game.d.luau`, and forge-script's tests run it.

-- How hurt a zombie must be to flee: its health at most this.
local FLEEING = 30

-- The zombies sent after the last noise, by id, with the tick it was heard at.
local chasing: { [number]: number } = {}

-- A noise heard (the host's event): every zombie that is not fleeing goes for it.
game.on("noise", function(x: number, z: number)
	for _, zombie in ipairs(game.find("zombie")) do
		local health = game.get(zombie, "health") :: number
		if health > FLEEING then
			game.command("go", zombie, x, z)
			chasing[zombie:id()] = 0
		end
	end
end)

-- Every second, the hurt ones among those chasing flee.
game.every(1, function()
	for _, zombie in ipairs(game.find("zombie")) do
		if chasing[zombie:id()] and (game.get(zombie, "health") :: number) <= FLEEING then
			game.command("flee", zombie)
			chasing[zombie:id()] = nil
		end
	end
end)
