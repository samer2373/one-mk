const Gio = imports.gi.Gio;
const sourcePath = ARGV[0];
const [, bytes] = Gio.File.new_for_path(sourcePath).load_contents(null);
const source = new TextDecoder().decode(bytes)
    .replace(/^import .*;\n/gm, '')
    .replace('export default class WallpaperExtension', 'class WallpaperExtension');
const Main = {
    layoutManager: {monitors: [{index: 0}]},
    sessionMode: {isLocked: false}, screenShield: {locked: false},
    overview: {visible: false},
};
const shell = {
    workspace_manager: {get_active_workspace: () => ({})},
    get_window_actors: () => [],
};
const Bridge = new Function('Main', 'global', 'Extension', `${source}\nreturn Bridge;`)(Main, shell, class {});
const bridge = Object.create(Bridge.prototype);
Object.assign(bridge, {
    _sources: new Map([['layout', 1]]), _monitors: [[0, 0, 100, 100, 1]],
    _renderer: {fresh: true}, _clones: [], _layer: {visible: false},
    _state: 4, _ownsName: true, _login: {preparingForSleep: false},
    _reasons: 0, _recovery: {}, _restart: () => {}, _watchdog: () => {},
});
bridge._coverage();
if (bridge._layer.visible)
    throw new Error('coverage re-revealed the old slices during monitor-layout debounce');
bridge._sources.delete('layout');
bridge._coverage();
if (!bridge._layer.visible)
    throw new Error('settled fresh layout must remain revealable');
print('PASS: layout debounce cannot reveal stale slices');
