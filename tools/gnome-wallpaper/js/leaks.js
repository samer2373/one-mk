(() => { const names = []; const walk = (a, d) => { if (a.name && a.name.startsWith('one-mk')) names.push(a.name); for (const c of a.get_children()) walk(c, d + 1); }; walk(global.stage, 0);
 const clones = []; const walk2 = a => { if (a instanceof imports.gi.Clutter.Clone) clones.push(a.source?.constructor.name ?? 'null'); for (const c of a.get_children()) walk2(c); }; walk2(global.stage);
 const ext = Main.extensionManager.lookup("wallpaper-spike@one-mk.test");
 return JSON.stringify({oneMkActors: names, clonesOnStage: clones, extState: ext.state, bridge: !!ext.stateObj?.bridge, windows: global.get_window_actors().map(a => a.get_meta_window().get_title())}); })()
