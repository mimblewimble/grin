// Copyright 2021 The Grin Developers
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

use futures::future::{self, Future};
use hyper::service::Service;
use hyper::{Body, Method, Request, Response, StatusCode};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

lazy_static! {
	static ref WILDCARD_HASH: u64 = calculate_hash(&"*");
	static ref WILDCARD_STOP_HASH: u64 = calculate_hash(&"**");
}

pub type ResponseFuture =
	Pin<Box<dyn Future<Output = Result<Response<Body>, hyper::Error>> + Send>>;

pub trait Handler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn post(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn put(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn patch(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn delete(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn head(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn trace(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn connect(&self, _req: Request<Body>) -> ResponseFuture {
		not_found()
	}

	fn call(
		&self,
		req: Request<Body>,
		mut _handlers: Box<dyn Iterator<Item = HandlerObj>>,
	) -> ResponseFuture {
		match *req.method() {
			Method::GET => self.get(req),
			Method::POST => self.post(req),
			Method::PUT => self.put(req),
			Method::DELETE => self.delete(req),
			Method::PATCH => self.patch(req),
			Method::OPTIONS => self.options(req),
			Method::CONNECT => self.connect(req),
			Method::TRACE => self.trace(req),
			Method::HEAD => self.head(req),
			_ => not_found(),
		}
	}
}

#[derive(Clone, thiserror::Error, Eq, Debug, PartialEq, Serialize, Deserialize)]
pub enum RouterError {
	#[error("Route already exists")]
	RouteAlreadyExists,
	#[error("Route not found")]
	RouteNotFound,
	#[error("Value not found")]
	NoValue,
}

#[derive(Clone)]
pub struct Router {
	nodes: Vec<Node>,
}

#[derive(Debug, Clone, Copy)]
struct NodeId(usize);

const MAX_CHILDREN: usize = 16;

pub type HandlerObj = Arc<dyn Handler + Send + Sync>;

#[derive(Clone)]
pub struct Node {
	key: u64,
	value: Option<HandlerObj>,
	children: [NodeId; MAX_CHILDREN],
	children_count: usize,
	mws: Option<Vec<HandlerObj>>,
}

impl Router {
	pub fn new() -> Router {
		let root = Node::new(calculate_hash(&""), None);
		let mut nodes = vec![];
		nodes.push(root);
		Router { nodes }
	}

	pub fn add_middleware(&mut self, mw: HandlerObj) {
		self.node_mut(NodeId(0)).add_middleware(mw);
	}

	fn root(&self) -> NodeId {
		NodeId(0)
	}

	fn node(&self, id: NodeId) -> &Node {
		&self.nodes[id.0]
	}

	fn node_mut(&mut self, id: NodeId) -> &mut Node {
		&mut self.nodes[id.0]
	}

	fn find(&self, parent: NodeId, key: u64) -> Option<NodeId> {
		let node = self.node(parent);
		node.children
			.iter()
			.find(|&id| {
				let node_key = self.node(*id).key;
				node_key == key || node_key == *WILDCARD_HASH || node_key == *WILDCARD_STOP_HASH
			})
			.cloned()
	}

	fn add_empty_node(&mut self, parent: NodeId, key: u64) -> NodeId {
		let id = NodeId(self.nodes.len());
		self.nodes.push(Node::new(key, None));
		self.node_mut(parent).add_child(id);
		id
	}

	pub fn add_route(
		&mut self,
		route: &'static str,
		value: HandlerObj,
	) -> Result<&mut Node, RouterError> {
		let keys = generate_path(route);
		let mut node_id = self.root();
		for key in keys {
			node_id = self
				.find(node_id, key)
				.unwrap_or_else(|| self.add_empty_node(node_id, key));
		}
		match self.node(node_id).value() {
			None => {
				let node = self.node_mut(node_id);
				node.set_value(value);
				Ok(node)
			}
			Some(_) => Err(RouterError::RouteAlreadyExists),
		}
	}

	pub fn get(&self, path: &str) -> Result<impl Iterator<Item = HandlerObj>, RouterError> {
		let keys = generate_path(path);
		let mut handlers = vec![];
		let mut node_id = self.root();
		collect_node_middleware(&mut handlers, self.node(node_id));
		for key in keys {
			node_id = self.find(node_id, key).ok_or(RouterError::RouteNotFound)?;
			let node = self.node(node_id);
			collect_node_middleware(&mut handlers, self.node(node_id));
			if node.key == *WILDCARD_STOP_HASH {
				break;
			}
		}

		if let Some(h) = self.node(node_id).value() {
			handlers.push(h);
			Ok(handlers.into_iter())
		} else {
			Err(RouterError::NoValue)
		}
	}
}

impl Service<Request<Body>> for Router {
	type Response = Response<Body>;
	type Error = hyper::Error;
	type Future = ResponseFuture;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, req: Request<Body>) -> Self::Future {
		match self.get(req.uri().path()) {
			Err(_) => not_found(),
			Ok(mut handlers) => match handlers.next() {
				None => not_found(),
				Some(h) => h.call(req, Box::new(handlers)),
			},
		}
	}
}

impl Node {
	fn new(key: u64, value: Option<HandlerObj>) -> Node {
		Node {
			key,
			value,
			children: [NodeId(0); MAX_CHILDREN],
			children_count: 0,
			mws: None,
		}
	}

	pub fn add_middleware(&mut self, mw: HandlerObj) -> &mut Node {
		if self.mws.is_none() {
			self.mws = Some(vec![]);
		}
		if let Some(ref mut mws) = self.mws {
			mws.push(mw.clone());
		}
		self
	}

	fn value(&self) -> Option<HandlerObj> {
		match &self.value {
			None => None,
			Some(v) => Some(v.clone()),
		}
	}

	fn set_value(&mut self, value: HandlerObj) {
		self.value = Some(value);
	}

	fn add_child(&mut self, child_id: NodeId) {
		if self.children_count == MAX_CHILDREN {
			panic!("Can't add a route, children limit exceeded");
		}
		self.children[self.children_count] = child_id;
		self.children_count += 1;
	}
}

pub fn not_found() -> ResponseFuture {
	let mut response = Response::new(Body::empty());
	*response.status_mut() = StatusCode::NOT_FOUND;
	Box::pin(future::ok(response))
}

fn calculate_hash<T: Hash>(t: &T) -> u64 {
	let mut s = DefaultHasher::new();
	t.hash(&mut s);
	s.finish()
}

fn generate_path(route: &str) -> Vec<u64> {
	route
		.split('/')
		.skip(1)
		.map(|path| calculate_hash(&path))
		.collect()
}

fn collect_node_middleware(handlers: &mut Vec<HandlerObj>, node: &Node) {
	if let Some(ref mws) = node.mws {
		for mw in mws {
			handlers.push(mw.clone());
		}
	}
}

#[cfg(test)]
mod tests {

	use super::*;
	use futures::executor::block_on;

	struct HandlerImpl(u16);

	impl Handler for HandlerImpl {
		fn get(&self, _req: Request<Body>) -> ResponseFuture {
			let code = self.0;
			Box::pin(async move {
				let res = Response::builder()
					.status(code)
					.body(Body::default())
					.unwrap();
				Ok(res)
			})
		}
	}

	#[test]
	fn test_add_route() {
		let mut routes = Router::new();
		let h1 = Arc::new(HandlerImpl(1));
		let h2 = Arc::new(HandlerImpl(2));
		let h3 = Arc::new(HandlerImpl(3));
		routes.add_route("/v1/users", h1.clone()).unwrap();
		assert!(routes.add_route("/v1/users", h2.clone()).is_err());
		routes.add_route("/v1/users/xxx", h3.clone()).unwrap();
		routes.add_route("/v1/users/xxx/yyy", h3.clone()).unwrap();
		routes.add_route("/v1/zzz/*", h3.clone()).unwrap();
		assert!(routes.add_route("/v1/zzz/ccc", h2.clone()).is_err());
		routes
			.add_route("/v1/zzz/*/zzz", Arc::new(HandlerImpl(6)))
			.unwrap();
	}

	#[test]
	fn test_get() {
		let mut routes = Router::new();
		routes
			.add_route("/v1/users", Arc::new(HandlerImpl(101)))
			.unwrap();
		routes
			.add_route("/v1/users/xxx", Arc::new(HandlerImpl(103)))
			.unwrap();
		routes
			.add_route("/v1/users/xxx/yyy", Arc::new(HandlerImpl(103)))
			.unwrap();
		routes
			.add_route("/v1/zzz/*", Arc::new(HandlerImpl(103)))
			.unwrap();
		routes
			.add_route("/v1/zzz/*/zzz", Arc::new(HandlerImpl(106)))
			.unwrap();

		let call_handler = |url| {
			let task = async {
				let resp = routes
					.get(url)
					.unwrap()
					.next()
					.unwrap()
					.get(Request::new(Body::default()))
					.await
					.unwrap();
				resp.status().as_u16()
			};
			block_on(task)
		};

		assert_eq!(call_handler("/v1/users"), 101);
		assert_eq!(call_handler("/v1/users/xxx"), 103);
		assert!(routes.get("/v1/users/yyy").is_err());
		assert_eq!(call_handler("/v1/users/xxx/yyy"), 103);
		assert!(routes.get("/v1/zzz").is_err());
		assert_eq!(call_handler("/v1/zzz/1"), 103);
		assert_eq!(call_handler("/v1/zzz/2"), 103);
		assert_eq!(call_handler("/v1/zzz/2/zzz"), 106);
	}
}
