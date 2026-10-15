use std::collections::{BTreeMap, HashMap, HashSet};

use crate::config::{DATA_AVAILABILITY_TIMELY_THRESHOLD, PAYLOAD_TIMELY_THRESHOLD, PROPOSER_SCORE_BOOST_PCT};

pub type Root = u64;
pub const GENESIS: Root = 0;

/// Gloas `PayloadStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PayloadStatus {
    Empty = 0,
    Full = 1,
    Pending = 2,
}

/// Gloas `ForkChoiceNode`: a block root plus which of its payload states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Node {
    pub root: Root,
    pub status: PayloadStatus,
}

impl Node {
    pub fn new(root: Root, status: PayloadStatus) -> Self {
        Self { root, status }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub slot: u64,
    pub parent: Root,
    /// Whether the block's bid builds on its parent's payload
    /// (`get_parent_payload_status == FULL`).
    pub parent_full: bool,
}

/// Aggregated latest messages: `weight` validators attested to `root` at
/// `slot` with `payload_present`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub root: Root,
    pub slot: u64,
    pub payload_present: bool,
    pub weight: u64,
}

/// Counts of non-empty PTC votes for one block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PtcTally {
    pub present: usize,
    pub absent: usize,
    pub available: usize,
    pub unavailable: usize,
}

/// A fork-choice store following the Gloas `get_head` rules. Justification
/// and finality are not modelled: `anchor` plays the justified checkpoint.
#[derive(Clone, Debug)]
pub struct Store {
    pub current_slot: u64,
    pub committee_weight: u64,
    pub anchor: Root,
    blocks: HashMap<Root, Block>,
    children: HashMap<Root, Vec<Root>>,
    messages: Vec<Message>,
    ptc: HashMap<Root, PtcTally>,
    revealed: HashSet<Root>,
    proposer_boost_root: Option<Root>,
    view: View,
}

/// Per-observer differences on top of the shared store.
#[derive(Clone, Copy, Debug, Default)]
struct View {
    /// The newest block's payload as seen by this observer.
    latest_payload: Option<(Root, bool)>,
    /// A block this observer has not received yet.
    hidden: Option<Root>,
}

impl Store {
    pub fn new(committee_weight: u64) -> Self {
        let mut blocks = HashMap::new();
        blocks.insert(GENESIS, Block { slot: 0, parent: GENESIS, parent_full: true });
        let mut revealed = HashSet::new();
        revealed.insert(GENESIS);
        Self {
            current_slot: 0,
            committee_weight,
            anchor: GENESIS,
            blocks,
            children: HashMap::new(),
            messages: Vec::new(),
            ptc: HashMap::new(),
            revealed,
            proposer_boost_root: None,
            view: View::default(),
        }
    }

    pub fn block(&self, root: Root) -> &Block {
        &self.blocks[&root]
    }

    /// `on_tick` at a slot boundary: proposer boost resets.
    pub fn on_slot(&mut self, slot: u64) {
        self.current_slot = slot;
        self.proposer_boost_root = None;
    }

    pub fn add_block(&mut self, root: Root, block: Block) {
        assert!(self.blocks.contains_key(&block.parent), "unknown parent");
        self.blocks.insert(root, block);
        self.children.entry(block.parent).or_default().push(root);
    }

    pub fn set_proposer_boost(&mut self, root: Option<Root>) {
        self.proposer_boost_root = root;
    }

    pub fn add_message(&mut self, m: Message) {
        if m.weight > 0 {
            self.messages.push(m);
        }
    }

    pub fn set_ptc(&mut self, root: Root, tally: PtcTally) {
        self.ptc.insert(root, tally);
    }

    pub fn ptc(&self, root: Root) -> PtcTally {
        self.ptc.get(&root).copied().unwrap_or_default()
    }

    pub fn mark_revealed(&mut self, root: Root) {
        self.revealed.insert(root);
    }

    pub fn is_revealed(&self, root: Root) -> bool {
        self.revealed.contains(&root)
    }

    /// Evaluates `f` from an observer that has (or lacks) the newest payload
    /// and may not have received block `hidden` yet.
    pub fn observe<R>(
        &mut self,
        latest_payload: Option<(Root, bool)>,
        hidden: Option<Root>,
        f: impl FnOnce(&Self) -> R,
    ) -> R {
        self.view = View { latest_payload, hidden };
        let out = f(self);
        self.view = View::default();
        out
    }

    fn known(&self, root: Root) -> bool {
        self.view.hidden != Some(root) && self.blocks.contains_key(&root)
    }

    /// `is_payload_verified`: the observer has the payload.
    pub fn is_payload_verified(&self, root: Root) -> bool {
        match self.view.latest_payload {
            Some((r, seen)) if r == root => seen,
            _ => self.revealed.contains(&root),
        }
    }

    pub fn parent_status(&self, block: &Block) -> PayloadStatus {
        if block.parent_full { PayloadStatus::Full } else { PayloadStatus::Empty }
    }

    pub fn get_ancestor(&self, mut node: Node, slot: u64) -> Node {
        loop {
            let block = &self.blocks[&node.root];
            if block.slot <= slot || node.root == GENESIS {
                return node;
            }
            node = Node::new(block.parent, self.parent_status(block));
        }
    }

    pub fn is_ancestor(&self, node: Node, ancestor: Node) -> bool {
        let a = self.get_ancestor(node, self.blocks[&ancestor.root].slot);
        a.root == ancestor.root && (a.status == ancestor.status || ancestor.status == PayloadStatus::Pending)
    }

    fn supported_node(&self, m: &Message) -> Node {
        let block = &self.blocks[&m.root];
        let status = if block.slot < m.slot {
            if m.payload_present { PayloadStatus::Full } else { PayloadStatus::Empty }
        } else {
            PayloadStatus::Pending
        };
        Node::new(m.root, status)
    }

    pub fn attestation_score(&self, node: Node) -> u64 {
        self.messages
            .iter()
            .filter(|m| self.known(m.root) && self.is_ancestor(self.supported_node(m), node))
            .map(|m| m.weight)
            .sum()
    }

    pub fn payload_timeliness(&self, root: Root, timely: bool) -> bool {
        if !self.is_payload_verified(root) {
            return !timely;
        }
        let t = self.ptc(root);
        (if timely { t.present } else { t.absent }) > PAYLOAD_TIMELY_THRESHOLD
    }

    pub fn payload_data_availability(&self, root: Root, available: bool) -> bool {
        if !self.is_payload_verified(root) {
            return !available;
        }
        let t = self.ptc(root);
        (if available { t.available } else { t.unavailable }) > DATA_AVAILABILITY_TIMELY_THRESHOLD
    }

    fn is_previous_slot_payload_decision(&self, node: Node) -> bool {
        self.blocks[&node.root].slot + 1 == self.current_slot && node.status != PayloadStatus::Pending
    }

    pub fn should_extend_payload(&self, root: Root) -> bool {
        if !self.is_payload_verified(root) {
            return false;
        }
        let boost = self.proposer_boost_root.filter(|r| self.known(*r));
        (self.payload_timeliness(root, true) && self.payload_data_availability(root, true))
            || boost.is_none()
            || boost.is_some_and(|b| self.blocks[&b].parent != root || self.blocks[&b].parent_full)
    }

    fn tiebreaker(&self, node: Node) -> u8 {
        if self.is_previous_slot_payload_decision(node) {
            match node.status {
                PayloadStatus::Empty => 1,
                _ if self.should_extend_payload(node.root) => 2,
                _ => 0,
            }
        } else {
            node.status as u8
        }
    }

    pub fn is_head_weak(&self, root: Root) -> bool {
        let w = self.attestation_score(Node::new(root, PayloadStatus::Pending));
        w * 100 < self.committee_weight * crate::config::REORG_HEAD_WEIGHT_THRESHOLD_PCT
    }

    /// Equivocations are not modelled, so the spec's boost conditions reduce
    /// to "a boosted block exists".
    fn proposer_boost(&self) -> Option<Root> {
        self.proposer_boost_root.filter(|r| self.known(*r))
    }

    /// Attestation scores of every node at once: each message's weight is
    /// pushed up the ancestor path of the node it supports, which yields the
    /// same totals as `attestation_score` for each node.
    fn scores(&self) -> HashMap<Node, u64> {
        let anchor_slot = self.blocks[&self.anchor].slot;
        let mut scores: HashMap<Node, u64> = HashMap::new();
        for m in self.messages.iter().filter(|m| self.known(m.root)) {
            let mut node = self.supported_node(m);
            loop {
                *scores.entry(node).or_default() += m.weight;
                if node.status != PayloadStatus::Pending {
                    *scores.entry(Node::new(node.root, PayloadStatus::Pending)).or_default() += m.weight;
                }
                let block = &self.blocks[&node.root];
                if node.root == GENESIS || block.slot <= anchor_slot {
                    break;
                }
                node = Node::new(block.parent, self.parent_status(block));
            }
        }
        scores
    }

    pub fn get_weight(&self, node: Node) -> u64 {
        self.weight_with(&self.scores(), node)
    }

    fn weight_with(&self, scores: &HashMap<Node, u64>, node: Node) -> u64 {
        if self.is_previous_slot_payload_decision(node) {
            return 0;
        }
        let mut weight = scores.get(&node).copied().unwrap_or(0);
        if let Some(boost) = self.proposer_boost() {
            if self.is_ancestor(Node::new(boost, PayloadStatus::Pending), node) {
                weight += self.committee_weight * PROPOSER_SCORE_BOOST_PCT / 100;
            }
        }
        weight
    }

    fn node_children(&self, node: Node) -> Vec<Node> {
        if node.status == PayloadStatus::Pending {
            let mut out = vec![Node::new(node.root, PayloadStatus::Empty)];
            if self.is_payload_verified(node.root) {
                out.push(Node::new(node.root, PayloadStatus::Full));
            }
            return out;
        }
        self.children
            .get(&node.root)
            .into_iter()
            .flatten()
            .filter(|c| self.known(**c) && self.parent_status(&self.blocks[c]) == node.status)
            .map(|c| Node::new(*c, PayloadStatus::Pending))
            .collect()
    }

    pub fn get_head(&self) -> Node {
        let scores = self.scores();
        let mut head = Node::new(self.anchor, PayloadStatus::Pending);
        loop {
            let children = self.node_children(head);
            let Some(best) = children.into_iter().max_by_key(|c| {
                (self.weight_with(&scores, *c), std::cmp::Reverse(c.root), self.tiebreaker(*c))
            }) else {
                return head;
            };
            head = best;
        }
    }

    /// `should_build_on_full` for a proposer at `slot`.
    pub fn should_build_on_full(&self, head: Node, slot: u64) -> bool {
        assert_ne!(head.status, PayloadStatus::Pending);
        if self.blocks[&head.root].slot + 1 != slot {
            return head.status == PayloadStatus::Full;
        }
        head.status == PayloadStatus::Full
            && !self.payload_timeliness(head.root, false)
            && !self.payload_data_availability(head.root, false)
    }

    pub fn is_parent_strong(&self, root: Root) -> bool {
        let parent = self.blocks[&root].parent;
        let w = self.attestation_score(Node::new(parent, PayloadStatus::Pending));
        w * 100 > self.committee_weight * crate::config::REORG_PARENT_WEIGHT_THRESHOLD_PCT
    }

    /// `get_proposer_head`: re-org a late, weak head from the previous slot by
    /// building on its parent node. FFG and finalization checks are assumed to
    /// pass and equivocations are not modelled.
    pub fn get_proposer_head(&self, head: Node, slot: u64, head_late: bool, proposing_on_time: bool) -> Node {
        let block = self.blocks[&head.root];
        if head.root == self.anchor || head.root == crate::forkchoice::GENESIS {
            return head;
        }
        let parent = self.blocks[&block.parent];
        let single_slot = parent.slot + 1 == block.slot && block.slot + 1 == slot;
        if head_late
            && proposing_on_time
            && single_slot
            && self.is_head_weak(head.root)
            && self.is_parent_strong(head.root)
        {
            Node::new(block.parent, self.parent_status(&block))
        } else {
            head
        }
    }

    /// Drops messages from before `slot` and moves the anchor to the head
    /// chain's block at that slot, keeping `get_head` bounded.
    pub fn prune(&mut self, head: Node, slot: u64) {
        let anchor = self.get_ancestor(head, slot);
        self.anchor = anchor.root;
        let min_slot = self.blocks[&anchor.root].slot;
        self.messages.retain(|m| m.slot >= min_slot);
    }

    /// The chain from `head` back to genesis as (root, payload status).
    pub fn chain(&self, head: Node) -> BTreeMap<u64, (Root, PayloadStatus)> {
        let mut out = BTreeMap::new();
        let mut node = head;
        while node.root != GENESIS {
            let block = self.blocks[&node.root];
            out.insert(block.slot, (node.root, node.status));
            node = Node::new(block.parent, self.parent_status(&block));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PayloadStatus::*;

    const W: u64 = 1000;

    fn store() -> Store {
        Store::new(W)
    }

    fn vote(store: &mut Store, root: Root, slot: u64, present: bool, weight: u64) {
        store.add_message(Message { root, slot, payload_present: present, weight });
    }

    fn tally(present: usize, available: usize) -> PtcTally {
        PtcTally { present, absent: 512 - present, available, unavailable: 512 - available }
    }

    #[test]
    fn pending_children_need_a_verified_payload() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.on_slot(1);
        assert_eq!(s.node_children(Node::new(1, Pending)), vec![Node::new(1, Empty)]);
        s.mark_revealed(1);
        assert_eq!(s.node_children(Node::new(1, Pending)).len(), 2);
        s.observe(Some((1, false)), None, |s| assert_eq!(s.node_children(Node::new(1, Pending)).len(), 1));
    }

    #[test]
    fn ancestry_respects_payload_status() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.add_block(2, Block { slot: 2, parent: 1, parent_full: false });
        let tip = Node::new(2, Pending);
        assert!(s.is_ancestor(tip, Node::new(1, Empty)));
        assert!(!s.is_ancestor(tip, Node::new(1, Full)));
        assert!(s.is_ancestor(tip, Node::new(1, Pending)));
        assert_eq!(s.get_ancestor(tip, 1), Node::new(1, Empty));
    }

    #[test]
    fn previous_slot_payload_decided_by_ptc() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.mark_revealed(1);
        vote(&mut s, 1, 1, false, 900);
        s.on_slot(2);
        // Timely, available payload: FULL wins the tiebreak.
        s.set_ptc(1, tally(400, 400));
        assert_eq!(s.get_head(), Node::new(1, Full));
        assert!(s.should_build_on_full(s.get_head(), 2));
        // A PTC majority saying "absent" makes proposers build on EMPTY.
        s.set_ptc(1, tally(100, 400));
        assert!(!s.should_build_on_full(Node::new(1, Full), 2));
        // Data unavailable likewise.
        s.set_ptc(1, tally(400, 100));
        assert!(!s.should_build_on_full(Node::new(1, Full), 2));
        // Exactly the threshold is not enough to be "timely".
        s.set_ptc(1, tally(256, 400));
        assert!(!s.payload_timeliness(1, true));
        assert!(!s.payload_timeliness(1, false));
    }

    #[test]
    fn unverified_payload_means_empty() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.on_slot(2);
        s.set_ptc(1, tally(500, 500));
        assert_eq!(s.get_head(), Node::new(1, Empty));
        assert!(s.payload_timeliness(1, false), "unseen payload counts as not timely");
    }

    #[test]
    fn proposer_boost_and_weights() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.add_block(2, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.on_slot(1);
        vote(&mut s, 1, 1, false, 300);
        vote(&mut s, 2, 1, false, 350);
        assert_eq!(s.get_head().root, 2);
        s.set_proposer_boost(Some(1));
        assert_eq!(s.get_head().root, 1, "a 40% boost outweighs a 50-vote lead");
        // A hidden block cannot be the head for that observer.
        assert_eq!(s.observe(None, Some(1), |s| s.get_head().root), 2);
    }

    #[test]
    fn later_slot_votes_choose_full_or_empty() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.mark_revealed(1);
        s.on_slot(3);
        vote(&mut s, 1, 2, true, 400);
        vote(&mut s, 1, 2, false, 300);
        assert_eq!(s.get_head(), Node::new(1, Full));
        vote(&mut s, 1, 3, false, 200);
        assert_eq!(s.get_head(), Node::new(1, Empty));
        assert!(!s.should_build_on_full(Node::new(1, Empty), 3));
    }

    #[test]
    fn proposer_reorgs_late_weak_heads() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        s.add_block(2, Block { slot: 2, parent: 1, parent_full: false });
        vote(&mut s, 1, 1, false, 1000);
        vote(&mut s, 1, 2, false, 900);
        vote(&mut s, 2, 2, false, 50);
        s.on_slot(3);
        let head = Node::new(2, Empty);
        assert_eq!(s.get_proposer_head(head, 3, true, true), Node::new(1, Empty));
        assert_eq!(s.get_proposer_head(head, 3, false, true), head, "timely heads are kept");
        assert_eq!(s.get_proposer_head(head, 3, true, false), head, "late proposers do not re-org");
        assert_eq!(s.get_proposer_head(head, 4, true, true), head, "only single-slot re-orgs");
        vote(&mut s, 2, 2, false, 300);
        assert_eq!(s.get_proposer_head(head, 3, true, true), head, "strong heads are kept");
    }

    #[test]
    fn batched_scores_match_direct_scores() {
        let mut s = store();
        let mut parent = GENESIS;
        for slot in 1..=12u64 {
            let root = slot * 2;
            s.add_block(root, Block { slot, parent, parent_full: slot % 3 != 0 });
            if slot % 4 == 0 {
                s.add_block(root + 1, Block { slot, parent, parent_full: slot % 2 == 0 });
                vote(&mut s, root + 1, slot, true, 40);
            }
            s.mark_revealed(root);
            vote(&mut s, root, slot, slot % 2 == 0, 100 + slot);
            vote(&mut s, parent, slot, slot % 3 == 0, 7);
            parent = root;
        }
        s.on_slot(13);
        let scores = s.scores();
        for &root in s.blocks.keys() {
            for status in [Empty, Full, Pending] {
                let node = Node::new(root, status);
                assert_eq!(scores.get(&node).copied().unwrap_or(0), s.attestation_score(node), "{node:?}");
            }
        }
    }

    #[test]
    fn weak_heads_and_pruning() {
        let mut s = store();
        s.add_block(1, Block { slot: 1, parent: GENESIS, parent_full: true });
        vote(&mut s, 1, 1, false, 100);
        assert!(s.is_head_weak(1));
        vote(&mut s, 1, 1, false, 150);
        assert!(!s.is_head_weak(1));
        s.add_block(2, Block { slot: 2, parent: 1, parent_full: false });
        s.prune(Node::new(2, Empty), 1);
        assert_eq!(s.anchor, 1);
        let chain = s.chain(Node::new(2, Full));
        assert_eq!(chain[&1], (1, Empty));
        assert_eq!(chain[&2], (2, Full));
    }
}
