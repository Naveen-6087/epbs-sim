use epbs_sim::config::{BuilderStrategy, PtcAttack};
use epbs_sim::{Config, SlotOutcome, preset, run, summarize};

fn scenario(name: &str, slots: u64) -> Config {
    let mut c = preset(name).unwrap();
    c.slots = slots;
    c.committee_size = 1024;
    c
}

#[test]
fn honest_low_latency_includes_every_payload() {
    let records = run(&scenario("baseline", 120));
    let s = summarize(&records);
    assert_eq!(s.full, 120, "{s:?}");
    assert_eq!(s.total_paid_gwei, s.total_bids_gwei);
    assert!(records.iter().all(|r| r.built_on_full_parent));
}

#[test]
fn withheld_payloads_leave_empty_slots_but_builders_still_pay() {
    let mut c = scenario("baseline", 150);
    c.builders.withhold_fraction = 0.5;
    let records = run(&c);
    for r in &records {
        if r.strategy == Some(BuilderStrategy::Withhold) {
            assert!(!r.revealed);
            assert_eq!(r.outcome, SlotOutcome::Empty, "slot {}", r.slot);
            // The block was timely and attested, so the unconditional payment applies.
            assert!(r.paid, "slot {}: same-slot votes {}", r.slot, r.same_slot_votes);
        }
    }
    let s = summarize(&records);
    assert!(s.paid_without_payload > 0);
    assert_eq!(s.honest_reveal_lost, 0);
}

#[test]
fn late_reveals_miss_the_payload_deadline() {
    let mut c = scenario("baseline", 120);
    c.builders.late_fraction = 1.0;
    let records = run(&c);
    assert!(records.iter().all(|r| r.revealed && r.outcome == SlotOutcome::Empty), "revealed after 6 s");
    assert!(records.iter().all(|r| r.ptc_present < 256));
}

#[test]
fn late_weak_blocks_are_reorged_and_unpaid() {
    let mut c = scenario("baseline", 200);
    c.proposers.late_fraction = 0.3;
    let records = run(&c);
    let late: Vec<_> = records.iter().filter(|r| r.proposal == epbs_sim::sim::Proposal::Late).collect();
    assert!(!late.is_empty());
    for r in &late {
        // Published at 3.5 s, after the 3 s attestation deadline.
        assert_eq!(r.timely_attesters, 0);
        assert!(!r.paid, "slot {}", r.slot);
        assert!(!r.revealed, "honest builders withhold for late blocks");
    }
    assert!(late.iter().any(|r| r.outcome == SlotOutcome::Orphaned), "on-time successors re-org late heads");
}

#[test]
fn offline_proposers_miss_slots() {
    let mut c = scenario("baseline", 150);
    c.proposers.offline_fraction = 0.2;
    let records = run(&c);
    for r in &records {
        assert_eq!(r.outcome == SlotOutcome::Missed, r.proposal == epbs_sim::sim::Proposal::Offline);
    }
}

#[test]
fn ptc_majority_voting_absent_empties_slots() {
    let mut c = scenario("baseline", 100);
    c.ptc.colluding_fraction = 0.55;
    c.ptc.attack = PtcAttack::VoteAbsent;
    let s = summarize(&run(&c));
    assert_eq!(s.full, 0, "{s:?}");
    assert_eq!(s.honest_reveal_lost, s.revealed, "honest builders revealed and still lost the payload");
}

#[test]
fn ptc_minority_cannot_empty_slots() {
    let mut c = scenario("baseline", 100);
    c.ptc.colluding_fraction = 0.45;
    let s = summarize(&run(&c));
    assert_eq!(s.full, 100, "{s:?}");
}

#[test]
fn ptc_cannot_fake_a_withheld_payload() {
    let mut c = scenario("baseline", 150);
    c.builders.withhold_fraction = 1.0;
    c.ptc.colluding_fraction = 1.0;
    c.ptc.attack = PtcAttack::VotePresent;
    let s = summarize(&run(&c));
    assert_eq!(s.full, 0, "nobody has the payload locally");
}

#[test]
fn unavailable_blobs_empty_slots() {
    let mut c = scenario("baseline", 80);
    c.network.blob_extra_ms = 6_000.0;
    let records = run(&c);
    assert!(records.iter().all(|r| r.outcome == SlotOutcome::Empty && r.ptc_available < 256));
}

#[test]
fn self_builds_pay_nothing() {
    let mut c = scenario("baseline", 60);
    c.builders.self_build_probability = 1.0;
    let records = run(&c);
    assert!(
        records.iter().all(|r| r.self_built && r.bid_gwei == 0 && !r.paid && r.outcome == SlotOutcome::Full)
    );
}

#[test]
fn latency_degrades_inclusion_monotonically() {
    let rates: Vec<f64> = [150.0, 900.0, 2_000.0]
        .into_iter()
        .map(|median| {
            let mut c = scenario("baseline", 150);
            c.network.latency_median_ms = median;
            c.network.latency_sigma = 0.8;
            summarize(&run(&c)).full_rate
        })
        .collect();
    assert!(rates[0] >= rates[1] && rates[1] >= rates[2], "{rates:?}");
    assert!(rates[2] < rates[0]);
}

#[test]
fn runs_are_reproducible_and_seed_dependent() {
    let c = scenario("high-latency", 80);
    assert_eq!(run(&c), run(&c));
    let mut other = c.clone();
    other.seed += 1;
    assert_ne!(run(&c), run(&other));
}
