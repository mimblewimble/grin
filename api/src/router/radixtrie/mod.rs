// Copyright 2018 The Grin Developers
//
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

//! Radix Trie based Router for Http Endpoints
//!
//! The router builds and maintains radix tree for endpoints, and
//! looks up a handler (and path parameters if any) for a provided path of request from radix tree.
//!
//! RE: Router design 
//!   written by Heung Lee
//!
//! Implementation Considerations
//! Issues)
//! 1. Storing an instance of Handler trait in node needs resolve
//!    error[E0277]: the trait bound `router::Handler: std::marker::Sized` is not satisfied.
//! Resolution)
//!    Handlers are stored in Vec field of RouteTrie.
//!    The index of Handler in Vec field of RouteTrie is stored in RouteNode.
//!
//! Date structure of storing children nodes of radix trie:
//! There are many ways to design the data structure for children nodes for a node of radix tree.
//! First of all, patricia tree of radix 2 is not suitable for router
//! because for each node of common path segment, there may be more than 2 branches.
//! For instance, /v1/peers/all, /v1/peers/connnect, and /v1/peers/:ip/ban.
//! "all", "connected" and ":ip" are the children of a node, "/v1/peers/"
//! Vec, HashMap and BTreeMap were considered to store children of a node.
//! For simplicity, Vec is however used to store the references of children of a route node.
//!
//! Consideration on separating building trie from looking up corresponding handler for a provided path.
//! Lookup function is contained in Service trait with Fn function type of hyper.
//! To guarantee "without mutating state" for Service trait,
//! building radix tree of endpoints needs be separated from lookup.
//! TrieBuilder is to build radix tree of endpoints.
//! Trie is to look up a handler of an endpoint.
//! 
//! Lastly, the part of design separating two major functions into two separate structures utilized
//! the design by ubnt-intrepid/susanoo.


pub mod trie;
pub mod node;

use std::iter::IntoIterator;

pub use self::trie::Trie;
pub use self::node::Node;

/// Trie Lookup trait
///
/// This trait is the facilitator for Service trait of hyper.
pub trait TrieLookup<'a, T> {
    type Params: IntoIterator<Item = (String, String)>;

    fn lookup(&self, path: &'a str) -> Option<(&T, Self::Params)>;
}
