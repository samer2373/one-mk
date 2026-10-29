//! `cargo run -p one-mk-ctl --example packaging-files -- ROOT [BINDIR]`
//!
//! Writes the systemd user unit and the desktop entries a Linux package
//! ships under ROOT (see `one_mk_ctl::install::packaged_files`), for the
//! program installed in BINDIR (default `/usr/bin`).

use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(root) = args.next().map(PathBuf::from) else {
        eprintln!("usage: packaging-files ROOT [BINDIR]");
        std::process::exit(2);
    };
    let bindir = args.next().map_or_else(|| PathBuf::from("/usr/bin"), PathBuf::from);
    for (rel, text) in one_mk_ctl::install::packaged_files(&bindir) {
        let path = root.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).expect("creating a folder");
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        println!("{}", path.display());
    }
}
