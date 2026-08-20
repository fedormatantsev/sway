//! Stable ids for nodes — design D9, resolved here.
//!
//! D9 says ids are minted once at node creation, but node creation happens in
//! `sway-graph`, which must not know the document format (`sway-graph`'s
//! dependency set has no `ron`, no `serde`, and no document type — see the
//! graph-core API contract §0). So the mapping from a document's stable
//! string id to a session's runtime `NodeId` lives here instead, as data
//! rather than as something minted inside `Graph::insert`.
//!
//! **Decision (not settled by the artifacts): mint lazily, at save time.**
//! Three requirements constrain the choice: an id is assigned once and never
//! changes; deleting one node must not touch any other node's id; ids are
//! unique within a document. A `StableIds` map that is seeded from the file's
//! own id text on [`crate::v4::load`] and is asked to mint any missing id
//! right before [`crate::v4::to_document`] serializes satisfies all three, as
//! long as the map itself persists for the session — which is the caller's
//! job (hold one `StableIds` alongside the `Graph` resource, update it on
//! every load, consult and extend it on every save). This is deliberately
//! **not** `claim.rs`'s per-frame reconcile: nothing here ever looks at a node
//! that already has an id, so an existing id is never revisited, and a
//! deleted node's entry is simply never read again (nothing purges it either
//! — an orphaned entry costs one unused map slot, not a correctness problem,
//! since `to_document` only ever iterates *live* nodes).
//!
//! An `App`-integrated alternative — a per-frame system that mints an id for
//! any node lacking one — is also compatible with everything here; it would
//! just call [`StableIds::ensure`] on every dirty node instead of
//! [`StableIds::mint_missing`] calling it once per save. Nothing in this type
//! favours one driver over the other.

use std::collections::HashMap;

use sway_graph::graph::{Graph, NodeId};

/// The bidirectional map between a document's stable string ids and one
/// session's runtime `NodeId`s.
///
/// Not a `Graph` field and not `Reflect`: `NodeId` is runtime-only (design
/// D9), so this map's lifetime is exactly the session's, seeded by a load and
/// extended by later creations.
#[derive(Debug, Clone, Default)]
pub struct StableIds {
    id_to_node: HashMap<String, NodeId>,
    node_to_id: HashMap<NodeId, String>,
    next: u64,
}

impl StableIds {
    pub fn new() -> Self {
        Self::default()
    }

    /// The node `id` currently names, if any.
    pub fn node_of(&self, id: &str) -> Option<NodeId> {
        self.id_to_node.get(id).copied()
    }

    /// The stable id `node` currently has, if any.
    pub fn id_of(&self, node: NodeId) -> Option<&str> {
        self.node_to_id.get(&node).map(String::as_str)
    }

    /// Records `id` as `node`'s stable identity. Used by [`crate::v4::load`]
    /// once per node entry, for the exact id text the file declared — a
    /// hand-edited id stays put across a reload rather than being replaced by
    /// a freshly minted one.
    pub fn assign(&mut self, id: String, node: NodeId) {
        self.node_to_id.insert(node, id.clone());
        self.id_to_node.insert(id, node);
    }

    /// The stable id for `node`, minting a fresh one if it does not have one
    /// yet. Never changes an id a node already has — the mint path is only
    /// reachable the first time a given `node` is asked for.
    pub fn ensure(&mut self, node: NodeId) -> &str {
        if !self.node_to_id.contains_key(&node) {
            let id = self.mint();
            self.assign(id, node);
        }
        self.node_to_id.get(&node).expect("just inserted above")
    }

    /// Ensures every live node in `graph` has a stable id, minting for any
    /// that do not. `crate::v4::to_document` calls this before it reads a
    /// single node, which is what lets a node created in the running session
    /// — never having gone through [`StableIds::assign`] — still save.
    pub fn mint_missing(&mut self, graph: &Graph) {
        for id in graph.node_ids() {
            self.ensure(id);
        }
    }

    fn mint(&mut self) -> String {
        loop {
            let candidate = format!("node{}", self.next);
            self.next += 1;
            if !self.id_to_node.contains_key(&candidate) {
                return candidate;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sway_graph::graph::Node;

    #[derive(bevy_reflect::Reflect, Default)]
    struct Fixture {
        inlets: (),
        state: (),
        outlets: (),
    }

    #[test]
    fn an_assigned_id_round_trips() {
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Fixture::default()));
        let mut ids = StableIds::new();
        ids.assign("lfoA".to_string(), node);

        assert_eq!(ids.id_of(node), Some("lfoA"));
        assert_eq!(ids.node_of("lfoA"), Some(node));
    }

    #[test]
    fn ensure_mints_once_and_never_again() {
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Fixture::default()));
        let mut ids = StableIds::new();

        let first = ids.ensure(node).to_string();
        let second = ids.ensure(node).to_string();
        assert_eq!(first, second, "asking twice must not re-mint");
    }

    #[test]
    fn minting_never_collides_with_a_loaded_id() {
        let mut graph = Graph::default();
        let loaded = graph.insert(Node::of(Fixture::default()));
        let fresh = graph.insert(Node::of(Fixture::default()));
        let mut ids = StableIds::new();
        // A file that happens to use the exact text a fresh mint would pick.
        ids.assign("node0".to_string(), loaded);

        let minted = ids.ensure(fresh).to_string();
        assert_ne!(minted, "node0");
    }

    #[test]
    fn deleting_a_node_leaves_other_ids_untouched() {
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Fixture::default()));
        let b = graph.insert(Node::of(Fixture::default()));
        let mut ids = StableIds::new();
        ids.assign("a".to_string(), a);
        ids.assign("b".to_string(), b);

        graph.remove(a);
        ids.mint_missing(&graph);

        assert_eq!(ids.id_of(b), Some("b"));
    }
}
