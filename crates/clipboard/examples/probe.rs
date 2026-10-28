//! Manual check of system clipboard access from a background process.
//!
//! ```text
//! cargo run -p one-mk-clipboard --example probe -- get
//! cargo run -p one-mk-clipboard --example probe -- set "hello" [seconds]
//! ```

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cb = arboard::Clipboard::new().expect("open clipboard");
    match args.first().map(String::as_str) {
        Some("set") => {
            let text = args.get(1).cloned().unwrap_or_else(|| "one-mk probe".into());
            cb.set_text(text).expect("set");
            let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            println!("set; holding for {secs}s");
            std::thread::sleep(std::time::Duration::from_secs(secs));
        }
        _ => {
            println!("text:  {:?}", cb.get_text());
            println!("html:  {:?}", cb.get().html().map(|h| h.len()));
            println!("image: {:?}", cb.get_image().map(|i| (i.width, i.height)));
        }
    }
}
