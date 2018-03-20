// Copyright 2016-2018 The Grin Developers
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
//! RE: Router Design/Implementation Considerations
//!   written by Heung Lee
//!
//! Implementation Considerations
//! Issues)
//! 1. Storing an instance of Handler trait in node needs resolve
//!    error[E0277]: the trait bound `router::Handler: std::marker::Sized` is not satisfied.
//! Resolution)
//!    Handlers are stored in Vec field of Route.
//!    The index of Handler in Vec field of Route is stored in Node of radix trie.
//!
//! Wildcard Formats:
//!   ":" for path parameters; /a/:b
//!   "*" for catch all; /a/*b
//!
//! Endpoint Format Requirements:
//!
//! 1. Must have the leading slash.
//! 2. No greedy wildcard, such as "/*" and "/:".
//! 3. No trailing slash.
//! 4. No multiple slashes in a row, such as "/a//b".
//! 5. Two different wildcards are not allowed at the same path component.
//! The following case will cause panic: /a/:b and /a/:c.
//! 6. No wildcard as part of wildcard name. /:a:b/, /*a*a/, so on.
//!
//! Data Stucture for Path Parameters (a.k.a. wildcard):
//! The data structure of path parameter consists of name stored in trie node and
//! value of wildcard in a provided path of request.
//! Data of path parameters are created and returned by lookup function of trie node.
//! For scalability and simplicity, FnvHashMap is used.
//! reference) http://cglab.ca/%7Eabeinges/blah/hash-rs/
//!

pub mod router;
pub mod builder;
#[macro_use]
pub mod macros;

mod route;
mod radixtrie;

pub use self::builder::Builder;
pub use self::route::Route;
pub use self::router::Router;
pub use self::radixtrie::TrieLookup;
pub use self::radixtrie::trie::{Trie, TrieBuilder};
