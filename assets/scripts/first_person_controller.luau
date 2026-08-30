--!strict
--
-- First-person character controller.
--
-- Goes on the player entity, which needs a `somnium.RigidBody` (a Jolt
-- capsule) and a `somnium.Transform`. The camera is a *child* entity
-- carrying `first_person_camera.luau`; this script owns yaw, the camera
-- owns pitch, and the hierarchy multiplies them together — which is why
-- neither script has to know about the other.
--
-- Every field below shows up in the Details panel. Change one while Play
-- is running and it takes effect on the next frame.

return Script.define({
	apiVersion = 1,
	schemaVersion = 1,

	-- Naming the fields matters: mirroring the whole of Transform would
	-- marshal a scale we never touch, every frame, for nothing.
	uses = {
		["somnium.Transform"] = { "translation", "rotation" },
		["somnium.RigidBody"] = { "velocity", "grounded" },
	},

	fields = {
		walkSpeed = Field.number(4.5, { min = 0.0, max = 30.0,
			description = "Metres per second on the ground." }),
		runSpeed = Field.number(8.0, { min = 0.0, max = 40.0,
			description = "Metres per second with Shift held." }),
		jumpSpeed = Field.number(5.2, { min = 0.0, max = 30.0,
			description = "Upward metres per second at the moment of a jump." }),
		airControl = Field.number(0.25, { min = 0.0, max = 1.0,
			description = "How much of your input steers you while airborne. 0 is none." }),
		lookSensitivity = Field.number(1.2, { min = 0.01, max = 10.0,
			description = "Degrees of yaw per Look action unit." }),
		invertMouseX = Field.boolean(false),
	},

	onInit = function(self, ctx)
		-- `or` rather than `=`: a hot reload calls `loadState` *before*
		-- `onInit` replays, so assigning unconditionally here would throw
		-- away the state that was just restored and spin the player round
		-- every time someone edited the file.
		self.yaw = self.yaw or 0.0

		-- Steps remaining before a jump may be attempted again.
		--
		-- Not politeness: `grounded` is a vertical-speed heuristic (there
		-- is no ground cast — see `somnium_core::character`), and vertical
		-- speed also passes through zero at the apex of a jump. Without
		-- this, holding Space gives you a second jump at the top of the
		-- first, and a third at the top of that.
		--
		-- Sized to the whole flight rather than to a guess: up and back
		-- down is `2 * jumpSpeed / g`, so the window closes at the moment
		-- you would have landed on flat ground. Landing early on a ledge
		-- means a short wait before you can jump again; that goes away
		-- when a real ground test replaces the heuristic.
		self.jumpCooldown = self.jumpCooldown or 0
		self.footstepDistance = self.footstepDistance or 0.0
		self.footstepIndex = self.footstepIndex or 1
	end,

	onStart = function(self, ctx)
		ctx:log("first-person controller ready — WASD, Shift to run, Space to jump")
	end,

	onFixedUpdate = function(self, ctx, dt)
		local body = ctx.self.rigidBody
		local transform = ctx.self.transform

		-- ── Look (yaw only; the camera child owns pitch) ─────────────
		local look = ctx.input:vector2("Look")
		local dx = look.x
		if self.invertMouseX then
			dx = -dx
		end
		self.yaw = self.yaw - dx * self.lookSensitivity

		-- Keep it in range so the number a script or a save sees stays
		-- small however long someone spins on the spot.
		if self.yaw > 180.0 then
			self.yaw = self.yaw - 360.0
		elseif self.yaw < -180.0 then
			self.yaw = self.yaw + 360.0
		end

		local yawRadians = math.rad(self.yaw)
		local half = yawRadians * 0.5
		transform.rotation = { x = 0.0, y = math.sin(half), z = 0.0, w = math.cos(half) }

		-- ── Move ─────────────────────────────────────────────────────
		local forwardX, forwardZ = -math.sin(yawRadians), -math.cos(yawRadians)
		local rightX, rightZ = math.cos(yawRadians), -math.sin(yawRadians)

		-- `Move` has already combined and normalised WASD/gamepad bindings.
		local move = ctx.input:vector2("Move")
		local wishX = rightX * move.x + forwardX * -move.y
		local wishZ = rightZ * move.x + forwardZ * -move.y

		local speed = if ctx.input:actionDown("Sprint") then self.runSpeed else self.walkSpeed
		local targetX, targetZ = wishX * speed, wishZ * speed

		-- Vertical velocity belongs to gravity and to the jump; the
		-- horizontal pair is ours. Writing all three would cancel the
		-- fall every step and leave the player hovering.
		local velocity = body.velocity
		local newX, newZ = targetX, targetZ
		if not body.grounded then
			-- Airborne: blend towards the target instead of snapping to
			-- it, so you cannot turn on a sixpence mid-jump.
			local t = self.airControl
			newX = velocity.x + (targetX - velocity.x) * t
			newZ = velocity.z + (targetZ - velocity.z) * t
		end

		local newY = velocity.y
		if self.jumpCooldown > 0 then
			self.jumpCooldown -= 1
		end
		-- `actionPressed`, not `actionDown`: holding the binding is one jump, not
		-- a jump every step you are near the ground.
		if body.grounded and self.jumpCooldown == 0 and ctx.input:actionPressed("Jump") then
			newY = self.jumpSpeed
			self.jumpCooldown = math.ceil(2.0 * self.jumpSpeed / 9.81 / dt)
		end

		body.velocity = vector.create(newX, newY, newZ)

		-- Grounded cadence drives four short CC0 variants through the same
		-- audio command path gameplay uses. Distance, not a timer, keeps steps
		-- matched to both walk and sprint speed.
		if body.grounded and (newX * newX + newZ * newZ) > 0.25 then
			self.footstepDistance += math.sqrt(newX * newX + newZ * newZ) * dt
			if self.footstepDistance >= 1.8 then
				self.footstepDistance -= 1.8
				local clips = {
					"audio/footsteps/footstep_01_cc0.ogg",
					"audio/footsteps/footstep_02_cc0.ogg",
					"audio/footsteps/footstep_03_cc0.ogg",
					"audio/footsteps/footstep_04_cc0.ogg",
				}
				ctx:playAudio(clips[self.footstepIndex], 0.45)
				self.footstepIndex = self.footstepIndex % #clips + 1
			end
		else
			self.footstepDistance = 0.0
		end
	end,

	-- Only pure data survives a reload; this is all of it.
	saveState = function(self)
		return { yaw = self.yaw, jumpCooldown = self.jumpCooldown,
			footstepDistance = self.footstepDistance, footstepIndex = self.footstepIndex }
	end,

	loadState = function(self, state)
		self.yaw = state.yaw
		self.jumpCooldown = state.jumpCooldown
		self.footstepDistance = state.footstepDistance or 0.0
		self.footstepIndex = state.footstepIndex or 1
	end,
})
