--!strict
--
-- Phase 16-C gate script.
--
-- Everything the sub-phase promises, in one file: fixed-step motion,
-- input, a physics force, a spawn and the despawn of what it spawned, an
-- event emitted and heard, and state that survives a reload.
--
-- Read `docs/editor/scripting.md` before changing this — the visibility
-- rule (one script's writes are invisible to another until the commit
-- point) is what makes the `ctx.self` mirror the right way to touch your
-- own components, and `ctx:get`/`ctx:set` the right way to touch someone
-- else's.

return Script.define({
	apiVersion = 1,
	schemaVersion = 1,

	-- Naming the fields, not just the component, is load-bearing: mirroring
	-- all of Transform marshals a rotation quaternion in both directions
	-- every frame even for a script that never reads it, and that measured
	-- 2.31 → 5.53 ms across a thousand entities.
	uses = { ["somnium.Transform"] = { "translation", "rotation" } },

	fields = {
		spinSpeed = Field.number(1.5, { min = 0.0, max = 20.0,
			description = "Radians per second about Y." }),
		thrust = Field.number(400.0, {
			description = "Newtons applied while W is held." }),
		riseHeight = Field.number(0.35, {
			description = "Amplitude of the vertical bob, in metres." }),
	},

	onInit = function(self, ctx)
		self.angle = self.angle or 0.0
		self.ticks = self.ticks or 0
		self.baseY = nil
		self.marker = nil
		ctx:log("rotator initialised")
	end,

	onStart = function(self, ctx)
		-- Delivered to every enabled listener on the next phase, with a
		-- sequence number that is the same on every replay.
		ctx:emit("rotator.started")
	end,

	onEvent = function(self, ctx, events)
		for _, event in ipairs(events) do
			ctx:log("heard " .. event.name .. " #" .. tostring(event.sequence))
		end
	end,

	onFixedUpdate = function(self, ctx, dt)
		local t = ctx.self.transform

		if self.baseY == nil then
			self.baseY = t.translation.y
		end

		self.angle = self.angle + self.spinSpeed * dt
		self.ticks = self.ticks + 1

		-- Rotation is four numbers; the engine re-tags it as a quaternion
		-- from the field's declared type, so writing a table is correct.
		local half = self.angle * 0.5
		t.rotation = { x = 0.0, y = math.sin(half), z = 0.0, w = math.cos(half) }

		-- Reads its own write, because the mirror is a plain table. Through
		-- `ctx:get`/`ctx:set` this loop would re-read the pre-phase value
		-- every iteration and only the last write would survive.
		t.translation = vector.create(
			t.translation.x,
			self.baseY + math.sin(self.angle) * self.riseHeight,
			t.translation.z
		)

		if ctx.input:isKeyDown(string.byte("W")) then
			ctx:applyForce(ctx.entity, vector.create(0.0, 0.0, -self.thrust))
		end

		-- A spawn returns a token, not an entity: the entity does not exist
		-- until the commit point. It arrives in `ctx.spawns` next phase.
		if self.ticks == 60 then
			self.token = ctx:spawn()
			ctx:log("spawned a marker")
		elseif self.token ~= nil and ctx.spawns ~= nil then
			local spawned = ctx.spawns[self.token]
			if spawned ~= nil then
				ctx:despawn(spawned)
				ctx:log("despawned the marker")
				self.token = nil
			end
		end
	end,

	onDestroy = function(self, ctx)
		ctx:log("rotator torn down after " .. tostring(self.ticks) .. " steps")
	end,

	-- Only pure data survives a reload. A closure, a coroutine or userdata
	-- returned here is refused rather than silently dropped.
	saveState = function(self)
		return { angle = self.angle, ticks = self.ticks }
	end,

	loadState = function(self, state)
		self.angle = state.angle
		self.ticks = state.ticks
	end,
})
