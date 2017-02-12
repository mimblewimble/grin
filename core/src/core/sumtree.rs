// Copyright 2016 The Grin Developers
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Sum-Merkle Trees
//!
//! Generic sum-merkle tree. See `doc/merkle.md` for design and motivation for
//! this structure. Most trees in Grin are stored and transmitted as either
//! (a) only a root, or (b) the entire unpruned tree. For these it is sufficient
//! to have a root-calculating function, for which the `TODO` function should
//! be used.
//!
//! The output set structure has much stronger requirements, as it is updated
//! and pruned in place, and needs to be efficiently storable even when it is
//! very sparse.
//!

use core::hash::{Hash, Hashed};
use ser::Writeable;
use std::collections::HashMap;
use std::{self, mem, ops};

/// Generic container to hold prunable data
#[derive(Debug, Clone)]
struct MaybePruned<T, S> {
    data: Option<T>,
    hash: Hash,
    sum: S
}

#[derive(Debug, Clone)]
enum NodeData<T, S> {
    /// Node with 2^n children which are not stored with the tree
    Pruned,
    /// Actual data
    Leaf(T),
    /// Node with 2^n children
    Internal {
        lchild: Box<Node<T, S>>,
        rchild: Box<Node<T, S>>
    },
}

#[derive(Debug, Clone)]
struct Node<T, S> {
    /// Whether or not the node has the maximum 2^n leaves under it.
    /// Leaves count as being full, so partial nodes are always internal.
    full: bool,
    data: NodeData<T, S>,
    hash: Hash,
    sum: S,
    depth: u8
}

impl<T, S: Clone> Node<T, S> {
    /// Get the root hash and sum of the node
    fn root_sum(&self) -> (Hash, S) {
        (self.hash, self.sum.clone())
    }

    fn n_children(&self) -> usize {
        if self.full {
            1 << self.depth
        } else {
            if let NodeData::Internal{ ref lchild, ref rchild } = self.data {
                lchild.n_children() + rchild.n_children()
            } else {
                unreachable!()
            }
        }
    }

}

/// An insertion ordered merkle sum tree.
#[derive(Debug, Clone)]
pub struct SumTree<T: std::hash::Hash + Eq, S> {
    /// Index mapping data to its index in the tree
    index: HashMap<T, usize>,
    /// Tree contents
    root: Option<Node<T, S>>
}

impl<T, S> SumTree<T, S>
    where T: Writeable + std::hash::Hash + Eq + Clone,
          S: ops::Add<Output=S> + std::hash::Hash + Clone + Writeable + Eq
{
    /// Create a new empty tree
    pub fn new() -> SumTree<T, S> {
        SumTree {
            index: HashMap::new(),
            root: None
        }
    }

    /// Accessor for the tree's root
    pub fn root_sum(&self) -> Option<(Hash, S)> {
        self.root.as_ref().map(|node| node.root_sum())
    }

    fn insert_right_of(mut old: Node<T, S>, new: Node<T, S>) -> Node<T, S> {
        assert!(old.depth >= new.depth);

        // If we are inserting next to a full node, make a parent. If we're
        // inserting a tree of equal depth then we get a full node, otherwise
        // we get a partial node. Leaves and pruned data both count as full
        // nodes.
        if old.full {
            let parent_depth = old.depth + 1;
            let parent_sum = old.sum.clone() + new.sum.clone();
            let parent_hash = (parent_depth, &parent_sum, old.hash, new.hash).hash();
            let parent_full = old.depth == new.depth;
            let parent_data = NodeData::Internal {
                lchild: Box::new(old),
                rchild: Box::new(new)
            };

            Node {
                full: parent_full,
                data: parent_data,
                hash: parent_hash,
                sum: parent_sum,
                depth: parent_depth
            }
        // If we are inserting next to a partial node, we should actually be
        // inserting under the node, so we recurse. The right child of a partial
        // node is always another partial node or a leaf.
        } else {
            if let NodeData::Internal{ ref lchild, ref mut rchild } = old.data {
                // Recurse
                let dummy_child = Node { full: true, data: NodeData::Pruned, hash: old.hash, sum: old.sum.clone(), depth: 0 };
                let moved_rchild = mem::replace(&mut **rchild, dummy_child);
                mem::replace(&mut **rchild, SumTree::insert_right_of(moved_rchild, new));
                // Update this node's states to reflect the new right child
                if rchild.full && rchild.depth == old.depth - 1 {
                    old.full = rchild.full;
                }
                old.sum = lchild.sum.clone() + rchild.sum.clone();
                old.hash = (old.depth, &old.sum, lchild.hash, rchild.hash).hash();
            } else {
                unreachable!()
            }
            old
        }
    }

    /// Accessor for number of elements in the tree, not including pruned ones
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Accessor for number of elements in the tree, including pruned ones
    pub fn unpruned_len(&self) -> usize {
        match self.root {
            None => 0,
            Some(ref node) => node.n_children()
        }
    }

    /// Add an element to the tree
    pub fn push(&mut self, elem: T, sum: S) {
        // Compute element hash
        let elem_hash = (0u8, &sum, Hashed::hash(&elem)).hash();

        // Special-case the first element
        if self.root.is_none() {
            self.root = Some(Node {
                full: true,
                data: NodeData::Leaf(elem.clone()),
                hash: elem_hash,
                sum: sum,
                depth: 0
            });
            // TODO panic on double-insert? find a different index?
            self.index.insert(elem, 0);
            return;
        }

        // Next, move the old root out of the structure so that we are allowed to
        // move it. We will move a new root back in at the end of the function
        let old_root = mem::replace(&mut self.root, None).unwrap();

        // Insert into tree, compute new root
        let new_node = Node {
            full: true,
            data: NodeData::Leaf(elem.clone()),
            hash: elem_hash,
            sum: sum,
            depth: 0
        };

        // Put new root in place and record insertion
        let index = old_root.n_children();
        self.root = Some(SumTree::insert_right_of(old_root, new_node));
        // TODO panic on double-insert? find a different index?
        self.index.insert(elem, index);
    }

    /// Determine whether an element exists in the tree
    pub fn contains(&self, elem: &T) -> bool {
        self.index.contains_key(&elem)
    }

    fn prune_recurse(node: &mut Node<T, S>, index: usize) {
        assert!(index < (1 << node.depth));

        if node.depth == 0 {
            node.data = NodeData::Pruned;
        } else {
            let mut prune_me = false;
            match node.data {
                NodeData::Internal { ref mut lchild, ref mut rchild } => {
                    let bit = index & (1 << (node.depth - 1));
                    if bit > 0 {
                        SumTree::prune_recurse(rchild, index - bit);
                    } else {
                        SumTree::prune_recurse(lchild, index);
                    }
                    if let (&NodeData::Pruned, &NodeData::Pruned) = (&lchild.data, &rchild.data) {
                        if node.full {
                            prune_me = true;
                        }
                    }
                }
                NodeData::Pruned => {
                    // Already pruned. Ok.
                }
                NodeData::Leaf(_) => unreachable!()
            }
            if prune_me {
                node.data = NodeData::Pruned;
            }
        }
    }

    /// Removes an element from storage, not affecting the tree
    /// Returns true if the element was actually in the tree
    pub fn prune(&mut self, elem: &T) -> bool {
        let root = match self.root {
            Some(ref mut node) => node,
            None => { return false; }
        };

        match self.index.remove(elem) {
            None => false,
            Some(index) => {
                SumTree::prune_recurse(root, index);
                true
            }
        }
    }

    // TODO push_many to allow bulk updates
}

#[cfg(test)]
mod test {
    use core::hash::Hashed;
    use super::*;

    fn sumtree_create(prune: bool) {
        let mut tree = SumTree::new();

        macro_rules! leaf {
            ($data: expr, $sum: expr) => ({
                (0u8, $sum, $data.hash())
            })
        };

        macro_rules! node {
            ($left: expr, $right: expr) => (
                ($left.0 + 1, $left.1 + $right.1, $left.hash(), $right.hash())
            )
        };

        macro_rules! prune {
            ($prune: expr, $tree: expr, $elem: expr) => {
                if $prune {
                    assert_eq!($tree.len(), 1);
                    $tree.prune($elem);
                    assert_eq!($tree.len(), 0);
                    // double-pruning shouldn't hurt anything
                    $tree.prune($elem);
                    assert_eq!($tree.len(), 0);
                } else {
                    assert_eq!($tree.len(), $tree.unpruned_len());
                }
            }
        };

        assert_eq!(tree.root_sum(), None);
        assert_eq!(tree.len(), 0);
        tree.push(*b"ABC0", 10u16);

        // One element
        let expected = leaf!(b"ABC0", 10u16).hash();
        assert_eq!(tree.root_sum(), Some((expected, 10)));
        assert_eq!(tree.unpruned_len(), 1);
        prune!(prune, tree, b"ABC0");

        // Two elements
        tree.push(*b"ABC1", 25);
        let expected = node!(leaf!(b"ABC0", 10u16),
                             leaf!(b"ABC1", 25u16)
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 35)));
        assert_eq!(tree.unpruned_len(), 2);
        prune!(prune, tree, b"ABC1");

        // Three elements
        tree.push(*b"ABC2", 15);
        let expected = node!(node!(leaf!(b"ABC0", 10u16),
                                   leaf!(b"ABC1", 25u16)),
                             leaf!(b"ABC2", 15u16)
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 50)));
        assert_eq!(tree.unpruned_len(), 3);
        prune!(prune, tree, b"ABC2");

        // Four elements
        tree.push(*b"ABC3", 11);
        let expected = node!(node!(leaf!(b"ABC0", 10u16),
                                   leaf!(b"ABC1", 25u16)),
                             node!(leaf!(b"ABC2", 15u16),
                                   leaf!(b"ABC3", 11u16))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 61)));
        assert_eq!(tree.unpruned_len(), 4);
        prune!(prune, tree, b"ABC3");

        // Five elements
        tree.push(*b"ABC4", 19);
        let expected = node!(node!(node!(leaf!(b"ABC0", 10u16),
                                         leaf!(b"ABC1", 25u16)),
                                   node!(leaf!(b"ABC2", 15u16),
                                         leaf!(b"ABC3", 11u16))),
                             leaf!(b"ABC4", 19u16)
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 80)));
        assert_eq!(tree.unpruned_len(), 5);
        prune!(prune, tree, b"ABC4");

        // Six elements
        tree.push(*b"ABC5", 13);
        let expected = node!(node!(node!(leaf!(b"ABC0", 10u16),
                                         leaf!(b"ABC1", 25u16)),
                                   node!(leaf!(b"ABC2", 15u16),
                                         leaf!(b"ABC3", 11u16))),
                             node!(leaf!(b"ABC4", 19u16),
                                   leaf!(b"ABC5", 13u16))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 93)));
        assert_eq!(tree.unpruned_len(), 6);
        prune!(prune, tree, b"ABC5");

        // Seven elements
        tree.push(*b"ABC6", 30);
        let expected = node!(node!(node!(leaf!(b"ABC0", 10u16),
                                         leaf!(b"ABC1", 25u16)),
                                   node!(leaf!(b"ABC2", 15u16),
                                         leaf!(b"ABC3", 11u16))),
                             node!(node!(leaf!(b"ABC4", 19u16),
                                         leaf!(b"ABC5", 13u16)),
                                   leaf!(b"ABC6", 30u16))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 123)));
        assert_eq!(tree.unpruned_len(), 7);
        prune!(prune, tree, b"ABC6");

        // Eight elements
        tree.push(*b"ABC7", 10000);
        let expected = node!(node!(node!(leaf!(b"ABC0", 10u16),
                                         leaf!(b"ABC1", 25u16)),
                                   node!(leaf!(b"ABC2", 15u16),
                                         leaf!(b"ABC3", 11u16))),
                             node!(node!(leaf!(b"ABC4", 19u16),
                                         leaf!(b"ABC5", 13u16)),
                                   node!(leaf!(b"ABC6", 30u16),
                                         leaf!(b"ABC7", 10000u16)))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 10123)));
        assert_eq!(tree.unpruned_len(), 8);
        prune!(prune, tree, b"ABC7");

        // If we weren't pruning as we went, try pruning everything now
        // and make sure nothing breaks.
        if !prune {
            use rand::{thread_rng, Rng};
            let mut rng = thread_rng();
            let mut elems = [b"ABC0", b"ABC1", b"ABC2", b"ABC3",
                             b"ABC4", b"ABC5", b"ABC6", b"ABC7"];
            rng.shuffle(&mut elems);
            let mut expected_count = 8;
            let expected_root_sum = tree.root_sum();
            for elem in elems.iter() {
                assert_eq!(tree.root_sum(), expected_root_sum);
                assert_eq!(tree.len(), expected_count);
                assert_eq!(tree.unpruned_len(), 8);
                tree.prune(elem);
                expected_count -= 1;
            }
        }
    }

    #[test]
    fn sumtree_test() {
        sumtree_create(false);
        sumtree_create(true);
    }
}



