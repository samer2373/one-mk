// The optional pointer extension, run against stand-ins for GNOME Shell (no desktop is touched).
// The promise: the pointer is hidden only while oneMK asks, and can never stay hidden.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../../extensions/gnome/one-mk-pointer@one-mk.org/extension.js', import.meta.url), 'utf8');

function fixture() {
    const state = {inhibited: 0, maxInhibited: 0, time: 1_000_000, timers: new Map(), nextTimer: 1, handlers: new Map(), nextHandler: 1,
        exported: false, unowned: 0, unsubscribed: 0, watch: null, removed: []};
    const tracker = {
        inhibit_cursor_visibility() { state.inhibited++; state.maxInhibited = Math.max(state.maxInhibited, state.inhibited); },
        uninhibit_cursor_visibility() { state.inhibited--; },
        get_pointer_visible: () => state.inhibited === 0,
        connect(signal, callback) { const id = state.nextHandler++; state.handlers.set(id, {signal, callback}); return id; },
        disconnect(id) { assert.ok(state.handlers.delete(id), 'disconnecting a handler that is not connected'); },
    };
    const context = {
        Extension: class { constructor() { this.dir = {get_child: () => ({load_contents: () => [true, new TextEncoder().encode('<node/>')]})}; } },
        Config: {PACKAGE_VERSION: '50.1'},
        TextDecoder,
        GLib: {
            Variant: class { constructor(signature, value) { this.signature = signature; this.value = Array.from(value); } },
            PRIORITY_DEFAULT: 0, SOURCE_REMOVE: false,
            get_monotonic_time: () => state.time,
            timeout_add_seconds(_priority, seconds, callback) { const id = state.nextTimer++; state.timers.set(id, {seconds, callback}); return id; },
            source_remove(id) { assert.ok(state.timers.delete(id), 'removing a timer that is not there'); },
        },
        Gio: {
            DBusExportedObject: class {
                constructor() { this.handlers = []; }
                connect(_signal, callback) { state.call = callback; }
                export() { state.exported = true; }
                unexport() { state.exported = false; }
            },
            DBusInterfaceInfo: {new_for_xml: () => ({})},
            DBus: {session: {signal_subscribe(_a, _b, _c, _d, _e, _f, callback) { state.watch = callback; return 9; }, signal_unsubscribe() { state.unsubscribed++; }}},
            BusNameOwnerFlags: {NONE: 0}, DBusSignalFlags: {NONE: 0},
            bus_own_name_on_connection: () => 5, bus_unown_name() { state.unowned++; },
        },
        global: {backend: {get_cursor_tracker: () => tracker}},
    };
    vm.createContext(context);
    vm.runInContext(source.replace(/^import .*;$/gm, '').replace('export default class ', 'class ') + '\nglobalThis.Pointer = OneMkPointer;', context);
    const extension = new context.Pointer();
    extension.enable();
    const call = (method, sender = ':1.7') => {
        const invocation = {get_sender: () => sender, return_value(value) { this.value = value; }, return_dbus_error(name, message) { this.error = {name, message}; }};
        state.call({}, method, invocation);
        return invocation;
    };
    const fire = id => { const timer = state.timers.get(id); state.timers.delete(id); return timer.callback(); };
    const moved = () => { for (const handler of [...state.handlers.values()]) if (handler.signal === 'position-invalidated') handler.callback(); };
    return {extension, state, call, fire, moved, tracker};
}

test('Hide hides once however often it is asked, and Show brings the pointer back exactly once', () => {
    const {state, call} = fixture();
    for (let index = 0; index < 5; index++) call('Hide');
    assert.equal(state.inhibited, 1, 'five Hides, one inhibition');
    assert.equal(state.timers.size, 1, 'one hold timer, re-armed each time');
    call('Show');
    assert.equal(state.inhibited, 0);
    assert.equal(state.timers.size, 0);
    call('Show');
    assert.equal(state.inhibited, 0, 'Show when shown does nothing');
});

test('if oneMK stops asking, the pointer comes back by itself after the hold time', () => {
    const {state, call, fire} = fixture();
    call('Hide');
    const [id, timer] = [...state.timers][0];
    assert.equal(timer.seconds, 15);
    fire(id);
    assert.equal(state.inhibited, 0);
    assert.equal(state.timers.size, 0);
});

test('HideUntilMoved hides, ignores the last of the motion that put it there, and shows on the next real motion', () => {
    const {state, call, moved} = fixture();
    call('HideUntilMoved');
    assert.equal(state.inhibited, 1);
    moved();
    assert.equal(state.inhibited, 1, 'motion inside the grace period is the same movement');
    state.time += 399_000;
    moved();
    assert.equal(state.inhibited, 1);
    state.time += 2_000;
    moved();
    assert.equal(state.inhibited, 0, 'the user moved: shown');
    assert.equal(state.handlers.size, 0, 'no motion handler is left connected');
});

test('while oneMK holds the mouse away HideUntilMoved changes nothing, and Hide takes over from it', () => {
    const {state, call} = fixture();
    call('Hide');
    call('HideUntilMoved');
    assert.equal(state.inhibited, 1);
    assert.equal(state.handlers.size, 0, 'no motion handler while held');
    call('Show');
    call('HideUntilMoved');
    assert.equal(state.handlers.size, 1);
    call('HideUntilMoved');
    assert.equal(state.handlers.size, 1, 'asking twice connects one handler');
    assert.equal(state.inhibited, 1);
    call('Hide');
    assert.equal(state.handlers.size, 0, 'Hide stops watching for motion');
    assert.equal(state.inhibited, 1, 'and never inhibits twice');
    call('Show');
    assert.equal(state.inhibited, 0);
});

test('the pointer comes back when the one that asked for it to be hidden leaves the bus, and only then', () => {
    const {state, call} = fixture();
    call('Hide', ':1.7');
    state.watch({}, '', '', '', '', {deep_unpack: () => [':1.99', ':1.99', '']});
    assert.equal(state.inhibited, 1, 'someone else leaving changes nothing');
    state.watch({}, '', '', '', '', {deep_unpack: () => [':1.7', ':1.7', ':1.8']});
    assert.equal(state.inhibited, 1, 'a new owner appearing changes nothing');
    state.watch({}, '', '', '', '', {deep_unpack: () => [':1.7', ':1.7', '']});
    assert.equal(state.inhibited, 0);
    state.watch({}, '', '', '', '', {deep_unpack: () => [':1.7', ':1.7', '']});
    assert.equal(state.inhibited, 0, 'a repeat is harmless');
});

test('disabling the extension always shows the pointer and lets go of everything', () => {
    for (const hide of ['Hide', 'HideUntilMoved']) {
        const {extension, state, call} = fixture();
        call(hide);
        assert.equal(state.inhibited, 1);
        extension.disable();
        assert.equal(state.inhibited, 0, `after ${hide}`);
        assert.equal(state.timers.size, 0);
        assert.equal(state.handlers.size, 0);
        assert.equal(state.exported, false);
        assert.equal(state.unowned, 1);
        assert.equal(state.unsubscribed, 1);
        extension.disable();
        assert.equal(state.inhibited, 0, 'a second disable is harmless');
    }
});

test('unknown methods are refused as errors and change nothing; State and Inspect answer', () => {
    const {state, call} = fixture();
    call('Hide');
    for (const method of ['', 'hide', 'Hide ', 'Reset', 'Show\u0000', '__proto__', 'constructor']) {
        const answer = call(method);
        assert.match(answer.error.message, /unknown method/, JSON.stringify(method));
        assert.equal(answer.error.name, 'org.onemk.Pointer1.Refused');
    }
    assert.equal(state.inhibited, 1);
    assert.equal(JSON.stringify(call('State').value.value), '[true,false]');
    assert.equal(JSON.stringify(call('Inspect').value.value), '["50.1"]');
    call('Show');
    assert.equal(JSON.stringify(call('State').value.value), '[false,true]');
});

test('in every order of calls the pointer is inhibited at most once and is shown after the last Show or a disable', () => {
    let seed = 0x2545f491;
    const next = () => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return seed >>> 0; };
    for (let run = 0; run < 300; run++) {
        const {extension, state, call, fire, moved} = fixture();
        for (let step = 0; step < 60; step++) {
            switch (next() % 7) {
            case 0: call('Hide'); break;
            case 1: call('HideUntilMoved'); break;
            case 2: call('Show'); break;
            case 3: state.time += (next() % 800) * 1000; moved(); break;
            case 4: if (state.timers.size) fire([...state.timers.keys()][0]); break;
            case 5: state.watch({}, '', '', '', '', {deep_unpack: () => [':1.7', ':1.7', '']}); break;
            default: call('State');
            }
            assert.ok(state.inhibited === 0 || state.inhibited === 1, `run ${run} step ${step}: inhibited ${state.inhibited} times`);
            assert.ok(state.timers.size <= 1 && state.handlers.size <= 1);
        }
        extension.disable();
        assert.equal(state.inhibited, 0, `run ${run}: the pointer stayed hidden after disable`);
        assert.equal(state.timers.size + state.handlers.size, 0);
    }
});
