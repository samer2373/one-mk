(() => { const b = Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge; const r = b._renderer;
 if (!r) return "no renderer; events=" + JSON.stringify(b.stats.events.slice(-6));
 const f = r.win.get_frame_rect(), bf = r.win.get_buffer_rect();
 return JSON.stringify({title: r.win.get_title(), frame: [f.x,f.y,f.width,f.height], buffer: [bf.x,bf.y,bf.width,bf.height], actorVisible: r.actor.visible, opacity: r.actor.opacity, mapped: r.actor.mapped, damage: b.stats.damage, focus: global.display.focus_window?.get_title() ?? null, type: r.win.get_window_type(), paused: b._paused, reason: b._reason, stale: b._stale, events: b.stats.events.slice(-6), hints: b.stats.hints.slice(-4)}); })()
