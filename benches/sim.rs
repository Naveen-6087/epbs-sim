use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use epbs_sim::forkchoice::{Block, GENESIS, Message, PtcTally, Store};
use epbs_sim::{preset, run};

fn scenarios(c: &mut Criterion) {
    let mut group = c.benchmark_group("run_100_slots");
    group.sample_size(10);
    for name in ["baseline", "high-latency", "withholding-builders"] {
        let mut cfg = preset(name).unwrap();
        cfg.slots = 100;
        group.bench_with_input(BenchmarkId::from_parameter(name), &cfg, |b, cfg| {
            b.iter(|| run(black_box(cfg)))
        });
    }
    group.finish();
}

fn get_head(c: &mut Criterion) {
    // A 32-slot window with a fork every 4 slots and one message group per slot.
    let mut store = Store::new(2048);
    let mut parent = GENESIS;
    for slot in 1..=32u64 {
        let root = slot * 2;
        store.add_block(root, Block { slot, parent, parent_full: slot % 3 != 0 });
        if slot % 4 == 0 {
            store.add_block(root + 1, Block { slot, parent, parent_full: true });
        }
        store.mark_revealed(root);
        store.set_ptc(root, PtcTally { present: 400, absent: 112, available: 400, unavailable: 112 });
        store.add_message(Message { root, slot, payload_present: slot % 2 == 0, weight: 1800 });
        parent = root;
    }
    store.on_slot(33);
    c.bench_function("get_head/32_slot_window", |b| b.iter(|| black_box(&store).get_head()));
}

criterion_group!(benches, scenarios, get_head);
criterion_main!(benches);
