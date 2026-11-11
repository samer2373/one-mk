//! Who makes oneMK: Bigfoot Technology, a software studio in Amman, Jordan.
//! Every mention of it (the window's About card, the tray menu) is a link to
//! its site, through the one address here.

use std::path::PathBuf;

/// The maker's name, as it is written.
pub const MAKER: &str = "Bigfoot Technology";
/// Its line.
pub const MAKER_LINE: &str = "Big software. Bigger footprint.";
/// Where every mention of it goes.
pub const MAKER_URL: &str = "https://bigfootjo.com/";
/// The site's name, as a button says it.
pub const MAKER_SITE: &str = "bigfootjo.com";

/// The program (and its arguments) that opens `url` in the default browser
/// on this system, for parts of oneMK without a window of their own (the
/// tray). The window opens links through egui instead.
pub fn opener(url: &str) -> (PathBuf, Vec<String>) {
    if cfg!(target_os = "macos") {
        (PathBuf::from("/usr/bin/open"), vec![url.to_owned()])
    } else if cfg!(windows) {
        // No console window, unlike `cmd /c start`.
        (PathBuf::from("rundll32.exe"), vec!["url.dll,FileProtocolHandler".to_owned(), url.to_owned()])
    } else {
        (PathBuf::from("xdg-open"), vec![url.to_owned()])
    }
}

/// Opens the maker's site in the default browser.
pub fn open_site() -> std::io::Result<()> {
    let (exe, args) = opener(MAKER_URL);
    crate::proc::spawn_detached(&exe, &args, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mention_links_to_the_makers_site() {
        assert_eq!(MAKER_URL, "https://bigfootjo.com/");
        assert_eq!(MAKER, "Bigfoot Technology");
        assert!(MAKER_URL.contains(MAKER_SITE));
        let (exe, args) = opener(MAKER_URL);
        assert!(!exe.as_os_str().is_empty());
        assert_eq!(args.last().map(String::as_str), Some(MAKER_URL), "the address is passed as it is");
    }
}
