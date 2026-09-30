-- Omatainer: opaque media surface, inhibit idle while focused.
-- Copy to ~/.config/hypr/apps/omatainer.lua and require from hyprland.lua.
o.window("org.omarchy.omatainer", { tag = "+omatainer" })
o.window("org.omarchy.omatainer", { tag = "-default-opacity" })
o.window("org.omarchy.omatainer", { opacity = "1 1" })
o.window("org.omarchy.omatainer", { idle_inhibit = "focus" })
o.window("org.omarchy.omatainer", { opaque = true })
