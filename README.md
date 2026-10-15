# epbs-sim

A deterministic, slot-by-slot simulator for **enshrined proposer-builder
separation** (ePBS, [EIP-7732](https://eips.ethereum.org/EIPS/eip-7732)) as
specified for Ethereum's Gloas fork. It runs the Gloas fork-choice rules over
*pending / empty / full* nodes and models:
- the payload timeliness committee (PTC)
- builder auctions and the unconditional builder payment
- proposer re-orgs
- a configurable network with adversaries

Each run reports what happened to every slot.

```console
$ epbs-sim run --scenario withholding-builders --slots 1000
scenario: withholding-builders
slots: 1000
  full         768  (76.8%)
  empty        232  (23.2%)
  orphaned       0  (0.0%)
  missed         0  (0.0%)
payloads revealed by builders: 768 (not included: 0, honest and lost: 0)
…
```

## Why

ePBS moves the proposer-builder handoff into the protocol. A slot now has
two halves:
1. The proposer publishes a beacon block that commits to a builder's bid.
2. The builder later reveals the execution payload, or doesn't.

A 512-member committee votes on whether the payload arrived in time, and
fork choice decides whether the next block builds on the *full* or the
*empty* version of the previous one. The design has to balance competing
guarantees:
- **Proposers** should get paid even if the builder withholds (the
  *unconditional payment*).
- **Honest builders** that reveal on time should not lose their payload.
- A **dishonest PTC majority** should be able to grief, but not to fake
  availability.

These properties depend on timing: the 3 s, 6 s and 9 s deadlines, network
latency, and how many participants misbehave. That is easier to explore by
simulation than by reading the spec. `epbs-sim` makes each mechanism explicit
and lets you sweep any parameter.

## What is modelled

| Mechanism | Source in the Gloas spec |
|-----------|--------------------------|
| Slot timeline: block at t₀, attestations due 25% (3 s), payload due 50% (6 s), payload attestations due 75% (9 s) | `validator.md` (`*_DUE_BPS`) |
| Fork-choice nodes with `PayloadStatus` *pending / empty / full*; `get_ancestor`, `is_ancestor`, `get_supported_node`, `get_node_children` | `fork-choice.md` |
| `get_head` with the payload-status tiebreaker, `is_previous_slot_payload_decision`, `should_extend_payload` | `fork-choice.md` |
| `payload_timeliness` / `payload_data_availability`: more than 256 of 512 PTC votes, and the payload must be locally available | `fork-choice.md` (`PAYLOAD_TIMELY_THRESHOLD`) |
| Proposer boost (40%), `is_head_weak` (20%), `is_parent_strong` (160%), `get_proposer_head` single-slot re-orgs of late, weak heads | `fork-choice.md` |
| `should_build_on_full`: proposers build on the empty parent if a PTC majority says the payload was late or its data unavailable | `fork-choice.md` |
| Honest PTC votes: `payload_present` if the envelope arrived by 6 s, `blob_data_available` if blobs arrived by 9 s, no vote if no block was seen | `validator.md` |
| Honest builders withhold the payload when their block was not timely | `builder.md` |
| Builder payment: settled when a canonical child builds on the full payload, or when the block's same-slot attestation weight reaches 60% of a slot's stake | `beacon-chain.md` (`settle_builder_payment`, `BUILDER_PAYMENT_THRESHOLD_*`) |

Agents and adversaries:

| Knob | Effect |
|------|--------|
| `builders.withhold_fraction` | Builders that never reveal |
| `builders.late_fraction`, `late_reveal_ms` | Builders that reveal after the payload deadline |
| `builders.reveal_ms`, `bid_fraction`, `value_*`, `self_build_probability` | Honest reveal time, bidding and self-building |
| `proposers.late_fraction`, `late_publish_ms` | Proposers that publish after the attestation deadline |
| `proposers.offline_fraction` | Missed slots |
| `ptc.colluding_fraction`, `ptc.attack` | PTC members that always vote *absent* or always vote *present* |
| `network.latency_*`, `payload_extra_ms`, `blob_extra_ms` | Log-normal propagation delay, plus extra time for payloads and blobs |
| `network.partition_fraction`, `partition_delay_ms` | A slow subset of the network |

### Simplifications (by design)

- **Observer classes, not individual stores.** Each slot's committee
  (default 2,048 attesters, one slot's stake) is split into four classes by
  (received the block before 3 s, has the previous payload). Fork choice runs
  once per class, and each class votes for its head. The next proposer runs
  fork choice on its own view.
- **No FFG.** Justification and finality are not modelled. The fork-choice
  anchor trails the head by 32 slots, and `get_proposer_head`'s FFG and
  finalization checks are assumed to pass.
- **No equivocation.** Duplicate blocks, equivocating attestations and
  slashing are not modelled, so the proposer-boost equivocation exception
  never fires.
- **Message timing.** Votes and PTC messages are assumed to reach the next
  proposer before its slot starts. Only block, payload and blob propagation
  are timed.
- **Auction.** Bids are sealed and reach the proposer in time. Values are
  log-normal per builder and slot.

Within those simplifications, every decision uses the spec's own predicates.
Unit tests check each predicate in isolation, including the threshold edge
(256 votes is not "timely").

## Usage

```sh
cargo install --git https://github.com/Naveen-6087/epbs-sim
```

```
epbs-sim scenarios                                   list presets and how they differ from defaults
epbs-sim config [--scenario NAME]                    print a full JSON config to start from
epbs-sim run [--scenario NAME | --config FILE] [--slots N] [--seed S]
             [--format text|json] [--slots-csv FILE]  one run, optionally one CSV row per slot
epbs-sim sweep --param PATH --values a,b,c [--seeds K] [--scenario | --config] [--slots N]
                                                     vary any field by its dotted path; CSV per run
epbs-sim compare [--slots N] [--seed S] [--format text|json]
                                                     every preset side by side, one row each
epbs-sim tui [--scenario NAME | --config FILE] [--slots N] [--seed S]
                                                     interactive dashboard
```

```sh
# How much inclusion do withholding builders cost? (3 seeds per value, runs in parallel)
epbs-sim sweep --param builders.withhold_fraction --values 0,0.25,0.5,0.75 --slots 1000 --seeds 3

# Your own scenario
epbs-sim config > my.json   # edit it
epbs-sim run --config my.json --slots-csv slots.csv
```

Each slot record contains:
- the proposer's behavior
- the builder and its strategy, and the bid
- whether the block built on a full parent, and whether the payload was revealed
- timely and same-slot attestation counts
- PTC tallies
- the outcome: `full`, `empty`, `orphaned` or `missed`
- whether the bid was paid

`scenarios/mixed-adversaries.json` combines several adversaries.

### Dashboard

`epbs-sim tui` plays a run back slot by slot (300 slots by default) in the terminal:
- a strip with one cell per slot, coloured by outcome
- live outcome shares, payment counters and the mean PTC vote
- the full record of the selected slot: proposer, builder strategy, bid, votes and PTC tally
- sparklines of the rolling full-slot rate and of PTC "present" votes

`↑`/`↓` switch scenario, `←`/`→` step through slots, `space` pauses or replays, `+`/`-` change the
playback speed, `s`/`S` change the seed and `End` jumps to the end of the run. A `--config` file is
added to the scenario list. The dashboard is behind the default `tui` feature; build with
`--no-default-features` to leave out ratatui.

## Results

These tables are 1,000-slot runs with seed 1 (`epbs-sim compare` prints the first one) and the default 2,048-attester
committee, reproducible with the commands above. "Paid w/o payload" counts
bids paid although the payload never became canonical. "Honest & lost"
counts honest builders that saw a timely block and revealed, but whose
payload still ended up empty.

| Scenario | Full | Empty | Orphaned | Missed | Paid w/o payload | Honest & lost | Mean PTC "present" |
|----------|-----:|------:|---------:|-------:|-----------------:|--------------:|-------------------:|
| `baseline` | 100.0% | 0.0% | 0.0% | 0.0% | 0 | 0 | 512 |
| `high-latency` (900 ms median) | 92.1% | 7.6% | 0.3% | 0.0% | 76 | 3 | 386 |
| `withholding-builders` (25%) | 76.8% | 23.2% | 0.0% | 0.0% | 232 | 0 | 393 |
| `late-builders` (25%, reveal at 7 s) | 76.9% | 23.1% | 0.0% | 0.0% | 231 | 0 | 394 |
| `late-proposers` (20%, publish at 3.5 s) | 78.4% | 8.4% | 13.2% | 0.0% | 0 | 0 | 401 |
| `offline-proposers` (10%) | 88.7% | 0.0% | 0.0% | 11.3% | 0 | 0 | 512 |
| `ptc-attack-absent` (55% vote absent) | 0.0% | 100.0% | 0.0% | 0.0% | 1000 | 1000 | 230 |
| `ptc-attack-present` (55% vote present + 25% withholding) | 76.8% | 23.2% | 0.0% | 0.0% | 232 | 0 | 459 |
| `partition` (30% of nodes +3 s) | 100.0% | 0.0% | 0.0% | 0.0% | 0 | 0 | 358 |
| `slow-blobs` (blobs +6 s) | 0.0% | 100.0% | 0.0% | 0.0% | 1000 | 1000 | 512 |

Sweeps (1,000 slots, mean of 3 seeds):

| Withholding builders | 0% | 12.5% | 25% | 50% | 75% |
|----------------------|---:|------:|----:|----:|----:|
| Full slots | 100.0% | 88.3% | 76.5% | 50.3% | 25.6% |

| Median latency | 100 ms | 400 ms | 800 ms | 1,200 ms | 1,600 ms |
|----------------|-------:|-------:|-------:|---------:|---------:|
| Full slots | 100.0% | 100.0% | 98.4% | 92.5% | 83.9% |

What the model shows:

- **Withholding costs the builder, not the proposer.** A withheld payload
  leaves the slot empty, but the block's attestations still meet the 60%
  quorum, so the bid is paid (232 of 232 cases). Late reveals after the 6 s
  deadline are treated the same way.
- **The PTC majority decides full vs. empty.** A colluding majority voting
  *absent* empties every slot even though honest builders revealed on time,
  and those builders still pay: the griefing vector the 50% threshold
  accepts. A colluding minority (45%) changes nothing.
- **The PTC cannot fabricate a payload.** Votes saying *present* do not make
  a withheld payload canonical, because `is_payload_verified` requires local
  availability.
- **Late proposers are re-orged.** Blocks published after the 3 s deadline
  get no same-slot votes. On-time successors re-org them with
  `get_proposer_head` (13.2% orphaned), and orphaned bids are never paid.
- **Latency mostly hurts the payload, not the block.** Up to around 800 ms
  median propagation, almost every payload makes it. Past that, slots go
  empty well before they get orphaned.

## Architecture

```
 Config (JSON / presets) ──► sim::run ──────────────────────────► Vec<SlotRecord> ──► metrics::summarize
                               │  per slot:
                               │   proposer view → get_head → get_proposer_head → should_build_on_full
                               │   auction → block → builder reveal / withhold
                               │   committee classes → get_head per class → aggregated votes
                               │   PTC members → payload/blob timeliness tally
                               ▼
                         forkchoice::Store  (pending/empty/full nodes, scores, boost, PTC tallies)
                               │
                         rng::Rng (xoshiro256**, forked per slot)
```

| Module | Responsibility |
|--------|----------------|
| `config` | Spec constants, scenario configuration, validation, presets |
| `forkchoice` | Gloas fork-choice store and predicates |
| `sim` | The slot loop, agents, network delays and payment rules |
| `metrics` | Summary statistics and text rendering |
| `rng` | A self-contained PRNG (xoshiro256** seeded by SplitMix64) |
| `tui` (binary) | The ratatui dashboard: playback state, key handling and layout |

### Design decisions

- **Determinism.** The PRNG is implemented in the crate and forked per slot.
  The same configuration gives the same result on every platform and
  compiler version, and changing what happens in one slot does not shift the
  randomness of the others.
- **Spec-shaped fork choice.** Function names and logic follow
  `fork-choice.md` so the model can be checked against the spec line by line.
- **Batched weights.** `get_head` computes every node's score in one pass,
  pushing each message's weight up its ancestor path. A test checks it
  against the per-node definition. This made `get_head` about 8.5× faster
  (486 µs → 57 µs on a 32-slot window).
- **Parallel sweeps.** Each run is independent, so `sweep` runs them on
  separate threads and prints them in a fixed order.

## Testing

```sh
cargo test        # 39 tests
```

| Suite | Checks |
|-------|--------|
| Unit tests (16) | PRNG reference values (xoshiro256**, SplitMix64) and distributions; spec timing constants; config validation and presets; fork-choice predicates: pending children need a verified payload, ancestry respects payload status, PTC-decided tiebreaks, the exact 256-vote threshold, unverified payload means empty, proposer boost, later-slot votes, proposer re-orgs (late, on-time, single-slot, weak-head conditions), and batched scores equal per-node scores |
| `tests/scenarios.rs` (12) | Honest runs include every payload and pay every bid; withheld payloads are empty but paid; late reveals miss the deadline; late weak blocks are re-orged and unpaid; offline proposers miss slots; a PTC majority empties slots while a minority cannot; the PTC cannot fake a withheld payload; missing blobs empty slots; self-builds pay nothing; inclusion degrades monotonically with latency; reproducibility |
| `tests/properties.rs` (proptest, 2) | For random adversarial configurations: outcome counts add up, payments never exceed bids, full slots were revealed, included payloads are always paid, unpaid outcomes are never canonical, and runs are deterministic |
| `tests/cli.rs` (6) | Every subcommand including `compare`, JSON and CSV output, config files with overrides, sweep ordering and reproducibility, exit codes |
| `tui` unit tests (3) | Playback reveals slots and stops at the end, key bindings for scenario, seed, cursor and speed, and every panel renders on a test backend |

## Performance

`cargo bench --bench sim` on an Intel Core i5-12500H (release, Criterion
median): 100 slots with 2,048 attesters take 43 ms (`baseline`) to 76 ms
(`high-latency`, where more observer classes differ). `get_head` takes
57 µs on a 32-slot window. A 1,000-slot run takes well under a second.

## Limitations

- See the simplifications above: no FFG, no equivocation, no per-validator
  stores, and no timing for votes or PTC messages.
- Builder economics are deliberately simple (sealed bids, a constant
  bid-to-value ratio). There is no bid-shading strategy or MEV-Boost-style
  relay layer.
- Per-validator stake is uniform, and committees are fresh each slot.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo +1.85 check --all-targets   # MSRV
cargo audit
```

