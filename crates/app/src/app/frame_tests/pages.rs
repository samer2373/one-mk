//! Every page in every state the daemon can put it in, at small, usual and
//! large windows, in both looks and at 100%, 125% and 200% scale: nothing
//! panics, every control has a name and stays inside the sides of the window,
//! no two controls lie on each other, no text is cut off at the side.

use super::*;

const PAGES: [Tab; 7] = [Tab::Desk, Tab::Arrange, Tab::Wallpaper, Tab::Files, Tab::Keys, Tab::Health, Tab::Settings];

/// (width, height, dark, scale): the smallest window, a usual one and a large
/// one, with the looks and scales spread over them so each pair shows up.
const SETUPS: [(f32, f32, bool, f32); 5] = [(820.0, 540.0, false, 1.0), (820.0, 540.0, true, 2.0), (1000.0, 700.0, true, 1.25), (1200.0, 780.0, false, 2.0), (1920.0, 1080.0, true, 1.0)];

fn sweep(label: &str, status: Option<Option<Status>>, pages: &[Tab]) {
    let mut bad = vec![];
    for (w, h, dark, ppp) in SETUPS {
        let mut win = Win::new(w, h, dark, ppp);
        if let Some(status) = status.clone() {
            win.status(status);
        }
        for tab in pages {
            win.app.set_tab(*tab);
            // Settings shows one category at a time: each is a page of its own.
            let cats: Vec<Option<settings_page::Cat>> = if *tab == Tab::Settings { settings_page::Cat::ALL.into_iter().map(Some).collect() } else { vec![None] };
            for cat in cats {
                if let Some(cat) = cat {
                    win.app.settings_cat = cat;
                }
                win.settle();
                // While oneMK is off the window may show another page than asked for.
                let shown = win.tab();
                for p in problems(&win).into_iter().chain(label_not_in_name(&win)) {
                    bad.push(format!("[{label} {w}x{h} {} {ppp}x] {tab:?}/{cat:?} (showing {shown:?}): {p}", if dark { "dark" } else { "light" }));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn while_looking_for_onemk() {
    sweep("loading", None, &[Tab::Desk]);
}

#[test]
fn onemk_is_not_running() {
    sweep("off", Some(None), &PAGES);
}

#[test]
fn one_computer_no_peer() {
    sweep("alone", Some(Some(alone())), &PAGES);
}

#[test]
fn two_computers() {
    sweep("two", Some(Some(two())), &PAGES);
}

#[test]
fn many_computers_and_monitors() {
    sweep("crowded", Some(Some(crowded())), &PAGES);
}

#[test]
fn long_unicode_and_right_to_left_names() {
    sweep("names", Some(Some(hard_names())), &PAGES);
}

/// A banner over the desk (control elsewhere, broadcast, a permission to
/// give, a computer that removed this one) changes the room every page has.
#[test]
fn banners_over_the_desk() {
    for (label, status) in flows::states_with_a_banner() {
        sweep(label, Some(Some(status)), &[Tab::Desk, Tab::Arrange, Tab::Files, Tab::Health]);
    }
}
