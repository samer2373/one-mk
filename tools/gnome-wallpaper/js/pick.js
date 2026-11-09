(() => { const Clutter = imports.gi.Clutter;
 const seat = global.stage.context.get_backend().get_default_seat();
 const dev = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
 const t = GLib.get_monotonic_time();
 dev.notify_absolute_motion(t, 900, 600);
 dev.notify_button(t + 1000, Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED);
 dev.notify_button(t + 2000, Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED);
 const picked = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, 900, 600);
 let a = picked, chain = [];
 while (a && chain.length < 8) { const w = a.get_meta_window?.(); chain.push(a.constructor.name + (w ? `("${w.get_title()}")` : '')); a = a.get_parent(); }
 globalThis._spikeDev = dev;
 return JSON.stringify({pickedChain: chain}); })()
