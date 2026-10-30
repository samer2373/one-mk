(() => { const Shell = imports.gi.Shell; const b = Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge; const rw = b._renderer?.win;
 const tab = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null).map(w => w.get_title());
 const running = Shell.AppSystem.get_default().get_running().map(a => a.get_id() + ":" + a.get_windows().map(w => w.get_title()).join("|"));
 const tracked = rw ? Shell.WindowTracker.get_default().get_window_app(rw)?.get_id() ?? null : null;
 return JSON.stringify({altTabCandidates: tab, runningApps: running, rendererApp: tracked, rendererSkipTaskbar: rw?.skip_taskbar, rendererOnAllWorkspaces: rw?.is_on_all_workspaces(), nWorkspaces: global.workspace_manager.n_workspaces, active: global.workspace_manager.get_active_workspace_index(), focus: global.display.focus_window?.get_title() ?? null}); })()
