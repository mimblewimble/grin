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
//! to have a root-calculating function, for which the `compute_root` function should
//! be used.
//!
//! The output set structure has much stronger requirements, as it is updated
//! and pruned in place, and needs to be efficiently storable even when it is
//! very sparse.
//!

use core::hash::{Hash, Hashed};
use ser::{self, Readable, Reader, Writeable, Writer};
use std::collections::HashMap;
use std::{self, mem, ops};

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

    fn replace_recurse(node: &mut Node<T, S>, index: usize, new_elem: T, new_sum: S) {
        assert!(index < (1 << node.depth));

        if node.depth == 0 {
            node.hash = (0u8, &new_sum, Hashed::hash(&new_elem)).hash();
            node.sum = new_sum;
            node.data = NodeData::Leaf(new_elem);
        } else {
            match node.data {
                NodeData::Internal { ref mut lchild, ref mut rchild } => {
                    let bit = index & (1 << (node.depth - 1));
                    if bit > 0 {
                        SumTree::replace_recurse(rchild, index - bit, new_elem, new_sum);
                    } else {
                        SumTree::replace_recurse(lchild, index, new_elem, new_sum);
                    }
                    node.sum = lchild.sum.clone() + rchild.sum.clone();
                    node.hash = (node.depth, &node.sum, lchild.hash, rchild.hash).hash();
                }
                // Pruned data would not have been in the index
                NodeData::Pruned => unreachable!(),
                NodeData::Leaf(_) => unreachable!()
            }
        }
    }

    /// Replaces an element in the tree. Returns true if the element existed
    /// and was replaced.
    pub fn replace(&mut self, elem: &T, new_elem: T, new_sum: S) -> bool {
        let root = match self.root {
            Some(ref mut node) => node,
            None => { return false; }
        };

        match self.index.remove(elem) {
            None => false,
            Some(index) => {
                SumTree::replace_recurse(root, index, new_elem.clone(), new_sum);
                self.index.insert(new_elem, index);
                true
            }
        }
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

// A SumTree is encoded as follows: an empty tree is the single byte 0x00.
// An nonempty tree is encoded recursively by encoding its root node. Each
// node is encoded as follows:
//   flag: two bits, 01 for partial, 10 for full, 11 for pruned
//         00 is reserved so that the 0 byte can uniquely specify an empty tree
//  depth: six bits, zero indicates a leaf
//   hash: 32 bytes
//    sum: <length of sum encoding>
//
// For a leaf, this is followed by an encoding of the element. For an
// internal node, the left child is encoded followed by the right child.
// For a pruned internal node, it is followed by nothing.
//
impl<T, S> Writeable for SumTree<T, S>
    where T: std::hash::Hash + Eq + Writeable,
          S: Writeable
{
    fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
        match self.root {
            None => writer.write_u8(0),
            Some(ref node) => node.write(writer)
        }
    }
}

impl<T, S> Writeable for Node<T, S>
    where T: Writeable,
          S: Writeable
{
    fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
        assert!(self.depth < 64);

        // Compute depth byte: 0x80 means full, 0xc0 means unpruned
        let mut depth = 0;
        if self.full == SubtreeFull::Yes {
            depth |= 0x80;
        }
        if let NodeData::Pruned = self.data {
        } else {
            depth |= 0xc0;
        }
        depth |= self.depth;
        // Encode node
        try!(writer.write_u8(depth));
        try!(self.hash.write(writer));
        try!(self.sum.write(writer));
        match self.data {
            NodeData::Pruned => { Ok(()) },
            NodeData::Leaf(ref data) => { data.write(writer) },
            NodeData::Internal { ref lchild, ref rchild } => {
                try!(lchild.write(writer));
                rchild.write(writer)
            },
        }
    }
}

fn node_read_recurse<T, S>(reader: &mut Reader, index: &mut HashMap<T, usize>, tree_index: &mut usize) -> Result<Node<T, S>, ser::Error>
    where T: std::hash::Hash + Eq + Readable + Clone,
          S: Readable
{
    // Read depth byte
    let depth = try!(reader.read_u8());
    let full = if depth & 0x80 == 0x80 { SubtreeFull::Yes } else { SubtreeFull::No };
    let pruned = depth & 0xc0 != 0xc0;
    let depth = depth & 0x3f;

    // Sanity-check for zero byte
    if pruned && full == SubtreeFull::No {
        return Err(ser::Error::CorruptedData);
    }

    // Read remainder of node
    let hash = try!(Readable::read(reader));
    let sum = try!(Readable::read(reader));
    let data = match (depth, pruned) {
        (_, true) => {
            *tree_index += 1 << depth as usize;
            NodeData::Pruned
        }
        (0, _) => {
            let elem: T = try!(Readable::read(reader));
            index.insert(elem.clone(), *tree_index);
            *tree_index += 1;
            NodeData::Leaf(elem)
        }
        (_, _) => NodeData::Internal {
            lchild: Box::new(try!(node_read_recurse(reader, index, tree_index))),
            rchild: Box::new(try!(node_read_recurse(reader, index, tree_index)))
        }
    };

    Ok(Node {
        full: full,
        data: data,
        hash: hash,
        sum: sum,
        depth: depth
    })
}

impl<T, S> Readable for SumTree<T, S>
    where T: Readable + std::hash::Hash + Eq + Clone,
          S: Readable
{
    fn read(reader: &mut Reader) -> Result<SumTree<T, S>, ser::Error> {
        // Read depth byte of root node
        let depth = try!(reader.read_u8());
        let full = if depth & 0x80 == 0x80 { SubtreeFull::Yes } else { SubtreeFull::No };
        let pruned = depth & 0xc0 != 0xc0;
        let depth = depth & 0x3f;

        // Special-case the zero byte
        if pruned && full == SubtreeFull::No {
            return Ok(SumTree {
                index: HashMap::new(),
                root: None
            });
        }

        // Otherwise continue reading it
        let mut index = HashMap::new();

        let hash = try!(Readable::read(reader));
        let sum = try!(Readable::read(reader));
        let data = match (depth, pruned) {
            (_, true) => NodeData::Pruned,
            (0, _) => NodeData::Leaf(try!(Readable::read(reader))),
            (_, _) => {
                let mut tree_index = 0;
                NodeData::Internal {
                    lchild: Box::new(try!(node_read_recurse(reader, &mut index, &mut tree_index))),
                    rchild: Box::new(try!(node_read_recurse(reader, &mut index, &mut tree_index)))
                }
            }
        };

        Ok(SumTree {
            index: index,
            root: Some(Node {
                full: full,
                data: data,
                hash: hash,
                sum: sum,
                depth: depth
            })
        })
    }
}

/// This is used to as a scratch space during root calculation so that we can
/// keep everything on the stack in a fixed-size array. It reflects a maximum
/// tree capacity of 2^48, which is not practically reachable.
const MAX_MMR_HEIGHT: usize = 48;

/// This algorithm is based on Peter Todd's in
/// https://github.com/opentimestamps/opentimestamps-server/blob/master/python-opentimestamps/opentimestamps/core/timestamp.py#L324
///
fn compute_peaks<S, I>(iter: I, peaks: &mut [Option<(u8, Hash, S)>])
    where S: Writeable + ops::Add<Output=S> + Clone,
          I: Iterator<Item=(u8, Hash, S)>
{
    for peak in peaks.iter_mut() {
        *peak = None;
    }
    for (mut new_depth, mut new_hash, mut new_sum) in iter {
        let mut index = 0;
        while let Some((old_depth, old_hash, old_sum)) = peaks[index].take() {
            // Erase current peak (done by `take()` above), then combine
            // it with the new addition, to be inserted one higher
            index += 1;
            new_depth = old_depth + 1;
            new_sum = old_sum.clone() + new_sum.clone();
            new_hash = (new_depth, &new_sum, old_hash, new_hash).hash();
        }
        peaks[index] = Some((new_depth, new_hash, new_sum));
    }
}

/// Directly compute the Merkle root of a sum-tree whose contents are given
/// explicitly in the passed iterator.
pub fn compute_root<'a, T, S, I>(iter: I) -> Option<(Hash, S)>
    where T: 'a + Writeable,
          S: 'a + Writeable + ops::Add<Output=S> + Clone + ::std::fmt::Debug,
          I: Iterator<Item=&'a (T, S)>
{
    let mut peaks = vec![None; MAX_MMR_HEIGHT];
    compute_peaks(iter.map(|&(ref elem, ref sum)| (0, (0u8, sum, Hashed::hash(elem)).hash(), sum.clone())), &mut peaks);

    let mut ret = None;
    for peak in peaks {
        ret = match (peak, ret) {
            (None, x) => x,
            (Some((_, hash, sum)), None) => Some((hash, sum)),
            (Some((depth, lhash, lsum)), Some((rhash, rsum))) => {
                let sum = lsum + rsum;
                let hash = (depth + 1, &sum, lhash, rhash).hash();
                Some((hash, sum))
            }
        };
    }
    ret
}

// a couple functions that help debugging
#[allow(dead_code)]
fn print_node<T, S>(node: &Node<T, S>, tab_level: usize)
    where T: Writeable + Eq + std::hash::Hash,
          S: Writeable + ::std::fmt::Debug
{
    for _ in 0..tab_level {
        print!("    ");
    }
    print!("[{:03}] {} {:?}", node.depth, node.hash, node.sum);
    match node.data {
        NodeData::Pruned => println!(" X"),
        NodeData::Leaf(_) => println!(" L"),
        NodeData::Internal { ref lchild, ref rchild } => {
            println!(":");
            print_node(lchild, tab_level + 1);
            print_node(rchild, tab_level + 1);
        }
    }
}

#[allow(dead_code)]
fn print_tree<T, S>(tree: &SumTree<T, S>)
    where T: Writeable + Eq + std::hash::Hash,
          S: Writeable + ::std::fmt::Debug
{
    match tree.root {
        None => println!("[empty tree]"),
        Some(ref node) => {
            print_node(node, 0);
        }
    }
}

#[cfg(test)]
mod test {
    use rand::{thread_rng, Rng};
    use core::hash::Hashed;
    use super::*;

    fn sumtree_create(prune: bool) {
        let mut tree = SumTree::new();

        macro_rules! leaf {
            ($data_sum: expr) => ({
                (0u8, $data_sum.1, $data_sum.0.hash())
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
                    $tree.prune(&$elem.0);
                    assert_eq!($tree.len(), 0);
                    // double-pruning shouldn't hurt anything
                    $tree.prune(&$elem.0);
                    assert_eq!($tree.len(), 0);
                } else {
                    assert_eq!($tree.len(), $tree.unpruned_len());
                }
            }
        };

        let mut elems = [(*b"ABC0", 10u16), (*b"ABC1", 25u16),
                         (*b"ABC2", 15u16), (*b"ABC3", 11u16),
                         (*b"ABC4", 19u16), (*b"ABC5", 13u16),
                         (*b"ABC6", 30u16), (*b"ABC7", 10000u16)];

        assert_eq!(tree.root_sum(), None);
        assert_eq!(tree.root_sum(), compute_root(elems[0..0].iter()));
        assert_eq!(tree.len(), 0);
        tree.push(elems[0].0, elems[0].1);

        // One element
        let expected = leaf!(elems[0]).hash();
        assert_eq!(tree.root_sum(), Some((expected, 10)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..1].iter()));
        assert_eq!(tree.unpruned_len(), 1);
        prune!(prune, tree, elems[0]);

        // Two elements
        tree.push(elems[1].0, elems[1].1);
        let expected = node!(leaf!(elems[0]),
                             leaf!(elems[1])
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 35)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..2].iter()));
        assert_eq!(tree.unpruned_len(), 2);
        prune!(prune, tree, elems[1]);

        // Three elements
        tree.push(elems[2].0, elems[2].1);
        let expected = node!(node!(leaf!(elems[0]),
                                   leaf!(elems[1])),
                             leaf!(elems[2])
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 50)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..3].iter()));
        assert_eq!(tree.unpruned_len(), 3);
        prune!(prune, tree, elems[2]);

        // Four elements
        tree.push(elems[3].0, elems[3].1);
        let expected = node!(node!(leaf!(elems[0]),
                                   leaf!(elems[1])),
                             node!(leaf!(elems[2]),
                                   leaf!(elems[3]))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 61)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..4].iter()));
        assert_eq!(tree.unpruned_len(), 4);
        prune!(prune, tree, elems[3]);

        // Five elements
        tree.push(elems[4].0, elems[4].1);
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             leaf!(elems[4])
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 80)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..5].iter()));
        assert_eq!(tree.unpruned_len(), 5);
        prune!(prune, tree, elems[4]);

        // Six elements
        tree.push(elems[5].0, elems[5].1);
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             node!(leaf!(elems[4]),
                                   leaf!(elems[5]))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 93)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..6].iter()));
        assert_eq!(tree.unpruned_len(), 6);
        prune!(prune, tree, elems[5]);

        // Seven elements
        tree.push(elems[6].0, elems[6].1);
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             node!(node!(leaf!(elems[4]),
                                         leaf!(elems[5])),
                                   leaf!(elems[6]))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 123)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..7].iter()));
        assert_eq!(tree.unpruned_len(), 7);
        prune!(prune, tree, elems[6]);

        // Eight elements
        tree.push(elems[7].0, elems[7].1);
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             node!(node!(leaf!(elems[4]),
                                         leaf!(elems[5])),
                                   node!(leaf!(elems[6]),
                                         leaf!(elems[7])))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 10123)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..8].iter()));
        assert_eq!(tree.unpruned_len(), 8);
        prune!(prune, tree, elems[7]);

        // If we weren't pruning, try changing some elements
        if !prune {
            for i in 0..8 {
                elems[i].1 += i as u16;
                tree.replace(&elems[i].0, elems[i].0, elems[i].1);
            }
            let expected = node!(node!(node!(leaf!(elems[0]),
                                             leaf!(elems[1])),
                                       node!(leaf!(elems[2]),
                                             leaf!(elems[3]))),
                                 node!(node!(leaf!(elems[4]),
                                             leaf!(elems[5])),
                                       node!(leaf!(elems[6]),
                                             leaf!(elems[7])))
                                ).hash();
            assert_eq!(tree.root_sum(), Some((expected, 10151)));
            assert_eq!(tree.root_sum(), compute_root(elems[0..8].iter()));
            assert_eq!(tree.unpruned_len(), 8);
        }

        let mut rng = thread_rng();
        // If we weren't pruning as we went, try pruning everything now
        // and make sure nothing breaks.
        if !prune {
            rng.shuffle(&mut elems);
            let mut expected_count = 8;
            let expected_root_sum = tree.root_sum();
            for elem in elems.iter() {
                assert_eq!(tree.root_sum(), expected_root_sum);
                assert_eq!(tree.len(), expected_count);
                assert_eq!(tree.unpruned_len(), 8);
                tree.prune(&elem.0);
                expected_count -= 1;
            }
        }

        // Build a large random tree and check its root against that computed
        // by `compute_root`.
        let mut big_elems: Vec<(u32, u64)> = vec![];
        let mut big_tree = SumTree::new();
        for i in 0..1000 {
            let new_elem = rng.gen();
            let new_sum_small: u8 = rng.gen();  // make a smaller number to prevent overflow when adding
            let new_sum = new_sum_small as u64;
            big_elems.push((new_elem, new_sum));
            big_tree.push(new_elem, new_sum);
            if i % 25 == 0 {
                // Verify root
                assert_eq!(big_tree.root_sum(), compute_root(big_elems.iter()));
                // Do serialization roundtrip
            }
        }
    }

    #[test]
    fn sumtree_test() {
        sumtree_create(false);
        sumtree_create(true);
    }
}



