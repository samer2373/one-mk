(() => {
  const out = [];
  for (const c of global.window_group.get_children()) {
    let d = c.constructor.name;
    if (c === Main.layoutManager._backgroundGroup)
      d += ' [Shell _backgroundGroup: ' + c.get_children().map(x => x.name || x.constructor.name).join(', ') + ']';
    const w = c.get_meta_window?.();
    if (w) d += ` "${w.get_title()}" type=${w.get_window_type()} skip_taskbar=${w.skip_taskbar} visible=${c.visible} opacity=${c.opacity}`;
    out.push(d);
  }
  return out.join('\n');
})()
