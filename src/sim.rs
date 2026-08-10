use serde::Serialize;

use crate::config::{
    ATTESTATION_DUE_BPS, BUILDER_PAYMENT_THRESHOLD, BuilderStrategy, Config, PAYLOAD_ATTESTATION_DUE_BPS,
    PAYLOAD_DUE_BPS, PTC_SIZE, PtcAttack, SLOT_MS, due_ms,
};
use crate::forkchoice::{Block, Message, Node, PayloadStatus, PtcTally, Root, Store};
use crate::rng::Rng;

/// Fork-choice look-back used to bound `get_head`, standing in for
/// justification lag.
const ANCHOR_LAG_SLOTS: u64 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotOutcome {
    /// Block and payload are canonical.
    Full,
    /// Block is canonical, its payload is not.
    Empty,
    /// A block was published but is not canonical.
    Orphaned,
    /// No block was published.
    Missed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Proposal {
    OnTime,
    Late,
    Offline,
}

/// Everything recorded about one slot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SlotRecord {
    pub slot: u64,
    pub proposal: Proposal,
    pub self_built: bool,
    pub builder: Option<usize>,
    pub strategy: Option<BuilderStrategy>,
    /// Winning bid, gwei (0 for self-builds).
    pub bid_gwei: u64,
    pub built_on_full_parent: bool,
    pub revealed: bool,
    /// Whether the revealing builder saw the block before the attestation deadline.
    pub builder_saw_timely_block: bool,
    /// Same-slot attesters that saw the block before the attestation deadline.
    pub timely_attesters: usize,
    /// Same-slot attestation weight for this block (what builder payments count).
    pub same_slot_votes: u64,
    pub ptc_present: usize,
    pub ptc_absent: usize,
    pub ptc_available: usize,
    pub outcome: SlotOutcome,
    /// Bid paid by the builder to the proposer.
    pub paid: bool,
}

struct Agents {
    strategies: Vec<BuilderStrategy>,
}

impl Agents {
    fn new(cfg: &Config) -> Self {
        let n = cfg.builders.count;
        let withhold = (cfg.builders.withhold_fraction * n as f64).round() as usize;
        let late = (cfg.builders.late_fraction * n as f64).round() as usize;
        let strategies = (0..n)
            .map(|i| match i {
                i if i < withhold => BuilderStrategy::Withhold,
                i if i < withhold + late => BuilderStrategy::Late,
                _ => BuilderStrategy::Honest,
            })
            .collect();
        Self { strategies }
    }
}

/// Network delay model: log-normal latency plus a fixed penalty for nodes on
/// the slow side of a partition.
struct Network<'a> {
    cfg: &'a crate::config::NetworkConfig,
}

impl Network<'_> {
    fn delay(&self, rng: &mut Rng, slow: bool, extra: f64) -> u64 {
        let d = rng.log_normal(self.cfg.latency_median_ms, self.cfg.latency_sigma) + extra;
        let d = if slow { d + self.cfg.partition_delay_ms } else { d };
        d.max(0.0) as u64
    }
}

/// Runs one scenario. The result is a pure function of the configuration
/// (including its seed).
pub fn run(cfg: &Config) -> Vec<SlotRecord> {
    cfg.validate().expect("invalid configuration");
    let root_rng = Rng::new(cfg.seed);
    let agents = Agents::new(cfg);
    let net = Network { cfg: &cfg.network };
    let committee = cfg.committee_size as u64;
    let mut store = Store::new(committee);

    let attestation_due = due_ms(ATTESTATION_DUE_BPS);
    let payload_due = due_ms(PAYLOAD_DUE_BPS);
    let ptc_due = due_ms(PAYLOAD_ATTESTATION_DUE_BPS);

    let mut records: Vec<SlotRecord> = Vec::with_capacity(cfg.slots as usize + 1);
    // The previous slot's block, and when its payload reached each observer class.
    let mut prev_root: Option<Root> = None;
    let mut prev_payload_ms: Option<u64> = None;
    // When the previous slot's block reached the current slot's proposer.
    let mut prev_block_at_proposer: Option<u64> = None;
    let mut next_root: Root = 1;

    // One extra slot lets a proposer resolve the last slot's payload using its
    // PTC votes before outcomes are classified; it is dropped from the output.
    let simulated = cfg.slots + 1;
    for slot in 1..=simulated {
        let mut rng = root_rng.fork(slot);
        store.on_slot(slot);

        let slow = |rng: &mut Rng| rng.chance(cfg.network.partition_fraction);
        // Whether an observer has the previous slot's payload by `at_ms` into this slot.
        let saw_prev_payload = |rng: &mut Rng, at_ms: u64| -> bool {
            prev_payload_ms.is_some_and(|reveal| {
                let is_slow = slow(rng);
                let arrival = reveal + net.delay(rng, is_slow, cfg.network.payload_extra_ms);
                arrival <= SLOT_MS + at_ms
            })
        };
        let view = |seen: bool| prev_root.map(|r| (r, seen));

        let roll = rng.unit();
        let proposal = if roll < cfg.proposers.offline_fraction {
            Proposal::Offline
        } else if roll < cfg.proposers.offline_fraction + cfg.proposers.late_fraction {
            Proposal::Late
        } else {
            Proposal::OnTime
        };

        // Proposer: head from its own view at the start of the slot, then the
        // optional re-org of a late, weak previous-slot block.
        let proposer_has_payload = saw_prev_payload(&mut rng, 0);
        let head_late = prev_block_at_proposer.is_none_or(|t| t > attestation_due);
        let (head, build_on_full) = store.observe(view(proposer_has_payload), None, |s| {
            let head = s.get_head();
            let head = if head.status == PayloadStatus::Pending {
                Node::new(head.root, PayloadStatus::Empty)
            } else {
                head
            };
            let head = if prev_root == Some(head.root) {
                s.get_proposer_head(head, slot, head_late, proposal == Proposal::OnTime)
            } else {
                head
            };
            (head, s.should_build_on_full(head, slot))
        });

        let mut record = SlotRecord {
            slot,
            proposal,
            self_built: false,
            builder: None,
            strategy: None,
            bid_gwei: 0,
            built_on_full_parent: build_on_full,
            revealed: false,
            builder_saw_timely_block: false,
            timely_attesters: 0,
            same_slot_votes: 0,
            ptc_present: 0,
            ptc_absent: 0,
            ptc_available: 0,
            outcome: SlotOutcome::Missed,
            paid: false,
        };

        let block_root = (proposal != Proposal::Offline).then(|| {
            let r = next_root;
            next_root += 1;
            r
        });

        // Auction: every builder bids a fraction of its private value.
        if block_root.is_some() {
            if cfg.builders.count == 0 || rng.chance(cfg.builders.self_build_probability) {
                record.self_built = true;
            } else {
                let (winner, bid) = (0..cfg.builders.count)
                    .map(|b| {
                        let value = rng.log_normal(cfg.builders.value_median_gwei, cfg.builders.value_sigma);
                        (b, (value * cfg.builders.bid_fraction) as u64)
                    })
                    .max_by_key(|(b, bid)| (*bid, std::cmp::Reverse(*b)))
                    .expect("at least one builder");
                record.builder = Some(winner);
                record.strategy = Some(agents.strategies[winner]);
                record.bid_gwei = bid;
            }
        }

        let publish_ms = match proposal {
            Proposal::OnTime => (rng.unit() * 200.0) as u64,
            Proposal::Late => cfg.proposers.late_publish_ms,
            Proposal::Offline => 0,
        };

        if let Some(root) = block_root {
            store.add_block(root, Block { slot, parent: head.root, parent_full: build_on_full });
        }

        // Payload reveal: builders follow their strategy; self-builders behave honestly.
        let mut reveal_ms = None;
        if block_root.is_some() {
            let builder_arrival = publish_ms + net.delay(&mut rng, false, 0.0);
            record.builder_saw_timely_block = builder_arrival <= attestation_due;
            let strategy = record.strategy.unwrap_or(BuilderStrategy::Honest);
            reveal_ms = match strategy {
                BuilderStrategy::Honest if record.builder_saw_timely_block => {
                    Some(cfg.builders.reveal_ms.max(builder_arrival))
                }
                BuilderStrategy::Honest | BuilderStrategy::Withhold => None,
                BuilderStrategy::Late => Some(cfg.builders.late_reveal_ms.max(builder_arrival)),
            };
            record.revealed = reveal_ms.is_some();
        }

        // Attesters: four observer classes by (has the new block in time,
        // has the previous payload), each voting for its own head.
        let mut classes = [[0u64; 2]; 2];
        let mut block_seen_at = Vec::with_capacity(cfg.committee_size);
        for _ in 0..cfg.committee_size {
            let is_slow = slow(&mut rng);
            let arrival = block_root.map(|_| publish_ms + net.delay(&mut rng, is_slow, 0.0));
            let timely = arrival.is_some_and(|a| a <= attestation_due);
            let has_prev = saw_prev_payload(&mut rng, attestation_due);
            classes[usize::from(timely)][usize::from(has_prev)] += 1;
            block_seen_at.push((arrival, is_slow));
        }
        record.timely_attesters = (classes[1][0] + classes[1][1]) as usize;

        let mut messages = Vec::new();
        for (timely, row) in classes.iter().enumerate() {
            for (has_prev, &count) in row.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let sees_block = timely == 1 && block_root.is_some();
                let hidden = if sees_block { None } else { block_root };
                store.set_proposer_boost(if sees_block { block_root } else { None });
                let head = store.observe(view(has_prev == 1), hidden, |s| s.get_head());
                let block_slot = store.block(head.root).slot;
                messages.push(Message {
                    root: head.root,
                    slot,
                    payload_present: block_slot < slot && head.status == PayloadStatus::Full,
                    weight: count,
                });
            }
        }
        store.set_proposer_boost(None);
        record.same_slot_votes =
            messages.iter().filter(|m| Some(m.root) == block_root).map(|m| m.weight).sum();
        for m in messages {
            store.add_message(m);
        }

        // Payload timeliness committee: 512 members of this slot's committee.
        if let Some(root) = block_root {
            let mut tally = PtcTally::default();
            let colluders = (cfg.ptc.colluding_fraction * PTC_SIZE as f64).round() as usize;
            for (i, member) in rng.sample(cfg.committee_size, PTC_SIZE).into_iter().enumerate() {
                let (arrival, is_slow) = block_seen_at[member];
                if i < colluders {
                    match cfg.ptc.attack {
                        PtcAttack::VoteAbsent => {
                            tally.absent += 1;
                            tally.unavailable += 1;
                        }
                        PtcAttack::VotePresent => {
                            tally.present += 1;
                            tally.available += 1;
                        }
                    }
                    continue;
                }
                if arrival.is_none_or(|a| a > ptc_due) {
                    continue; // no block seen: no vote
                }
                let payload_at =
                    reveal_ms.map(|r| r + net.delay(&mut rng, is_slow, cfg.network.payload_extra_ms));
                let blobs_at = reveal_ms.map(|r| r + net.delay(&mut rng, is_slow, cfg.network.blob_extra_ms));
                if payload_at.is_some_and(|t| t <= payload_due) {
                    tally.present += 1;
                } else {
                    tally.absent += 1;
                }
                if blobs_at.is_some_and(|t| t <= ptc_due) {
                    tally.available += 1;
                } else {
                    tally.unavailable += 1;
                }
            }
            store.set_ptc(root, tally);
            record.ptc_present = tally.present;
            record.ptc_absent = tally.absent;
            record.ptc_available = tally.available;
            if record.revealed {
                store.mark_revealed(root);
            }
        }

        prev_root = block_root;
        prev_payload_ms = reveal_ms;
        let next_proposer_slow = slow(&mut rng);
        prev_block_at_proposer =
            block_root.map(|_| publish_ms + net.delay(&mut rng, next_proposer_slow, 0.0));

        if slot > ANCHOR_LAG_SLOTS {
            let head = store.get_head();
            store.prune(head, slot - ANCHOR_LAG_SLOTS);
        }
        records.push(record);
    }

    records.truncate(cfg.slots as usize);
    // Final view, one slot later: classify every slot against the canonical chain.
    store.on_slot(simulated + 1);
    let final_head = store.get_head();
    let final_head = if final_head.status == PayloadStatus::Pending {
        Node::new(final_head.root, PayloadStatus::Empty)
    } else {
        final_head
    };
    let chain = store.chain(final_head);
    let quorum = committee * BUILDER_PAYMENT_THRESHOLD.0 / BUILDER_PAYMENT_THRESHOLD.1;
    for record in &mut records {
        record.outcome = match (record.proposal, chain.get(&record.slot)) {
            (Proposal::Offline, _) => SlotOutcome::Missed,
            (_, None) => SlotOutcome::Orphaned,
            (_, Some((_, PayloadStatus::Full))) => SlotOutcome::Full,
            (_, Some(_)) => SlotOutcome::Empty,
        };
        // Bids are paid when the payload is applied by a canonical child, or
        // when the block gathered a same-slot attestation quorum.
        record.paid = record.bid_gwei > 0
            && match record.outcome {
                SlotOutcome::Full => true,
                SlotOutcome::Empty => record.same_slot_votes >= quorum,
                _ => false,
            };
    }
    records
}
