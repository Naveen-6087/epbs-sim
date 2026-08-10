use serde::Serialize;

use crate::config::BuilderStrategy;
use crate::sim::{Proposal, SlotOutcome, SlotRecord};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub slots: u64,
    pub full: u64,
    pub empty: u64,
    pub orphaned: u64,
    pub missed: u64,
    pub full_rate: f64,
    pub self_built: u64,
    pub revealed: u64,
    /// Payloads revealed by builders whose slot still ended up empty or orphaned.
    pub revealed_not_included: u64,
    /// Honest builders that saw a timely block and revealed, yet lost the payload.
    pub honest_reveal_lost: u64,
    /// Bids paid although the payload was not included (withholding is costly).
    pub paid_without_payload: u64,
    /// Canonical blocks whose builder bid was not paid.
    pub unpaid_canonical: u64,
    pub total_bids_gwei: u64,
    pub total_paid_gwei: u64,
    pub mean_ptc_present: f64,
}

pub fn summarize(records: &[SlotRecord]) -> Summary {
    let mut s = Summary { slots: records.len() as u64, ..Summary::default() };
    let mut ptc_sum = 0u64;
    let mut ptc_n = 0u64;
    for r in records {
        match r.outcome {
            SlotOutcome::Full => s.full += 1,
            SlotOutcome::Empty => s.empty += 1,
            SlotOutcome::Orphaned => s.orphaned += 1,
            SlotOutcome::Missed => s.missed += 1,
        }
        s.self_built += u64::from(r.self_built);
        if r.proposal != Proposal::Offline {
            ptc_sum += r.ptc_present as u64;
            ptc_n += 1;
        }
        let included = r.outcome == SlotOutcome::Full;
        if r.revealed && !r.self_built {
            s.revealed += 1;
            if !included {
                s.revealed_not_included += 1;
                if r.strategy == Some(BuilderStrategy::Honest) && r.builder_saw_timely_block {
                    s.honest_reveal_lost += 1;
                }
            }
        }
        s.total_bids_gwei += r.bid_gwei;
        if r.paid {
            s.total_paid_gwei += r.bid_gwei;
            if !included {
                s.paid_without_payload += 1;
            }
        } else if r.bid_gwei > 0 && matches!(r.outcome, SlotOutcome::Full | SlotOutcome::Empty) {
            s.unpaid_canonical += 1;
        }
    }
    s.full_rate = if s.slots == 0 { 0.0 } else { s.full as f64 / s.slots as f64 };
    s.mean_ptc_present = if ptc_n == 0 { 0.0 } else { ptc_sum as f64 / ptc_n as f64 };
    s
}

impl Summary {
    pub fn render(&self, name: &str) -> String {
        let pct = |n: u64| if self.slots == 0 { 0.0 } else { 100.0 * n as f64 / self.slots as f64 };
        let eth = |gwei: u64| gwei as f64 / 1e9;
        format!(
            "scenario: {name}\n\
             slots: {}\n\
             \x20 full      {:>6}  ({:.1}%)\n\
             \x20 empty     {:>6}  ({:.1}%)\n\
             \x20 orphaned  {:>6}  ({:.1}%)\n\
             \x20 missed    {:>6}  ({:.1}%)\n\
             payloads revealed by builders: {} (not included: {}, honest and lost: {})\n\
             bids: {:.3} ETH offered, {:.3} ETH paid; paid without payload: {}, unpaid canonical: {}\n\
             mean PTC 'present' votes: {:.1} / 512\n",
            self.slots,
            self.full,
            pct(self.full),
            self.empty,
            pct(self.empty),
            self.orphaned,
            pct(self.orphaned),
            self.missed,
            pct(self.missed),
            self.revealed,
            self.revealed_not_included,
            self.honest_reveal_lost,
            eth(self.total_bids_gwei),
            eth(self.total_paid_gwei),
            self.paid_without_payload,
            self.unpaid_canonical,
            self.mean_ptc_present,
        )
    }
}
