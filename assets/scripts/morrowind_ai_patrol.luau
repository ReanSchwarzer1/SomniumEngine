--!strict
-- Attach to an actor with Navigation Agent and Perception Sensor. Bake a
-- Navigation Profile, then Play. Uses the existing generic component API.
-- The same module can be explicitly bound as a ScriptTaskHost task by a game.
return Script.define({
    apiVersion = 1,
    schemaVersion = 1,
    fields = {
        nearX = Field.number(-3.0),
        farX = Field.number(3.0),
        destinationY = Field.number(0.0),
        destinationZ = Field.number(0.0),
        speed = Field.number(2.4, { min = 0.1, max = 100.0 }),
    },
    onFixedUpdate = function(self, ctx, dt)
        local arrived = ctx:get(ctx.entity, "somnium.NavigationAgent", "arrived")
        if self.outbound == nil or arrived then
            self.outbound = if self.outbound == nil then true else not self.outbound
            local x = if self.outbound then self.farX else self.nearX
            ctx:set(ctx.entity, "somnium.NavigationAgent", "destination",
                vector.create(x, self.destinationY, self.destinationZ))
            ctx:set(ctx.entity, "somnium.NavigationAgent", "speed", self.speed)
        end
        -- Diagnostics are script-readable and remain owned by the runtime.
        self.confidence = ctx:get(ctx.entity, "somnium.Perception", "confidence") or 0
    end,
    loadState = function(self, state)
        self.outbound = state.outbound
        self.board = state.blackboard or {}
        self.status = state.status or "running"
    end,
    onEvent = function(self, ctx, events)
        for _, event in ipairs(events) do
            if event.name == "ai.cancel" then
                ctx:set(ctx.entity, "somnium.NavigationAgent", "enabled", false)
                self.status = "failure"
            elseif event.name == "ai.tick" then
                ctx:set(ctx.entity, "somnium.NavigationAgent", "enabled", true)
                local confidence = ctx:get(ctx.entity, "somnium.Perception", "confidence") or 0
                self.board.alert = confidence > 0
                self.status = if ctx:get(ctx.entity, "somnium.NavigationAgent", "arrived") then "success" else "running"
            end
        end
    end,
    saveState = function(self)
        return { outbound = self.outbound, blackboard = self.board or {}, status = self.status or "running" }
    end,
})
