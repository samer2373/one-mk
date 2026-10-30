#[path = "../tests/fixtures/mod.rs"]
mod fixtures;

fn main() {
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).expect("explicit ignored scratch directory required"));
    assert!(directory.components().any(|component| component.as_os_str() == "target"));
    std::fs::create_dir_all(&directory).unwrap();
    for (name, bytes) in [("two.gif", fixtures::gif(2, 2, 2)), ("two.png", fixtures::apng()), ("two.webp", fixtures::webp())] {
        std::fs::write(directory.join(name), bytes).unwrap();
    }
}
