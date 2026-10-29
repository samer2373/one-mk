//! Windows and macOS: the system tray / menu bar through `tray-icon`, with
//! a `tao` event loop that sleeps until the system has something for it.
//!
//! These platforms give no "menu is about to open" callback, so the menu is
//! refreshed when the pointer enters or clicks the icon (Windows reports
//! both before the user can pick anything) and, on macOS, where the menu is
//! drawn by the system without any event, also every 5 seconds while the
//! daemon is running. Nothing else wakes the process.

use crate::core::Ctx;
use crate::icon::{self, IconState};
use crate::model::{self, Action, Entry, Snapshot};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

const SLOW_REFRESH: Option<Duration> = if cfg!(target_os = "macos") { Some(Duration::from_secs(5)) } else { None };
/// Hovering and clicking send several events in a row; one look is enough.
const MIN_GAP: Duration = Duration::from_millis(800);

enum Ev {
    Menu(MenuEvent),
    /// The pointer touched the icon: look at the daemon.
    Touched,
    Snapshot(Box<Snapshot>),
}

fn icon(state: IconState) -> Icon {
    Icon::from_rgba(icon::rgba(32, state), 32, 32).expect("a 32x32 RGBA buffer")
}

/// Sets the icon. On macOS the plain states are a template image (the menu
/// bar recolours it); the amber ones keep their colour.
fn set_icon(tray: &TrayIcon, state: IconState) {
    #[cfg(target_os = "macos")]
    if let Some(t) = icon::template_rgba(state) {
        let t = Icon::from_rgba(t, 32, 32).expect("a 32x32 RGBA buffer");
        let _ = tray.set_icon_templated(Some(t));
        return;
    }
    let _ = tray.set_icon(Some(icon(state)));
}

/// Builds the native menu, and remembers which item does what.
fn build(entries: &[Entry], actions: &mut HashMap<MenuId, Action>) -> Menu {
    let menu = Menu::new();
    for e in entries {
        add(&menu, e, actions);
    }
    menu
}

trait Parent {
    fn push(&self, item: &dyn tray_icon::menu::IsMenuItem);
}
impl Parent for Menu {
    fn push(&self, item: &dyn tray_icon::menu::IsMenuItem) {
        let _ = self.append(item);
    }
}
impl Parent for Submenu {
    fn push(&self, item: &dyn tray_icon::menu::IsMenuItem) {
        let _ = self.append(item);
    }
}

fn add(parent: &dyn Parent, e: &Entry, actions: &mut HashMap<MenuId, Action>) {
    match e {
        Entry::Separator => parent.push(&PredefinedMenuItem::separator()),
        Entry::Label(l) => parent.push(&MenuItem::new(l, false, None)),
        Entry::Item { label, action, checked: None } => {
            let i = MenuItem::new(label, true, None);
            actions.insert(i.id().clone(), action.clone());
            parent.push(&i);
        }
        Entry::Item { label, action, checked: Some(c) } => {
            let i = CheckMenuItem::new(label, true, *c, None);
            actions.insert(i.id().clone(), action.clone());
            parent.push(&i);
        }
        Entry::Sub { label, items } => {
            let s = Submenu::new(label, true);
            for i in items {
                add(&s, i, actions);
            }
            parent.push(&s);
        }
    }
}

/// Looks at the daemon on another thread, so the menu never waits for it.
fn refresh(ctx: &Arc<Ctx>, proxy: &EventLoopProxy<Ev>, after: Option<Action>) {
    let (ctx, proxy) = (ctx.clone(), proxy.clone());
    std::thread::spawn(move || {
        if let Some(a) = after {
            ctx.perform(&a);
        }
        let _ = proxy.send_event(Ev::Snapshot(Box::new(ctx.snapshot())));
    });
}

pub fn run(ctx: Arc<Ctx>, start_daemon: bool) -> anyhow::Result<()> {
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    // A menu bar item only: no Dock icon.
    #[cfg(target_os = "macos")]
    tao::platform::macos::EventLoopExtMacOS::set_activation_policy(&mut event_loop, tao::platform::macos::ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();

    let p = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e| drop(p.send_event(Ev::Menu(e)))));
    let p = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |e| {
        if matches!(e, TrayIconEvent::Enter { .. } | TrayIconEvent::Click { .. }) {
            let _ = p.send_event(Ev::Touched);
        }
    }));

    let mut tray: Option<TrayIcon> = None;
    let mut actions = HashMap::new();
    let mut snap = Snapshot::default();
    let mut last_look = Instant::now();

    event_loop.run(move |event, _, flow| {
        match event {
            Event::NewEvents(StartCause::Init) => {
                let first = ctx.snapshot();
                let menu = build(&model::menu(&first), &mut actions);
                tray = TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_icon(icon(first.icon_state()))
                    .with_tooltip(first.headline())
                    .build()
                    .map_err(|e| eprintln!("one-mk-tray: cannot create the tray icon: {e}"))
                    .ok();
                #[cfg(target_os = "macos")]
                if let Some(t) = &tray {
                    set_icon(t, first.icon_state());
                }
                if tray.is_none() {
                    *flow = ControlFlow::Exit;
                    return;
                }
                snap = first;
                last_look = Instant::now();
                if start_daemon {
                    refresh(&ctx, &proxy, Some(Action::StartDaemon));
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => refresh(&ctx, &proxy, None),
            Event::UserEvent(Ev::Touched) => {
                if last_look.elapsed() > MIN_GAP {
                    last_look = Instant::now();
                    refresh(&ctx, &proxy, None);
                }
            }
            Event::UserEvent(Ev::Menu(e)) => match actions.get(e.id()) {
                Some(Action::Quit) => {
                    tray = None;
                    *flow = ControlFlow::Exit;
                    return;
                }
                Some(a) => {
                    last_look = Instant::now();
                    refresh(&ctx, &proxy, Some(a.clone()));
                }
                None => {}
            },
            Event::UserEvent(Ev::Snapshot(s)) => {
                let s = *s;
                last_look = Instant::now();
                if let (Some(t), true) = (&tray, s != snap) {
                    actions.clear();
                    t.set_menu(Some(Box::new(build(&model::menu(&s), &mut actions))));
                    set_icon(t, s.icon_state());
                    let _ = t.set_tooltip(Some(s.headline()));
                }
                snap = s;
            }
            _ => {}
        }
        let next = SLOW_REFRESH.filter(|_| snap.running()).map(|d| Instant::now() + d);
        *flow = next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
    });
}
