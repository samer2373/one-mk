//! How fast the key mapper is, and that it never allocates per event.
//! Ignored by default (timing depends on the machine and the build):
//!
//! ```sh
//! cargo test -p one-mk-proto --release --test remap_bench -- --ignored --nocapture
//! ```

use one_mk_proto::remap::{DEFAULT_TAP, Emit, Keymap, Mapper, Output, RuleSpec, Target, What};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

/// Counts allocations made on the current thread while counting is on.
struct Counting;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ON.with(Cell::get) {
            COUNT.with(|c| c.set(c.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ON.with(Cell::get) {
            COUNT.with(|c| c.set(c.get() + 1));
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn rule(from: &str, to: &str) -> RuleSpec {
    RuleSpec { from: from.parse().unwrap(), layer: None, what: What::To(to.parse().unwrap()) }
}

#[test]
#[ignore = "a benchmark: run with --ignored --nocapture, best with --release"]
fn key_mapper_does_over_5m_events_a_second_without_allocating() {
    let specs = vec![
        RuleSpec {
            from: "capslock".parse().unwrap(),
            layer: None,
            what: What::TapHold { tap: "esc".parse().unwrap(), hold: "lctrl".parse().unwrap(), timeout: None },
        },
        rule("lalt", "lmeta"),
        rule("ctrl+h", "backspace"),
        rule("ralt", "layer:nav"),
        RuleSpec { layer: Some("nav".into()), ..rule("j", "down") },
        RuleSpec { from: "f2".parse().unwrap(), layer: None, what: What::To(Output::sequence(&["ctrl+a", "c"]).unwrap()) },
    ];
    let target = Target { os: one_mk_proto::os::LINUX, swap: false };
    let map = Keymap::compile(&specs, target, DEFAULT_TAP).unwrap().map;
    let mut m = Mapper::new(map);
    // Typing: Caps as Ctrl, combos, a layer, plain keys, remapped Alt.
    let script: &[(u32, bool)] = &[
        (58, true), (46, true), (46, false), (35, true), (35, false), (58, false),
        (100, true), (36, true), (36, true), (36, false), (100, false),
        (30, true), (30, false), (56, true), (31, true), (31, false), (56, false),
        (60, true), (60, false), (58, true), (58, false),
    ];
    let now = Instant::now();
    let mut emitted = 0u64;
    let mut sink = |e: Emit| {
        if let Emit::Key { .. } = e {
            emitted += 1;
        }
    };
    // Warm up, then count.
    for &(code, down) in script {
        m.key(code, down, now, &mut sink);
    }
    let rounds = 500_000u32;
    ON.with(|c| c.set(true));
    let start = Instant::now();
    for i in 0..rounds {
        let t = now + Duration::from_micros(u64::from(i));
        for &(code, down) in script {
            m.key(std::hint::black_box(code), down, t, &mut sink);
        }
    }
    let took = start.elapsed();
    ON.with(|c| c.set(false));
    let allocs = COUNT.with(Cell::get);
    let events = f64::from(rounds) * script.len() as f64;
    let rate = events / took.as_secs_f64();
    println!(
        "key mapper: {events} events in {took:?}: {:.1} M events/s, {emitted} emitted, {allocs} allocations",
        rate / 1e6
    );
    assert_eq!(allocs, 0, "the hot path allocated");
    assert!(m.is_idle());
    if cfg!(debug_assertions) {
        println!("(a debug build; the 5 M/s bound is checked in release builds)");
    } else {
        assert!(rate > 5e6, "{rate} events/s");
    }
}
