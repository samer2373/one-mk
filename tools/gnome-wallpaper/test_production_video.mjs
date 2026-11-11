import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../../extensions/gnome/one-mk-wallpaper@one-mk.org/extension.js', import.meta.url), 'utf8');
const secret = '0123456789abcdef0123456789abcdef';
function fixture() {
    const context = {
        Extension: class {}, Config: {PACKAGE_VERSION: '50.1'},
        GLib: {Variant: class { constructor(signature, value) { this.value = value; } }},
        Gio: {Credentials: class { get_unix_user() { return 1000; } }},
        Meta: {WindowType: {DESKTOP: 1}}, Main: {layoutManager: {monitors: [{x: -1920, y: 0, width: 1920, height: 1080}]}},
    };
    vm.createContext(context);
    vm.runInContext(source.replace(/^import .*;$/gm, '').replace('export default class ', 'class ') + '\nglobalThis.Video = WallpaperVideo;', context);
    const bridge = new context.Video();
    Object.assign(bridge, {_generation: 0, _client: null, _object: {}, _windows: new Map()});
    bridge._credential = async (_sender, method) => method.endsWith('User') ? 1000 : 42;
    const invocation = {get_sender: () => ':1.42', return_value(value) { this.value = value; }};
    const register = () => bridge._method('Register', invocation, {deep_unpack: () => [secret]});
    return {bridge, context, invocation, register};
}

test('enable rejects every Shell outside the GNOME 50 series before exporting anything', () => {
    for (const version of ['49.5', '49', '51.0', '5', '500.1', '', '4.50', 'x']) {
        const {bridge, context} = fixture();
        context.Config.PACKAGE_VERSION = version;
        assert.throws(() => bridge.enable(), /run on GNOME 50, not GNOME Shell/, version);
        assert.equal(bridge._object?.exported, undefined, 'nothing is exported before the version is accepted');
    }
});

test('the whole GNOME 50 series passes the version check', () => {
    for (const version of ['50', '50.0', '50.1', '50.2', '50.1-test']) {
        const {bridge, context} = fixture();
        context.Config.PACKAGE_VERSION = version;
        // The rest of enable() needs a real Shell (Clutter is not here); only the version check is under test.
        try { bridge.enable(); } catch (error) { assert.doesNotMatch(String(error.message), /run on GNOME 50/, version); }
    }
});

test('registration derives PID and UID from the bus, never from user arguments', async () => {
    const {bridge, register} = fixture();
    await register();
    assert.equal(bridge._client.pid, 42);
    assert.equal(bridge._client.sender, ':1.42');
    await assert.rejects(register, /already running/);
});

test('foreign UID, malformed tokens and a disabled registration in flight fail closed', async () => {
    let {bridge, register} = fixture();
    bridge._credential = async () => 2000;
    await assert.rejects(register, /desktop user/);
    ({bridge, register} = fixture());
    bridge._credential = async () => { bridge._generation++; return 1000; };
    await assert.rejects(register, /desktop user/);
    const invocation = fixture().invocation;
    await assert.rejects(() => bridge._method('Register', invocation, {deep_unpack: () => ['not-a-secret']}), /invalid/);
});

test('a matching secret on a different PID never becomes wallpaper; wrong monitor geometry is rejected', async () => {
    const {bridge} = fixture();
    bridge._client = {pid: 42, secret};
    bridge._rebuild = () => {};
    let altered = 0;
    const actor = {visible: true, hide() { this.visible = false; }, connectObject() {}};
    const window = {
        get_pid: () => 41, get_title: () => `oneMK:${secret}:7:-1920:0:1920:1080`,
        get_compositor_private: () => actor, hide_from_window_list() { altered++; },
        set_type() {}, stick() {}, lower() {}, move_resize_frame() {},
    };
    bridge._adopt(window);
    assert.equal(altered, 0);
    window.get_pid = () => 42;
    window.get_title = () => `oneMK:${secret}:7:-1920:0:800:600`;
    bridge._adopt(window);
    assert.equal(altered, 0);
    window.get_title = () => `oneMK:${secret}:7:-1920:0:1920:1080`;
    bridge._adopt(window);
    assert.equal(altered, 1);
    assert.equal(actor.visible, false);
    bridge._adopt(window);
    assert.equal(altered, 1);
});

test('only the registered sender controls playback, and Stop removes every clone', async () => {
    const {bridge, invocation, register} = fixture();
    await register();
    const stranger = {...invocation, get_sender: () => ':1.99'};
    await assert.rejects(() => bridge._method('Stop', stranger), /registered renderer/);
    let removed = 0, disconnected = 0;
    bridge._windows.set({}, {actor: {disconnectObject() { disconnected++; }}, box: {destroy() { removed++; }}});
    await bridge._method('Stop', invocation);
    assert.equal(removed, 1);
    assert.equal(disconnected, 1, 'no handler of ours stays attached to a window actor');
    assert.equal(bridge._client, null);
    assert.equal(bridge._windows.size, 0);
});

test('Inspect answers anyone; every other method is refused to a stranger, an unregistered caller and for unknown names', async () => {
    const {bridge, invocation, register} = fixture();
    await bridge._method('Inspect', invocation);
    assert.equal(JSON.stringify(invocation.value.value), '["50.1"]');
    const stranger = {...invocation, get_sender: () => ':1.99'};
    for (const method of ['Presented', 'Stop', 'Anything']) {
        await assert.rejects(() => bridge._method(method, stranger), /registered renderer/, `${method} before anyone registered`);
    }
    await register();
    await assert.rejects(() => bridge._method('Presented', stranger), /registered renderer/);
    await assert.rejects(() => bridge._method('Explode', invocation), /Unknown wallpaper method/);
    assert.equal(bridge._client.sender, ':1.42', 'a refused call does not unregister the renderer');
});

test('a renderer that left the bus is replaced; one that is still there blocks a second', async () => {
    const {bridge, invocation, register} = fixture();
    await register();
    const second = {...invocation, get_sender: () => ':1.50'};
    const registerSecond = () => bridge._method('Register', second, {deep_unpack: () => [secret]});
    bridge._credential = async (_sender, method) => method === 'NameHasOwner' ? true : method.endsWith('User') ? 1000 : 50;
    await assert.rejects(registerSecond, /already running/);
    assert.equal(bridge._client.sender, ':1.42');
    bridge._credential = async (sender, method) => method === 'NameHasOwner' ? sender !== ':1.42' : method.endsWith('User') ? 1000 : 50;
    await registerSecond();
    assert.equal(bridge._client.sender, ':1.50');
    assert.equal(bridge._client.pid, 50);
});

test('a renderer that leaves between the credential check and the end of registration is not kept', async () => {
    const {bridge, register} = fixture();
    bridge._credential = async (_sender, method) => method === 'NameHasOwner' ? false : method.endsWith('User') ? 1000 : 42;
    await assert.rejects(register, /left before it registered/);
    assert.equal(bridge._client, null);
});

test('system processes and root are never accepted as the renderer', async () => {
    for (const pid of [0, 1]) {
        const {bridge, register} = fixture();
        bridge._credential = async (_sender, method) => method.endsWith('User') ? 1000 : pid;
        await assert.rejects(register, /desktop user/, `pid ${pid}`);
    }
});

test('tokens are exactly 32 lowercase hex characters', async () => {
    const {bridge, invocation} = fixture();
    for (const token of ['', 'A'.repeat(32), 'a'.repeat(31), 'a'.repeat(33), 'g'.repeat(32), `${'a'.repeat(31)}\n`, ` ${'a'.repeat(31)}`, '../'.repeat(11)]) {
        await assert.rejects(() => bridge._method('Register', invocation, {deep_unpack: () => [token]}), /invalid/, JSON.stringify(token));
        assert.equal(bridge._client, null);
    }
});

function adopting(title, {pid = 42, windows = 0, monitors} = {}) {
    const {bridge, context} = fixture();
    if (monitors) context.Main.layoutManager.monitors = monitors;
    bridge._client = {pid: 42, secret};
    bridge._rebuild = () => {};
    for (let index = 0; index < windows; index++) bridge._windows.set({index}, {});
    let altered = 0;
    const actor = {visible: true, hide() { this.visible = false; }, connectObject() {}};
    const window = {get_pid: () => pid, get_title: () => title, get_compositor_private: () => actor,
        hide_from_window_list() { altered++; }, set_type() {}, stick() {}, lower() {}, move_resize_frame() {}};
    bridge._adopt(window);
    return altered;
}

test('only a window titled with the secret, from the registered process, on a real monitor, is adopted', () => {
    const good = `oneMK:${secret}:7:-1920:0:1920:1080`;
    assert.equal(adopting(good), 1);
    const bad = [
        `oneMK:${secret.replace('0', '1')}:7:-1920:0:1920:1080`, `oneMK:${secret}:7:-1920:0:1920`, `oneMK:${secret}:7:-1920:0:1920:1080:1`,
        `oneMK:${secret}:7:-1920:0:1920.5:1080`, `oneMK:${secret}:7:-1920:0:abc:1080`, `oneMK:${secret}:7:-1920:0:1920:`,
        `oneMK:${secret}:7:-1920:0:1e3:1080`, `oneMK:${secret}:7:-1920:0:99999999999999999999:1080`, `oneMK:${secret}:`, '', `oneMK:${secret}`,
        `oneMK:${secret}:7:-1920:0:1920:1080`.toUpperCase(), `Xx${good}`, `oneMK:${secret}:7:0:0:1920:1080`,
    ];
    for (const title of bad) assert.equal(adopting(title), 0, JSON.stringify(title));
    assert.equal(adopting(good, {pid: 41}), 0, 'another process with the right secret');
    assert.equal(adopting(good, {monitors: []}), 0, 'no monitors');
    assert.equal(adopting(good, {monitors: [{x: 0, y: 0, width: 1920, height: 1080}]}), 0, 'a monitor somewhere else');
    assert.equal(adopting(good, {windows: 16}), 0, 'at most sixteen windows');
    assert.equal(adopting(good, {windows: 15}), 1);
});

test('a window with no title, or none yet, is ignored without an error', () => {
    const {bridge} = fixture();
    bridge._client = {pid: 42, secret};
    for (const title of [null, undefined]) bridge._adopt({get_pid: () => 42, get_title: () => title});
    bridge._client = null;
    bridge._adopt({get_pid: () => 42, get_title: () => `oneMK:${secret}:7:-1920:0:1920:1080`});
});

test('Presented lists only windows that are fresh and showing', async () => {
    const {bridge, invocation, register} = fixture();
    await register();
    const entry = (title, fresh, visible) => ({window: {get_title: () => title}, fresh, box: visible === undefined ? null : {visible}});
    bridge._windows.set({a: 1}, entry('one', true, true));
    bridge._windows.set({a: 2}, entry('two', false, true));
    bridge._windows.set({a: 3}, entry('three', true, false));
    bridge._windows.set({a: 4}, entry('four', true));
    await bridge._method('Presented', invocation);
    assert.equal(JSON.stringify(invocation.value.value), '[["one"]]');
});

test('disable stops the renderer and lets go of every handler, window and name', async () => {
    const {bridge, context, register} = fixture();
    await register();
    const calls = [];
    context.global = {display: {disconnect: id => calls.push(['display', id])}};
    context.Main.layoutManager.disconnect = id => calls.push(['layout', id]);
    context.Gio.DBus = {session: {signal_unsubscribe: id => calls.push(['watch', id])}};
    context.Gio.bus_unown_name = id => calls.push(['name', id]);
    Object.assign(bridge, {_created: 1, _layout: 2, _watch: 3, _owner: 4, _object: {unexport: () => calls.push(['unexport'])}, _layer: {destroy: () => calls.push(['layer'])}});
    bridge._candidates = new Map([[{disconnectObject: owner => calls.push(['candidate', owner === bridge])}, true]]);
    const actor = {disconnectObject: () => calls.push(['actor'])};
    bridge._windows.set({}, {actor, box: {destroy: () => calls.push(['box'])}});
    bridge.disable();
    assert.deepEqual(calls.map(call => call.join(':')).sort(), ['actor', 'box', 'candidate:true', 'display:1', 'layer', 'layout:2', 'name:4', 'unexport', 'watch:3']);
    assert.equal(bridge._client, null);
    assert.equal(bridge._object, null);
    assert.equal(bridge._windows.size, 0);
    assert.equal(bridge._candidates.size, 0);
});
