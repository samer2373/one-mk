import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../../../extensions/gnome/one-mk-wallpaper@one-mk.org/extension.js', import.meta.url), 'utf8');
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

test('enable rejects every unqualified Shell before exporting anything', () => {
    for (const version of ['49.5', '50', '50.0', '50.2', '51.0', '50.1-test']) {
        const {bridge, context} = fixture();
        context.Config.PACKAGE_VERSION = version;
        assert.throws(() => bridge.enable(), /qualified only/);
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
    let removed = 0;
    bridge._windows.set({}, {box: {destroy() { removed++; }}});
    await bridge._method('Stop', invocation);
    assert.equal(removed, 1);
    assert.equal(bridge._client, null);
    assert.equal(bridge._windows.size, 0);
});
