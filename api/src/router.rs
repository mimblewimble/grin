use hyper::{Body, Method, Request, Response, StatusCode};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use util::LOGGER;

lazy_static! {
	static ref WILDCARD_HASH: u64 = calculate_hash(&"*");
	static ref WILDCARD_STOP_HASH: u64 = calculate_hash(&"**");
}

pub trait Handler {
	fn get(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn post(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn put(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn patch(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn delete(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn head(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn options(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn trace(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}

	fn connect(&self, _req: Request<Body>) -> Response<Body> {
		not_found()
	}
}
#[derive(Fail, Debug)]
pub enum RouterError {
	#[fail(display = "Route already exists")]
	RouteAlreadyExists,
	#[fail(display = "Route not found")]
	RouteNotFound,
	#[fail(display = "Value not found")]
	NoValue,
}

#[derive(Clone)]
pub struct Router {
	nodes: Vec<Node>,
}

#[derive(Debug, Clone, Copy)]
struct NodeId(usize);

const MAX_CHILDREN: usize = 16;

type HandlerObj = Box<Handler>;

#[derive(Clone)]
struct Node {
	key: u64,
	value: Option<Arc<HandlerObj>>,
	children: [NodeId; MAX_CHILDREN],
	children_count: usize,
}

impl Router {
	pub fn new() -> Router {
		let root = Node::new(calculate_hash(&""), None);
		let mut nodes = vec![];
		nodes.push(root);
		Router { nodes }
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

	pub fn add_route(&mut self, route: &'static str, value: HandlerObj) -> Result<(), RouterError> {
		let keys = generate_path(route);
		let mut node_id = self.root();
		for key in keys {
			node_id = self.find(node_id, key)
				.unwrap_or_else(|| self.add_empty_node(node_id, key));
		}
		match self.node(node_id).value() {
			None => {
				self.node_mut(node_id).set_value(value);
				Ok(())
			}
			Some(_) => Err(RouterError::RouteAlreadyExists),
		}
	}

	pub fn get(&self, path: &str) -> Result<Arc<HandlerObj>, RouterError> {
		let keys = generate_path(path);
		let mut node_id = self.root();
		for key in keys {
			node_id = self.find(node_id, key).ok_or(RouterError::RouteNotFound)?;
			if self.node(node_id).key == *WILDCARD_STOP_HASH {
				debug!(LOGGER, "ROUTER stop card");
				break;
			}
		}
		self.node(node_id).value().ok_or(RouterError::NoValue)
	}

	pub fn handle(&self, req: Request<Body>) -> Response<Body> {
		match self.get(req.uri().path()) {
			Err(_) => not_found(),
			Ok(h) => match req.method() {
				&Method::GET => h.get(req),
				&Method::POST => h.post(req),
				&Method::PUT => h.put(req),
				&Method::DELETE => h.delete(req),
				&Method::PATCH => h.patch(req),
				&Method::OPTIONS => h.options(req),
				&Method::CONNECT => h.connect(req),
				&Method::TRACE => h.trace(req),
				&Method::HEAD => h.head(req),
				_ => not_found(),
			},
		}
	}
}

impl Node {
	fn new(key: u64, value: Option<Arc<HandlerObj>>) -> Node {
		Node {
			key,
			value,
			children: [NodeId(0); MAX_CHILDREN],
			children_count: 0,
		}
	}

	fn value(&self) -> Option<Arc<HandlerObj>> {
		match &self.value {
			None => None,
			Some(v) => Some(v.clone()),
		}
	}

	fn set_value(&mut self, value: HandlerObj) {
		self.value = Some(Arc::new(value));
	}

	fn add_child(&mut self, child_id: NodeId) {
		if self.children_count == MAX_CHILDREN {
			panic!("Can't add a route, children limit exceeded");
		}
		self.children[self.children_count] = child_id;
		self.children_count += 1;
	}
}

pub fn not_found() -> Response<Body> {
	let mut response = Response::new(Body::empty());
	*response.status_mut() = StatusCode::NOT_FOUND;
	response
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

#[cfg(test)]
mod tests {

	use super::*;

	#[test]
	fn test_add_route() {
		let mut routes = Router::new();
		routes.add_route("/v1/users", 1).unwrap();
		assert!(routes.add_route("/v1/users", 2).is_err());
		routes.add_route("/v1/users/xxx", 3).unwrap();
		routes.add_route("/v1/users/xxx/yyy", 3).unwrap();
		routes.add_route("/v1/zzz/*", 3).unwrap();
		assert!(routes.add_route("/v1/zzz/ccc", 2).is_err());
		routes.add_route("/v1/zzz/*/zzz", 6).unwrap();
	}

	#[test]
	fn test_get() {
		let mut routes = Router::new();
		routes.add_route("/v1/users", 1).unwrap();
		routes.add_route("/v1/users/xxx", 3).unwrap();
		routes.add_route("/v1/users/xxx/yyy", 3).unwrap();
		routes.add_route("/v1/zzz/*", 3).unwrap();
		routes.add_route("/v1/zzz/*/zzz", 6).unwrap();

		assert_eq!(routes.get("/v1/users").unwrap(), Arc::new(1));
		assert_eq!(routes.get("/v1/users/xxx").unwrap(), Arc::new(3));
		assert!(routes.get("/v1/users/yyy").is_err());
		assert_eq!(routes.get("/v1/users/xxx/yyy").unwrap(), Arc::new(3));
		assert!(routes.get("/v1/zzz").is_err());
		assert_eq!(routes.get("/v1/zzz/1").unwrap(), Arc::new(3));
		assert_eq!(routes.get("/v1/zzz/2").unwrap(), Arc::new(3));
		assert_eq!(routes.get("/v1/zzz/2/zzz").unwrap(), Arc::new(6));
	}
}
