//! Linux: a StatusNotifierItem over D-Bus with the `ksni` crate (pure Rust,
//! no GTK, no appindicator library). It is what KDE Plasma shows natively
//! and what GNOME shows with the AppIndicator extension. It runs on a
//! thread that sleeps until D-Bus delivers something, and the host asks us
//! to refresh (`AboutToShow`) every time the menu is opened: no polling.

use crate::core::Ctx;
use crate::icon::{self, IconState};
use crate::model::{self, Entry, Snapshot};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip, Tray, blocking::TrayMethods};
use std::sync::Arc;

struct OneMk {
    ctx: Arc<Ctx>,
    snap: Snapshot,
}

/// ARGB32 in network byte order, as the specification wants it.
fn pixmap(size: u32, state: IconState) -> Icon {
    let data = icon::rgba(size, state).as_chunks::<4>().0.iter().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
    Icon { width: size as i32, height: size as i32, data }
}

fn item(e: &Entry) -> MenuItem<OneMk> {
    match e {
        Entry::Separator => MenuItem::Separator,
        Entry::Label(l) => StandardItem { label: l.clone(), enabled: false, ..Default::default() }.into(),
        Entry::Item { label, action, checked } => {
            let action = action.clone();
            let run = move |t: &mut OneMk| {
                if action == model::Action::Quit {
                    std::process::exit(0);
                }
                t.ctx.perform(&action);
                t.snap = t.ctx.snapshot();
            };
            match checked {
                Some(c) => CheckmarkItem { label: label.clone(), checked: *c, activate: Box::new(run), ..Default::default() }.into(),
                None => StandardItem { label: label.clone(), activate: Box::new(run), ..Default::default() }.into(),
            }
        }
        Entry::Sub { label, items } => SubMenu { label: label.clone(), submenu: items.iter().map(item).collect(), ..Default::default() }.into(),
    }
}

impl Tray for OneMk {
    const MENU_ON_ACTIVATE: bool = false;

    fn id(&self) -> String {
        "one-mk".into()
    }

    fn title(&self) -> String {
        "one-mk".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let st = self.snap.icon_state();
        icon::SIZES.into_iter().map(|size| pixmap(size, st)).collect()
    }

    /// A left click opens the settings window; the menu is on right click.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.ctx.perform(&model::Action::OpenApp);
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "one-mk".into(), description: self.snap.headline(), ..Default::default() }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        model::menu(&self.snap).iter().map(item).collect()
    }

    /// The menu is being opened: look at the daemon now.
    fn menu_about_to_show(&mut self) {
        self.snap = self.ctx.snapshot();
    }
}

pub fn run(ctx: Arc<Ctx>, start_daemon: bool) -> anyhow::Result<()> {
    if start_daemon {
        let ctx = ctx.clone();
        std::thread::spawn(move || drop(ctx.start_daemon()));
    }
    let snap = ctx.snapshot();
    // `assume_sni_available`: at login the tray may start before the desktop
    // has its notification area; the icon then appears when it does.
    let _handle = OneMk { ctx, snap }.assume_sni_available(true).spawn()?;
    loop {
        std::thread::park();
    }
}
