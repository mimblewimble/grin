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

// HashMap using Fnv hasher:
// Data structure used to store map of handler to index of handler vector in the node of radix trie.
// Fowler-Noll-Vo (fnv): A hash function with excellent distribution,
// but no particular cryptographic properties.
// FNV is the best at small inputs, but does terribly on large inputs.
// Since small inputs are much more common,
// it's the Rust community's most popular override for SipHash where security doesn't matter.
// The Rust compiler itself uses this function.
// source: http://cglab.ca/%7Eabeinges/blah/hash-rs/
// credit: https://github.com/ubnt-intrepid/susanoo

use fnv::FnvHashMap;
use hyper::Method;
use std::{error, fmt, mem};

use rest::Handler;
use super::{Route, Router, Trie};
use super::radixtrie;

#[derive(Debug)]
pub enum BuildError {
    Trie(radixtrie::trie::Error),
    Node(radixtrie::node::Error),
    InvalidFormat(String),
}

impl From<radixtrie::node::Error> for BuildError {
    fn from(err: radixtrie::node::Error) -> BuildError {
        BuildError::Node(err)
    }
}

impl From<radixtrie::trie::Error> for BuildError {
    fn from(err: radixtrie::trie::Error) -> BuildError {
        BuildError::Trie(err)
    }
}

impl error::Error for BuildError {
    fn description(&self) -> &str {
        match *self {
            BuildError::Trie(ref e) => e.description(),
            BuildError::Node(ref e) => e.description(),
            BuildError::InvalidFormat(ref _endpoint) => "Invalid endpoint format",
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            BuildError::Trie(ref e) => e.cause(),
            BuildError::Node(ref e) => e.cause(),
            BuildError::InvalidFormat(ref _endpoint) => None,
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            BuildError::Trie(ref e) => fmt::Display::fmt(e, f),
            BuildError::Node(ref e) => fmt::Display::fmt(e, f),
            BuildError::InvalidFormat(ref endpoint) => write!(f, "Invalid endpoint format - {:?}", endpoint),
        }
    }
}

/// Router Builder
///
/// Defined to separate building a router that requires mutating state
/// from looking up corresponding handler for a provided path.
pub struct Builder {
    routes: Vec<Route>,
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            routes: Vec::new(),
        }
    }
}

impl Builder {

    /// Creates a Route for provided endpoint of http Get method and corresponding Handler, and
    /// add the Route into the list of Routes.
    pub fn get<H: Handler>(&mut self, endpoint: &str, handler: H) {
        let route = Route::new(Method::Get, endpoint, handler);
        self.routes.push(route);
    }

    /// Creates a Route for provided endpoint of http Post method and corresponding Handler, and
    /// add the Route into the list of Routes.
    pub fn post<H: Handler>(&mut self, endpoint: &str, handler: H) {
        let route = Route::new(Method::Post, endpoint, handler);
        self.routes.push(route);
    }

    /// Inserts each Route of Builder into Radix Trie, and
    /// returns Router containing Radix Trie and Routes.
    pub fn into_router(&mut self) -> Result<Router, BuildError> {
        // clone and diassociate routes.
        let Builder {
            routes,
        } = mem::replace(self, Default::default());

        let mut endpoints: FnvHashMap<String, FnvHashMap<Method, usize>> = FnvHashMap::with_hasher(Default::default());
        for (i, route) in routes.iter().enumerate() {
            endpoints.entry(route.endpoint().to_owned())
                .or_insert_with(Default::default)
                .insert(route.method().clone(), i);
        }

        let router_trie = {
            let mut trie_builder = Trie::<FnvHashMap<Method, usize>>::builder();
            for (endpoint, methods) in endpoints {
                if let Err(e) = validate_endpoint(&endpoint) {
                    return Err(e);
                }
                trie_builder.insert(&endpoint, methods)?;
            }
            trie_builder.into_trie()? 
        };

        Ok(Router::new(router_trie, routes))
    }
}

fn validate_endpoint(endpoint: &str) -> Result<(), BuildError> {
    let path = endpoint.as_bytes();

    if path.len() == 0 {
        return Err(BuildError::InvalidFormat(endpoint.to_string()));
    }
    // require leading slash
    if path[0] != b'/' {
        return Err(BuildError::InvalidFormat(endpoint.to_string()));
    }

    if path.len() > 1 {
        // not allow greedy wildcard, "/*" and "/:"
        if path[1] == b'*' || path[1] == b':' {
            return Err(BuildError::InvalidFormat(endpoint.to_string()));
        }

        // not allow wasteful trailing slash
        if path[path.len() - 1] == b'/' {
            return Err(BuildError::InvalidFormat(endpoint.to_string()));
        }

        let limit = path.len() - 1;
        for i in 0..path.len() - 1 {
            if path[i] == b'/' {
                if i < limit && path[i + 1] == b'/' {
                    return Err(BuildError::InvalidFormat(endpoint.to_string()));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_valid_endpoint() {
        let endpoint = "/v1/peers/:ip";
        assert_eq!(validate_endpoint(endpoint).is_ok(), true);
    }

    #[test]
    fn test_no_leading_slash_endpoint() {
        let endpoint = "v1/peers/:ip";
        assert_eq!(validate_endpoint(endpoint).is_ok(), false);
    }

    #[test]
    fn test_trailing_slash_endpoint() {
        let endpoint = "/v1/peers/:ip/";
        assert_eq!(validate_endpoint(endpoint).is_ok(), false);
    }

    #[test]
    fn test_double_slash_endpoint() {
        let endpoint = "/v1//peers/:ip";
        assert_eq!(validate_endpoint(endpoint).is_ok(), false);
    }

    #[test]
    fn test_leading_catchall_slash_endpoint() {
        let endpoint = "/*v1/peers/:ip";
        assert_eq!(validate_endpoint(endpoint).is_ok(), false);
    }

    #[test]
    fn test_leading_param_endpoint() {
        let endpoint = "/:v1/peers/:ip";
        assert_eq!(validate_endpoint(endpoint).is_ok(), false);
    }              
}
