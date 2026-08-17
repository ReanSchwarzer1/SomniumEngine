--!strict
--
-- First-person camera.
--
-- Goes on a **child** of the player entity. The player script owns yaw and
-- writes it to the player's transform; this owns pitch and writes it to
-- its own. Because the camera is a child, the hierarchy multiplies the
-- two together — so neither script has to know the other exists, and
-- either can be replaced on its own.
--
-- Its local translation is the eye offset, which is why the camera sits at
-- head height rather than at the player's feet.

return Script.define({
	apiVersion = 1,
	schemaVersion = 1,

	uses = {
		["somnium.Transform"] = { "translation", "rotation" },
	},

	fields = {
		eyeHeight = Field.number(0.72, { min = -2.0, max = 3.0,
			description = "Metres above the capsule's centre. The player is 1.8 m tall, so this is roughly eye level." }),
		mouseSensitivity = Field.number(0.12, { min = 0.005, max = 2.0,
			description = "Degrees of pitch per pixel of mouse movement. Match the controller's to keep looking even." }),
		pitchLimit = Field.number(89.0, { min = 0.0, max = 89.9,
			description = "How far up or down you may look. Stops short of 90 so the view never flips." }),
		invertMouseY = Field.boolean(false),
	},

	onInit = function(self, ctx)
		-- `or` rather than `=`: a hot reload calls `loadState` before
		-- `onInit` replays, so assigning here would snap the view back to
		-- level every time someone edited the file.
		self.pitch = self.pitch or 0.0
	end,

	onFixedUpdate = function(self, ctx, dt)
		local transform = ctx.self.transform

		local dy = ctx.input.mouseDeltaY
		if self.invertMouseY then
			dy = -dy
		end
		self.pitch = self.pitch - dy * self.mouseSensitivity

		-- Clamped, not wrapped. Looking past straight up should stop, not
		-- roll the world over.
		if self.pitch > self.pitchLimit then
			self.pitch = self.pitchLimit
		elseif self.pitch < -self.pitchLimit then
			self.pitch = -self.pitchLimit
		end

		-- Pitch about local X. Yaw is the parent's, and the hierarchy
		-- composes them — doing both here would gimbal.
		local half = math.rad(self.pitch) * 0.5
		transform.rotation = { x = math.sin(half), y = 0.0, z = 0.0, w = math.cos(half) }

		-- Local offset from the player's origin, so the eye stays at head
		-- height however the capsule moves.
		transform.translation = vector.create(0.0, self.eyeHeight, 0.0)
	end,

	saveState = function(self)
		return { pitch = self.pitch }
	end,

	loadState = function(self, state)
		self.pitch = state.pitch
	end,
})
