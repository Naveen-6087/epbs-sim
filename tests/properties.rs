use epbs_sim::config::PtcAttack;
use epbs_sim::sim::Proposal;
use epbs_sim::{Config, SlotOutcome, run, summarize};
use proptest::prelude::*;

fn config() -> impl Strategy<Value = Config> {
    (
        any::<u64>(),
        0.0f64..0.5,
        0.0f64..0.5,
        0.0f64..0.3,
        0.0f64..0.3,
        50.0f64..1500.0,
        0.0f64..0.7,
        any::<bool>(),
        0.0f64..0.5,
    )
        .prop_map(|(seed, withhold, late, late_prop, offline, latency, colluding, absent, partition)| {
            let mut c = Config { seed, slots: 40, committee_size: 600, ..Config::default() };
            c.builders.withhold_fraction = withhold;
            c.builders.late_fraction = late;
            c.proposers.late_fraction = late_prop;
            c.proposers.offline_fraction = offline;
            c.network.latency_median_ms = latency;
            c.network.partition_fraction = partition;
            c.network.partition_delay_ms = 2_000.0;
            c.ptc.colluding_fraction = colluding;
            c.ptc.attack = if absent { PtcAttack::VoteAbsent } else { PtcAttack::VotePresent };
            c
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn invariants_hold_for_any_scenario(c in config()) {
        let records = run(&c);
        prop_assert_eq!(records.len() as u64, c.slots);
        let s = summarize(&records);
        prop_assert_eq!(s.full + s.empty + s.orphaned + s.missed, c.slots);
        prop_assert!(s.total_paid_gwei <= s.total_bids_gwei);
        for (i, r) in records.iter().enumerate() {
            prop_assert_eq!(r.slot, i as u64 + 1);
            prop_assert!(r.ptc_present + r.ptc_absent <= 512);
            prop_assert!(r.same_slot_votes as usize <= c.committee_size);
            // A payload can only be canonical if it was revealed.
            if r.outcome == SlotOutcome::Full {
                prop_assert!(r.revealed);
            }
            // Offline proposers produce nothing; everyone else produces a block.
            prop_assert_eq!(r.proposal == Proposal::Offline, r.outcome == SlotOutcome::Missed);
            // Payments: only for canonical blocks with a bid, always when the payload is included.
            if r.paid {
                prop_assert!(r.bid_gwei > 0);
                prop_assert!(matches!(r.outcome, SlotOutcome::Full | SlotOutcome::Empty));
            }
            if r.outcome == SlotOutcome::Full && r.bid_gwei > 0 {
                prop_assert!(r.paid);
            }
        }
    }

    #[test]
    fn same_config_same_result(c in config()) {
        prop_assert_eq!(run(&c), run(&c));
    }
}
