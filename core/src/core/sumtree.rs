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

/// Trait describing an object that has a well-defined sum that the tree can sum over
pub trait Summable {
    /// The type of an object's sum
    type Sum: Clone + ops::Add<Output=Self::Sum> + Readable + Writeable;

    /// Obtain the sum of the object
    fn sum(&self) -> Self::Sum;
}

/// An empty sum that takes no space
#[derive(Copy, Clone)]
pub struct NullSum;
impl ops::Add for NullSum {
    type Output = NullSum;
    fn add(self, _: NullSum) -> NullSum { NullSum }
}

impl Readable for NullSum {
    fn read(_: &mut Reader) -> Result<NullSum, ser::Error> {
        Ok(NullSum)
    }
}

impl Writeable for NullSum {
    fn write<W: Writer>(&self, _: &mut W) -> Result<(), ser::Error> {
        Ok(())
    }
}

/// Wrapper for a type that allows it to be inserted in a tree without summing
pub struct NoSum<T>(T);
impl<T> Summable for NoSum<T> {
    type Sum = NullSum;
    fn sum(&self) -> NullSum { NullSum }
}

#[derive(Clone)]
enum NodeData<T: Summable> {
    /// Node with 2^n children which are not stored with the tree
    Pruned(T::Sum),
    /// Actual data
    Leaf(T),
    /// Node with 2^n children
    Internal {
        lchild: Box<Node<T>>,
        rchild: Box<Node<T>>,
        sum: T::Sum
    },
}

impl<T: Summable> Summable for NodeData<T> {
    type Sum = T::Sum;
    fn sum(&self) -> T::Sum {
        match *self {
            NodeData::Pruned(ref sum) => sum.clone(),
            NodeData::Leaf(ref data) => data.sum(),
            NodeData::Internal { ref sum, .. } => sum.clone()
        }
    }
}

#[derive(Clone)]
struct Node<T: Summable> {
    full: bool,
    data: NodeData<T>,
    hash: Hash,
    depth: u8
}

impl<T: Summable> Summable for Node<T> {
    type Sum = T::Sum;
    fn sum(&self) -> T::Sum {
        self.data.sum()
    }
}

impl<T: Summable> Node<T> {
    /// Get the root hash and sum of the node
    fn root_sum(&self) -> (Hash, T::Sum) {
        (self.hash, self.sum())
    }

    fn n_children(&self) -> usize {
        if self.full {
            1 << self.depth
        } else {
            if let NodeData::Internal{ ref lchild, ref rchild, .. } = self.data {
                lchild.n_children() + rchild.n_children()
            } else {
                unreachable!()
            }
        }
    }

}

/// An insertion ordered merkle sum tree.
#[derive(Clone)]
pub struct SumTree<T: Summable + Writeable> {
    /// Index mapping data to its index in the tree
    index: HashMap<Hash, usize>,
    /// Tree contents
    root: Option<Node<T>>
}

impl<T> SumTree<T>
    where T: Summable + Writeable
{
    /// Create a new empty tree
    pub fn new() -> SumTree<T> {
        SumTree {
            index: HashMap::new(),
            root: None
        }
    }

    /// Accessor for the tree's root
    pub fn root_sum(&self) -> Option<(Hash, T::Sum)> {
        self.root.as_ref().map(|node| node.root_sum())
    }

    fn insert_right_of(mut old: Node<T>, new: Node<T>) -> Node<T> {
        assert!(old.depth >= new.depth);

        // If we are inserting next to a full node, make a parent. If we're
        // inserting a tree of equal depth then we get a full node, otherwise
        // we get a partial node. Leaves and pruned data both count as full
        // nodes.
        if old.full {
            let parent_depth = old.depth + 1;
            let parent_sum = old.sum() + new.sum();
            let parent_hash = (parent_depth, &parent_sum, old.hash, new.hash).hash();
            let parent_full = old.depth == new.depth;
            let parent_data = NodeData::Internal {
                lchild: Box::new(old),
                rchild: Box::new(new),
                sum: parent_sum,
            };

            Node {
                full: parent_full,
                data: parent_data,
                hash: parent_hash,
                depth: parent_depth
            }
        // If we are inserting next to a partial node, we should actually be
        // inserting under the node, so we recurse. The right child of a partial
        // node is always another partial node or a leaf.
        } else {
            if let NodeData::Internal{ ref lchild, ref mut rchild, ref mut sum } = old.data {
                // Recurse
                let dummy_child = Node { full: true, data: NodeData::Pruned(sum.clone()), hash: old.hash, depth: 0 };
                let moved_rchild = mem::replace(&mut **rchild, dummy_child);
                mem::replace(&mut **rchild, SumTree::insert_right_of(moved_rchild, new));
                // Update this node's states to reflect the new right child
                if rchild.full && rchild.depth == old.depth - 1 {
                    old.full = rchild.full;
                }
                *sum = lchild.sum() + rchild.sum();
                old.hash = (old.depth, &*sum, lchild.hash, rchild.hash).hash();
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

    /// Add an element to the tree. Returns true if the element was added,
    /// false if it already existed in the tree.
    pub fn push(&mut self, elem: T) -> bool {
        // Compute element hash and depth-0 node hash
        let index_hash = Hashed::hash(&elem);
        let elem_sum = elem.sum();
        let elem_hash = (0u8, &elem_sum, index_hash).hash();

        if self.index.contains_key(&index_hash) {
            return false;
        }

        // Special-case the first element
        if self.root.is_none() {
            self.root = Some(Node {
                full: true,
                data: NodeData::Leaf(elem),
                hash: elem_hash,
                depth: 0
            });
            self.index.insert(index_hash, 0);
            return true;
        }

        // Next, move the old root out of the structure so that we are allowed to
        // move it. We will move a new root back in at the end of the function
        let old_root = mem::replace(&mut self.root, None).unwrap();

        // Insert into tree, compute new root
        let new_node = Node {
            full: true,
            data: NodeData::Leaf(elem),
            hash: elem_hash,
            depth: 0
        };

        // Put new root in place and record insertion
        let index = old_root.n_children();
        self.root = Some(SumTree::insert_right_of(old_root, new_node));
        self.index.insert(index_hash, index);
        true
    }

    fn replace_recurse(node: &mut Node<T>, index: usize, new_elem: T) {
        assert!(index < (1 << node.depth));

        if node.depth == 0 {
            assert!(node.full);
            node.hash = (0u8, new_elem.sum(), Hashed::hash(&new_elem)).hash();
            node.data = NodeData::Leaf(new_elem);
        } else {
            match node.data {
                NodeData::Internal { ref mut lchild, ref mut rchild, ref mut sum } => {
                    let bit = index & (1 << (node.depth - 1));
                    if bit > 0 {
                        SumTree::replace_recurse(rchild, index - bit, new_elem);
                    } else {
                        SumTree::replace_recurse(lchild, index, new_elem);
                    }
                    *sum = lchild.sum() + rchild.sum();
                    node.hash = (node.depth, &*sum, lchild.hash, rchild.hash).hash();
                }
                // Pruned data would not have been in the index
                NodeData::Pruned(_) => unreachable!(),
                NodeData::Leaf(_) => unreachable!()
            }
        }
    }

    /// Replaces an element in the tree. Returns true if the element existed
    /// and was replaced. Returns false if the old element did not exist or
    /// if the new element already existed
    pub fn replace(&mut self, elem: &T, new_elem: T) -> bool {
        let index_hash = Hashed::hash(elem);

        let root = match self.root {
            Some(ref mut node) => node,
            None => { return false; }
        };

        match self.index.remove(&index_hash) {
            None => false,
            Some(index) => {
                let new_index_hash = Hashed::hash(&new_elem);
                if self.index.contains_key(&new_index_hash) {
                    false
                } else {
                    SumTree::replace_recurse(root, index, new_elem);
                    self.index.insert(new_index_hash, index);
                    true
                }
            }
        }
    }

    /// Determine whether an element exists in the tree.
    /// If so, return its index
    pub fn contains(&self, elem: &T) -> Option<usize> {
        let index_hash = Hashed::hash(elem);
        self.index.get(&index_hash).map(|x| *x)
    }

    fn prune_recurse(node: &mut Node<T>, index: usize) {
        assert!(index < (1 << node.depth));

        if node.depth == 0 {
            let sum = if let NodeData::Leaf(ref elem) = node.data {
                elem.sum()
            } else {
                unreachable!()
            };
            node.data = NodeData::Pruned(sum);
        } else {
            let mut prune_me = None;
            match node.data {
                NodeData::Internal { ref mut lchild, ref mut rchild, .. } => {
                    let bit = index & (1 << (node.depth - 1));
                    if bit > 0 {
                        SumTree::prune_recurse(rchild, index - bit);
                    } else {
                        SumTree::prune_recurse(lchild, index);
                    }
                    if let (&NodeData::Pruned(ref lsum), &NodeData::Pruned(ref rsum)) = (&lchild.data, &rchild.data) {
                        if node.full {
                            prune_me = Some(lsum.clone() + rsum.clone());
                        }
                    }
                }
                NodeData::Pruned(_) => {
                    // Already pruned. Ok.
                }
                NodeData::Leaf(_) => unreachable!()
            }
            if let Some(sum) = prune_me {
                node.data = NodeData::Pruned(sum);
            }
        }
    }

    /// Removes an element from storage, not affecting the tree
    /// Returns true if the element was actually in the tree
    pub fn prune(&mut self, elem: &T) -> bool {
        let index_hash = Hashed::hash(elem);

        let root = match self.root {
            Some(ref mut node) => node,
            None => { return false; }
        };

        match self.index.remove(&index_hash) {
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
impl<T> Writeable for SumTree<T>
    where T: Summable + Writeable
{
    fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
        match self.root {
            None => writer.write_u8(0),
            Some(ref node) => node.write(writer)
        }
    }
}

impl<T> Writeable for Node<T>
    where T: Summable + Writeable
{
    fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
        assert!(self.depth < 64);

        // Compute depth byte: 0x80 means full, 0xc0 means unpruned
        let mut depth = 0;
        if self.full {
            depth |= 0x80;
        }
        if let NodeData::Pruned(_) = self.data {
        } else {
            depth |= 0xc0;
        }
        depth |= self.depth;
        // Encode node
        try!(writer.write_u8(depth));
        try!(self.hash.write(writer));
        match self.data {
            NodeData::Pruned(ref sum) => {
                sum.write(writer)
            },
            NodeData::Leaf(ref data) => {
                data.write(writer)
            },
            NodeData::Internal { ref lchild, ref rchild, ref sum } => {
                try!(sum.write(writer));
                try!(lchild.write(writer));
                rchild.write(writer)
            },
        }
    }
}

fn node_read_recurse<T>(reader: &mut Reader, index: &mut HashMap<Hash, usize>, tree_index: &mut usize) -> Result<Node<T>, ser::Error>
    where T: Summable + Readable + Hashed
{
    // Read depth byte
    let depth = try!(reader.read_u8());
    let full = depth & 0x80 == 0x80;
    let pruned = depth & 0xc0 != 0xc0;
    let depth = depth & 0x3f;

    // Sanity-check for zero byte
    if pruned && !full {
        return Err(ser::Error::CorruptedData);
    }

    // Read remainder of node
    let hash = try!(Readable::read(reader));
    let data = match (depth, pruned) {
        (_, true) => {
            let sum = try!(Readable::read(reader));
            *tree_index += 1 << depth as usize;
            NodeData::Pruned(sum)
        }
        (0, _) => {
            let elem: T = try!(Readable::read(reader));
            index.insert(Hashed::hash(&elem), *tree_index);
            *tree_index += 1;
            NodeData::Leaf(elem)
        }
        (_, _) => {
            let sum = try!(Readable::read(reader));
            NodeData::Internal {
                lchild: Box::new(try!(node_read_recurse(reader, index, tree_index))),
                rchild: Box::new(try!(node_read_recurse(reader, index, tree_index))),
                sum: sum
            }
        }
    };

    Ok(Node {
        full: full,
        data: data,
        hash: hash,
        depth: depth
    })
}

impl<T> Readable for SumTree<T>
    where T: Summable + Writeable + Readable + Hashed
{
    fn read(reader: &mut Reader) -> Result<SumTree<T>, ser::Error> {
        // Read depth byte of root node
        let depth = try!(reader.read_u8());
        let full = depth & 0x80 == 0x80;
        let pruned = depth & 0xc0 != 0xc0;
        let depth = depth & 0x3f;

        // Special-case the zero byte
        if pruned && !full {
            return Ok(SumTree {
                index: HashMap::new(),
                root: None
            });
        }

        // Otherwise continue reading it
        let mut index = HashMap::new();

        let hash = try!(Readable::read(reader));
        let data = match (depth, pruned) {
            (_, true) => {
                let sum = try!(Readable::read(reader));
                NodeData::Pruned(sum)
            }
            (0, _) => NodeData::Leaf(try!(Readable::read(reader))),
            (_, _) => {
                let sum = try!(Readable::read(reader));
                let mut tree_index = 0;
                NodeData::Internal {
                    lchild: Box::new(try!(node_read_recurse(reader, &mut index, &mut tree_index))),
                    rchild: Box::new(try!(node_read_recurse(reader, &mut index, &mut tree_index))),
                    sum: sum
                }
            }
        };

        Ok(SumTree {
            index: index,
            root: Some(Node {
                full: full,
                data: data,
                hash: hash,
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
    where S: Clone + ops::Add<Output=S> + Writeable,
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
pub fn compute_root<'a, T, I>(iter: I) -> Option<(Hash, T::Sum)>
    where T: 'a + Summable + Writeable,
          I: Iterator<Item=&'a T>
{
    let mut peaks = vec![None; MAX_MMR_HEIGHT];
    compute_peaks(iter.map(|elem| {
        let depth = 0u8;
        let sum = elem.sum();
        let hash = (depth, &sum, Hashed::hash(elem)).hash();
        (depth, hash, sum)
    }), &mut peaks);

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
fn print_node<T>(node: &Node<T>, tab_level: usize)
    where T: Summable + Writeable,
          T::Sum: std::fmt::Debug
{
    for _ in 0..tab_level {
        print!("    ");
    }
    print!("[{:03}] {} {:?}", node.depth, node.hash, node.sum());
    match node.data {
        NodeData::Pruned(_) => println!(" X"),
        NodeData::Leaf(_) => println!(" L"),
        NodeData::Internal { ref lchild, ref rchild, .. } => {
            println!(":");
            print_node(lchild, tab_level + 1);
            print_node(rchild, tab_level + 1);
        }
    }
}

#[allow(dead_code)]
fn print_tree<T>(tree: &SumTree<T>)
    where T: Summable + Writeable,
          T::Sum: std::fmt::Debug
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
    use ser;
    use super::*;

    #[derive(Copy, Clone, Debug)]
    struct TestElem([u32; 4]);
    impl Summable for TestElem {
        type Sum = u64;
        fn sum(&self) -> u64 {
            // sums are not allowed to overflow, so we use this simple
            // non-injective "sum" function that will still be homomorphic
            self.0[0] as u64 * 0x1000 +
            self.0[1] as u64 * 0x100 +
            self.0[2] as u64 * 0x10 +
            self.0[3] as u64
        }
    }

    impl Writeable for TestElem {
        fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
            try!(writer.write_u32(self.0[0]));
            try!(writer.write_u32(self.0[1]));
            try!(writer.write_u32(self.0[2]));
            writer.write_u32(self.0[3])
        }
    }


    fn sumtree_create_(prune: bool) {
        let mut tree = SumTree::new();

        macro_rules! leaf {
            ($data: expr) => ({
                (0u8, $data.sum(), $data.hash())
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
                    $tree.prune(&$elem);
                    assert_eq!($tree.len(), 0);
                    // double-pruning shouldn't hurt anything
                    $tree.prune(&$elem);
                    assert_eq!($tree.len(), 0);
                } else {
                    assert_eq!($tree.len(), $tree.unpruned_len());
                }
            }
        };

        let mut elems = [TestElem([0, 0, 0, 1]), TestElem([0, 0, 0, 2]),
                         TestElem([0, 0, 0, 3]), TestElem([0, 0, 0, 4]),
                         TestElem([0, 0, 0, 5]), TestElem([0, 0, 0, 6]),
                         TestElem([0, 0, 0, 7]), TestElem([1, 0, 0, 0])];

        assert_eq!(tree.root_sum(), None);
        assert_eq!(tree.root_sum(), compute_root(elems[0..0].iter()));
        assert_eq!(tree.len(), 0);
        assert_eq!(tree.contains(&elems[0]), None);
        assert!(tree.push(elems[0]));
        assert_eq!(tree.contains(&elems[0]), Some(0));

        // One element
        let expected = leaf!(elems[0]).hash();
        assert_eq!(tree.root_sum(), Some((expected, 1)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..1].iter()));
        assert_eq!(tree.unpruned_len(), 1);
        prune!(prune, tree, elems[0]);

        // Two elements
        assert_eq!(tree.contains(&elems[1]), None);
        assert!(tree.push(elems[1]));
        assert_eq!(tree.contains(&elems[1]), Some(1));
        let expected = node!(leaf!(elems[0]),
                             leaf!(elems[1])
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 3)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..2].iter()));
        assert_eq!(tree.unpruned_len(), 2);
        prune!(prune, tree, elems[1]);

        // Three elements
        assert_eq!(tree.contains(&elems[2]), None);
        assert!(tree.push(elems[2]));
        assert_eq!(tree.contains(&elems[2]), Some(2));
        let expected = node!(node!(leaf!(elems[0]),
                                   leaf!(elems[1])),
                             leaf!(elems[2])
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 6)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..3].iter()));
        assert_eq!(tree.unpruned_len(), 3);
        prune!(prune, tree, elems[2]);

        // Four elements
        assert_eq!(tree.contains(&elems[3]), None);
        assert!(tree.push(elems[3]));
        assert_eq!(tree.contains(&elems[3]), Some(3));
        let expected = node!(node!(leaf!(elems[0]),
                                   leaf!(elems[1])),
                             node!(leaf!(elems[2]),
                                   leaf!(elems[3]))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 10)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..4].iter()));
        assert_eq!(tree.unpruned_len(), 4);
        prune!(prune, tree, elems[3]);

        // Five elements
        assert_eq!(tree.contains(&elems[4]), None);
        assert!(tree.push(elems[4]));
        assert_eq!(tree.contains(&elems[4]), Some(4));
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             leaf!(elems[4])
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 15)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..5].iter()));
        assert_eq!(tree.unpruned_len(), 5);
        prune!(prune, tree, elems[4]);

        // Six elements
        assert_eq!(tree.contains(&elems[5]), None);
        assert!(tree.push(elems[5]));
        assert_eq!(tree.contains(&elems[5]), Some(5));
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             node!(leaf!(elems[4]),
                                   leaf!(elems[5]))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 21)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..6].iter()));
        assert_eq!(tree.unpruned_len(), 6);
        prune!(prune, tree, elems[5]);

        // Seven elements
        assert_eq!(tree.contains(&elems[6]), None);
        assert!(tree.push(elems[6]));
        assert_eq!(tree.contains(&elems[6]), Some(6));
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             node!(node!(leaf!(elems[4]),
                                         leaf!(elems[5])),
                                   leaf!(elems[6]))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 28)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..7].iter()));
        assert_eq!(tree.unpruned_len(), 7);
        prune!(prune, tree, elems[6]);

        // Eight elements
        assert_eq!(tree.contains(&elems[7]), None);
        assert!(tree.push(elems[7]));
        assert_eq!(tree.contains(&elems[7]), Some(7));
        let expected = node!(node!(node!(leaf!(elems[0]),
                                         leaf!(elems[1])),
                                   node!(leaf!(elems[2]),
                                         leaf!(elems[3]))),
                             node!(node!(leaf!(elems[4]),
                                         leaf!(elems[5])),
                                   node!(leaf!(elems[6]),
                                         leaf!(elems[7])))
                            ).hash();
        assert_eq!(tree.root_sum(), Some((expected, 28 + 0x1000)));
        assert_eq!(tree.root_sum(), compute_root(elems[0..8].iter()));
        assert_eq!(tree.unpruned_len(), 8);
        prune!(prune, tree, elems[7]);

        // If we weren't pruning, try changing some elements
        if !prune {
            for i in 0..8 {
                let old_elem = elems[i];
                elems[i].0[2] += 1 + i as u32;
                assert_eq!(tree.contains(&old_elem), Some(i));
                assert_eq!(tree.contains(&elems[i]), None);
                assert!(tree.replace(&old_elem, elems[i]));
                assert_eq!(tree.contains(&elems[i]), Some(i));
                assert_eq!(tree.contains(&old_elem), None);
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
            assert_eq!(tree.root_sum(), Some((expected, 28 + 36 * 0x10 + 0x1000)));
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
            for elem in &elems {
                assert_eq!(tree.root_sum(), expected_root_sum);
                assert_eq!(tree.len(), expected_count);
                assert_eq!(tree.unpruned_len(), 8);
                tree.prune(elem);
                expected_count -= 1;
            }
        }

        // Build a large random tree and check its root against that computed
        // by `compute_root`.
        let mut big_elems: Vec<TestElem> = vec![];
        let mut big_tree = SumTree::new();
        for i in 0..1000 {
            // To avoid RNG overflow we generate random elements that are small.
            // Though to avoid repeat elements they have to be reasonably big.
            let new_elem;
            let word1 = rng.gen::<u16>() as u32;
            let word2 = rng.gen::<u16>() as u32;
            if rng.gen() {
                if rng.gen() {
                    new_elem = TestElem([word1, word2, 0, 0]);
                } else {
                    new_elem = TestElem([word1, 0, word2, 0]);
                }
            } else {
                if rng.gen() {
                    new_elem = TestElem([0, word1, 0, word2]);
                } else {
                    new_elem = TestElem([0, 0, word1, word2]);
                }
            }

            big_elems.push(new_elem);
            assert!(big_tree.push(new_elem));
            if i % 25 == 0 {
                // Verify root
                println!("{}", i);
                assert_eq!(big_tree.root_sum(), compute_root(big_elems.iter()));
                // Do serialization roundtrip
            }
        }
    }

    #[test]
    fn sumtree_create() {
        sumtree_create_(false);
        sumtree_create_(true);
    }

    #[test]
    fn sumtree_double_add() {
        let elem = TestElem([10, 100, 1000, 10000]);

        let mut tree = SumTree::new();
        // Cannot prune a nonexistant element
        assert!(!tree.prune(&elem));
        // Can add
        assert!(tree.push(elem));
        // Cannot double-add
        assert!(!tree.push(elem));
        // Can prune but not double-prune
        assert!(tree.prune(&elem));
        assert!(!tree.prune(&elem));
        // Can re-add
        assert!(tree.push(elem));
    }
}



