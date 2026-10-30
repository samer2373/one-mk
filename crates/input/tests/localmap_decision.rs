use one_mk_input::localmap_handoff::{Destination, Handoff, Owner, PhysicalKey, Route};
use one_mk_input::localmap_runtime::{Admission, Physical, Rules, Snapshots};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

struct Counted;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if COUNTING.load(Ordering::Relaxed) {
            DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counted = Counted;

#[test]
fn decision_allocates_nothing_and_reports_debug_throughput() {
    let mut gate = Handoff::default();
    let ticket = gate.begin(Destination::Local, || {}).unwrap();
    assert!(gate.acknowledge(ticket));
    let hook = gate.hook_gate();
    let mut admission = Admission::new(hook.clone());
    let mut physical = Physical::default();
    let snapshots = Snapshots::default();
    let snapshot = snapshots.load();
    drop(snapshot);
    let rules = Rules {
        hotkeys: one_mk_input::hotkeys::Bindings::new(vec![
            (one_mk_input::hotkeys::Action::Cycle, "ctrl+alt+right".parse().unwrap()),
            (one_mk_input::hotkeys::Action::FetchClipboard, "ctrl+alt+c".parse().unwrap()),
        ]).unwrap(),
        ..Rules::default()
    };
    let key = PhysicalKey {
        down: true, injected: false, shortcut: false, eligible: true, healthy: true, owner: Owner::Unknown,
    };
    assert_eq!(gate.decide(key), Route::Local);
    let iterations = 1_000_000;
    ALLOCATIONS.store(0, Ordering::Relaxed);
    DEALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let started = Instant::now();
    for destination in [None, Some(Destination::Local), Some(Destination::Remote)] {
        gate.fail(|| {});
        if let Some(destination) = destination {
            let ticket = gate.begin(destination, || {}).unwrap();
            assert!(gate.acknowledge(ticket));
        }
        for index in 0..iterations {
            let event = PhysicalKey {
                owner: [Owner::Unknown, Owner::Os, Owner::Local, Owner::Remote][index % 4],
                down: index & 4 != 0, shortcut: index & 8 != 0, injected: index & 16 != 0,
                healthy: index & 32 != 0, eligible: index & 64 != 0,
            };
            std::hint::black_box(hook.decide(std::hint::black_box(event)));
        }
    }
    let ticket = gate.begin(Destination::Local, || {}).unwrap();
    assert!(gate.acknowledge(ticket));
    let now = Instant::now();
    for _ in 0..100_000 {
        let snapshot = snapshots.load();
        std::hint::black_box(snapshot.count);
        drop(snapshot);
        for code in [29, 56, 106, 46, 30] {
            for down in [true, false] {
                std::hint::black_box(physical.key(PhysicalKey { down, ..key }, code, now, &rules, &mut admission, |_| true));
                while let Some(key) = admission.pop() { admission.acknowledge(key.sequence); }
            }
        }
        for code in [29, 56, 106, 46] {
            std::hint::black_box(physical.key(key, code, now, &rules, &mut admission, |_| true));
            while let Some(key) = admission.pop() { admission.acknowledge(key.sequence); }
        }
        for code in [106, 46, 56, 29] {
            std::hint::black_box(physical.key(PhysicalKey { down: false, ..key }, code, now, &rules, &mut admission, |_| true));
            while let Some(key) = admission.pop() { admission.acknowledge(key.sequence); }
        }
    }
    let elapsed = started.elapsed();
    COUNTING.store(false, Ordering::Relaxed);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    assert_eq!(DEALLOCATIONS.load(Ordering::Relaxed), 0);
    eprintln!("pure decision only (not native hook latency): {iterations} decisions per gate mode in {elapsed:?}");
}
