// Copyright 2017-2018 The Grin Developers
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

use fnv::FnvHashMap;
use hyper::{Method, Error as HyperError, StatusCode};
use hyper::server::{Request, Response};

use super::{Builder, Route, Trie, TrieLookup};

#[derive(Debug)]
pub struct Router {
    radixtrie: Trie<FnvHashMap<Method, usize>>,    
    routes: Vec<Route>,
}

impl Router {

    /// Constructs Router.
    pub fn new(radixtrie: Trie<FnvHashMap<Method, usize>>, routes: Vec<Route>) -> Router {
        Router {
            radixtrie: radixtrie,
            routes: routes,
        }
    }

    /// Constructs Builder that is called to insert endpoints and corresponding handers.
    pub fn builder() -> Builder {
        Default::default()
    }

    /// Look up the handler for a provided path.
    ///
    /// Calls lookup function of radix trie and
    /// gets the index of corresponding hander for a provided method and path.
    /// Then executes corresponding handler through RouterContext.
    pub fn lookup(&self, req: Request) -> Result<Response, HyperError> {
        // look up in radix tree.
        match self.radixtrie.lookup(req.path()) {
            Some((methods, params)) => {
                RouterContext {
                    methods,
                    routes: &self.routes[..]
                }.handle(req, params)
            }
            None => Ok(Response::new().with_status(StatusCode::NotFound))
        }
    }
}

/// Router Context.
///
/// Router context actually executes corresponding handler within the lifetime of hyper service.
struct RouterContext<'a> {
    methods: &'a FnvHashMap<Method, usize>,
    routes: &'a [Route],
}

impl<'a> RouterContext<'a> {
    pub fn handle(&self, req: Request, params: FnvHashMap<String, String>) -> Result<Response, HyperError> {
        match self.methods.get(req.method()) {
            Some(&i) => {
                self.routes[i].handler().handle(req, params)
            }
            None => Ok(Response::new().with_status(StatusCode::NotFound))
        }
    }
}