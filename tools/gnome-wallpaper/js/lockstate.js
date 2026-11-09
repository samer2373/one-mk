(() => { const e = Main.extensionManager.lookup("wallpaper-spike@one-mk.test"); const b = e.stateObj?.bridge;
 return JSON.stringify({locked: Main.screenShield.locked, mode: Main.sessionMode.currentMode, spikeState: e.state, layerVisible: b ? b._layer.visible : null, hints: b ? b.stats.hints.slice(-2) : null}); })()
