use one_mk_input::localmap_handoff::{Destination, Handoff, Owner, PhysicalKey, Route};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

struct Counted;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counted = Counted;

#[test]
fn decision_allocates_nothing_and_reports_debug_throughput() {
    let mut gate = Handoff::default();
    let ticket = gate.begin(Destination::Local, || {}).unwrap();
    assert!(gate.acknowledge(ticket));
    let key = PhysicalKey {
        down: true, injected: false, shortcut: false, eligible: true, healthy: true, owner: Owner::Unknown,
    };
    assert_eq!(gate.decide(key), Route::Local);
    let iterations = 1_000_000;
    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let started = Instant::now();
    for index in 0..iterations {
        let event = PhysicalKey { healthy: index % 2 == 0, ..key };
        std::hint::black_box(gate.decide(std::hint::black_box(event)));
    }
    let elapsed = started.elapsed();
    COUNTING.store(false, Ordering::Relaxed);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    eprintln!("pure decision only (not native hook latency): {iterations} decisions in {elapsed:?}");
}
