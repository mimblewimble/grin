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

use fnv::FnvHashMap;

use std::{cmp, fmt};
use std::str;

error_chain!{}

/// Node types
///
/// Root created by the first insertion.
/// Static for non-wildcard path segment.
/// Param for path parameter segment leading by ":".
/// CatchAll for catchall segment leading by "*".
#[derive(Debug, PartialEq)]
pub enum NodeType {
	Root,
    Static,
	Param,
	CatchAll,
}

impl Default for NodeType {
    fn default() -> NodeType { NodeType::Static }
}

impl fmt::Display for NodeType {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			NodeType::Root => write!(f, "Root"),
			NodeType::Static => write!(f, "Static"),
			NodeType::Param => write!(f, "Param"),
            NodeType::CatchAll => write!(f, "CatchAll"),
		}
	}
}

/// Node of radix trie
#[derive(Default, Debug, PartialEq)]
pub struct Node {
    pub path: Vec<u8>,
    pub value: Option<usize>,
    pub node_type: NodeType,
    pub has_wildcard_child: bool,
    pub child_indeces: Vec<u8>,
    pub children: Vec<Option<Box<Node>>>,
}

impl Node {
    /// Constructs Node.
    pub fn new() -> Node {
        Node {
            path: Vec::new(),
            value: None,
            node_type: NodeType::Static,
            has_wildcard_child: false,
            child_indeces: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Inserts an endpoint.
    ///
    /// Tracks down existing nodes of radix trie util portion of endpoint matched.
    /// If the common segment is found, the endpoint is splitted and the remaining part is stored in a new node.
    /// The index of vec of handlers is stored in the leaf node of the endpoint.
    ///
    /// # Panics
    ///
    /// when an endpoint has wildcard ("*", ":") as part of wildcard identifier, such as "/:p:arm/" and "/*cat:chall/".
    /// when two endpoints or more have wildcard at the same path component.
    ///    For example, when trying to insert both "/a/b/:c/d" and "/a/b/:e".
    ///
    /// # Errors
    ///
    /// when trying to insert duplicate endpoint.
    ///
    pub fn insert(&mut self, endpoint: &[u8], value_index: usize) -> Result<()> {
        let mut node = self;
        let path = endpoint;
        let path_len = path.len();
        let mut index = 0;

        loop {
            let (matched_len, segment_len) = node.match_paths(&path[index..]);

            if matched_len == 0 {
                // check if wildcard exists.
                let (wildcard_index, name_len) = find_wildcard(path, index);
                
                // root node created by the first insertion.
                if segment_len == 0 && node.path.len() == 0 {
                    node.node_type = NodeType::Root;
                    if name_len == 0 {
                        node.path = path.to_vec();
                        node.value = Some(value_index);
                        return Ok(());
                    }
                    node.path = path[..wildcard_index].to_vec();
                    index += wildcard_index;
                    continue;
                } else if segment_len > 0 {
                    // borrow checker complains.
                    // node transfers.
                    let curr = node;
                    index = curr.add_child(path, index, value_index);
                    if index < path_len {
                        let tmp = curr.children.last_mut().unwrap();
                        node = &mut **tmp.as_mut().unwrap();
                    } else {
                        return Ok(());
                    }
                } else {
                    // should not reach here, but...
                    panic!("Invalid endpoint - {:?}", str::from_utf8(&path));
                }
            }
            // partially matched
            else if node.path.len() > matched_len {
                // split node
                node.split(path, value_index, index, matched_len);
                if node.path.len() < path_len {
                    continue;
                }
                return Ok(());
            }
            // fully matched up to full length of node path (node.path.len == matched_len)
            else {
                // path.len > node.path len
                index += matched_len; 
                if path_len > index {
                    let c = path[index];
                    let mut child_index = 0;
                    let curr = node;
                    let child_count = curr.child_indeces.len();
                    // check if there is any matching child index.
                    while child_index < child_count {
                        if c == curr.child_indeces[child_index] {
                            break;
                        }
                        child_index += 1;
                    }
                    // if there is any matched child index, then move to the child and continue.
                    // otherwise, insert new child.
                    if child_index < child_count {
                        node = &mut **curr.children[child_index].as_mut().unwrap();
                        continue;
                    } else {
                        index = curr.add_child(path, index, value_index);
                        if index < path_len {
                            let tmp = curr.children.last_mut().unwrap();
                            node = &mut **tmp.as_mut().unwrap();
                        } else {
                            return Ok(());
                        }
                    }
                } else {
                    // a duplicate endpoint reaches here.
                    // Or reach here when inserting an endpoint that is a part of already inserted endpoint.
                    // i.e. inserting "/v1" after "/v1/status" inserted.
                    //bail!("Api server error while inserting endpoints: the endpoint already exists. - {:?}", str::from_utf8(path).unwrap().to_string());                    
                    return Ok(());
                }
            }
        }
    }

    /// Look up the index of handler for a provided path.
    ///
    /// Is called to get the index of vector storage of handlers and 
    /// path parameters if any for a provided endpoint.
    /// Returns tuple of index of handler and path parameters if any.
    /// Returns None when no match found.
    pub fn lookup(&self, endpoint: &[u8]) -> Option<(usize, FnvHashMap<String, String>)> {

        let mut node = self;
        let mut path = endpoint;
        let mut params: FnvHashMap<String, String> = FnvHashMap::with_hasher(Default::default());

        // path lengh based lookup
        loop {
            match node.path.len() < path.len() {
                // node.path.len < path.len
                true => {
                    // find the length of common path component of both node.path and path argument.
                    let path_match = |a: &[u8], b: &[u8]| {
                        let limit = cmp::min(a.len(), b.len());
                        let mut count: usize = 0;
                        while count < limit && a[count] == b[count] {
                            count += 1;
                        }
                        count
                    };

                    let matched_len = path_match(&node.path, path);
                
                    match matched_len == node.path.len() {
                        // matched, move to child
                        true => {
                            // take out the matched part
                            path = &path[matched_len..];
                            // if children are not wildcard nodes,
                            // move to the child.
                            if !node.has_wildcard_child {
                                // borrow checker complains.
                                // so the node transfers.
                                let curr = node;
                                let c = path[0];
                                let mut child_index = 0;
                                let child_count = curr.child_indeces.len();
                                // check if there is any matching child index.
                                while child_index < child_count {
                                    if c == curr.child_indeces[child_index] {
                                        break;
                                    }
                                    child_index += 1;
                                }
                                // if there is any matched child index, then move to the child and continue.
                                // otherwise, not found and return.
                                if child_index < child_count {
                                    node = &**curr.children[child_index].as_ref().unwrap();
                                    continue;
                                } else {
                                    return None;
                                }
                            } 
                            // node has wild card as one of children.
                            else {
                                // move to the wildcard child
                                let curr = node;
                                let c = path[0];
                                let mut idx = 0;
                                let mut static_index = 0;
                                let mut static_found = false;
                                let mut param_index = 0;
                                let mut param_found = false;
                                let mut catchall_index = 0;
                                let mut catchall_found = false;
                                let child_count = curr.child_indeces.len();
                                // check if there is any matching child index.
                                while idx < child_count {
                                    let k = curr.child_indeces[idx];
                                    if c == k {
                                        static_index = idx;
                                        static_found = true;
                                    }
                                    if k == b':' {
                                        param_index = idx;
                                        param_found = true;
                                    }
                                    if k == b'*' {
                                        catchall_index = idx;
                                        catchall_found = true;
                                    }
                                    idx += 1;
                                }
                                if static_found {
                                    let mut matched = true;
                                    // the node has both of static component and wildcard.
                                    // need full path check. 
                                    // if there is a child of static component exactly matching,
                                    // move on to the child.
                                    if param_found || catchall_found {
                                        matched = curr.match_static_child(path, static_index);
                                    }
                                    if matched {
                                        node = &**curr.children[static_index].as_ref().unwrap();
                                        continue;
                                    }
                                }
                                // otherwise, move to a wildcard child.
                                if param_found {
                                    node = &**curr.children[param_index].as_ref().unwrap();
                                } else {
                                    node = &**curr.children[catchall_index].as_ref().unwrap();
                                }

                                // get value of wildcard
                                let get_value_len = |p: &[u8]| {
                                    let mut index = 0;
                                    while index < p.len() && p[index] != b'/' {
                                        index += 1;
                                    }
                                    index
                                };
                                let value_len = get_value_len(path);

                                match node.node_type {
                                    NodeType::Param => {
                                        if value_len > 0 {
                                            let key = String::from_utf8_lossy(&node.path[1..]).into_owned();
                                            let value = String::from_utf8_lossy(&path[..value_len]).into_owned();
                                            params.insert(key, value);
                                        }
                                        // check if more in path after the wildcard
                                        if value_len < path.len() {
                                            // slice path
                                            path = &path[value_len..];
                                            // borrow checker complains.
                                            // so the node transfers.
                                            let curr = node;
                                            let c = path[0];
                                            let mut child_index = 0;
                                            let child_count = curr.child_indeces.len();
                                            // check if there is any matching child index.
                                            while child_index < child_count {
                                                if c == curr.child_indeces[child_index] {
                                                    break;
                                                }
                                                child_index += 1;
                                            }
                                            // if there is any matched child index, then move to the child and continue.
                                            // otherwise, not found and return.
                                            if child_index < child_count {
                                                node = &**curr.children[child_index].as_ref().unwrap();
                                                continue;
                                            } else {
                                                // no match found. hits the dead end.
                                                return None;
                                            }
                                        }
                                        // wildcard is the last part of path.
                                        else {
                                            if node.value.is_some() {
                                                return Some((node.value.unwrap(), params));
                                            } else {
                                                // no match found. hits the dead end.
                                                return None;                                                    
                                            }
                                        }
                                    },
                                    NodeType::CatchAll => {
                                        let key = String::from_utf8_lossy(&node.path).into_owned();
                                        let value = String::from_utf8_lossy(path).into_owned();
                                        params.insert(key, value);
                                        return Some((node.value.unwrap(), params));
                                    },
                                    _ => {
                                        if path.len() > 0 {
                                            continue;
                                        }
                                        // no match found. hits the dead end.
                                        return None;
                                    }
                                }
                            }
                        },
                        // no match found. hits the dead end.
                        false => {
                            return None;
                        }
                    } // match
                },
                // node.path.len >= path.len
                false => {
                    // node path == path
                    if match_path(&node.path, path) {
                        return Some((node.value.unwrap(), params));
                    }
                    // node path > path
                    return None;
                }
            }
        }   // loop
    }

    // split the path of current node only if the matching segment is shorter than the path of current node.
    // panic if adding another wildcard for the position of existing wildcard.
    // e.g. for existing path, /v1/peers/:name/ban, add /v1/peers/:ip/ban
    // :name conflicts with :ip, and panic.
    fn split(&mut self, path: &[u8], value_index: usize, index: usize, matched_len: usize) {

        let node = self;

        // only one wildcard is allowed at the path position.
        if (path[index] == b':' || path[index] == b'*') && (node.node_type == NodeType::Param || node.node_type == NodeType::CatchAll) {
            panic!(format!("Only one wildcard allowed at the path position. - conflict between {:?} and {:?} in {:?}", str::from_utf8(&node.path).as_ref().unwrap().to_string(), str::from_utf8(path).as_ref().unwrap().to_string(), str::from_utf8(&path[index..]).as_ref().unwrap().to_string()));    
        }

        // split node's path
        let prefix = node.path[0..matched_len].to_vec();
        let suffix = node.path[matched_len..].to_vec();
        let has_wildcard_child = node.has_wildcard_child;
        let suffix_index = suffix[0];
        let mut child_indeces = Vec::new();
        let mut children = Vec::new();

        let max = node.child_indeces.len();
        for i in 0..max {
            child_indeces.push(node.child_indeces[i]);
            children.push(node.children[i].take());
        }

        // take children and handler of node
        let curr_value = node.value.take();

        // the remaining part of path after match
        let child = Some(Box::new(Node {
            path: suffix,
            value: curr_value,
            has_wildcard_child: has_wildcard_child,
            node_type: NodeType::Static,
            child_indeces: child_indeces,
            children: children,
        }));
        node.path = prefix;
        node.child_indeces = vec![suffix_index];
        node.children = vec![child];
        node.has_wildcard_child = false;
        node.value = Some(value_index);
    }

    // add a child of current node.
    // return the path index moved down next to the path of new child.
    // new child can be:
    // 1. a static path component before wildcard,
    // 2. a wildcard, or
    // 3. a static path component all the way down to the end of path.
    fn add_child(&mut self, path: &[u8], skip: usize, value_index: usize) -> usize {
        let mut index = skip;
        let child_initial;
        // check if wildcard exists in path.
        let (wildcard_index, name_len) = find_wildcard(path, index);

        let child: Option<Box<Node<>>>;
                        
        // wildcard exists.
        if name_len > 0 {
            // when a static path component exists before wildcard.
            // e.g. static/:param
            if wildcard_index > index {
                child = Some(Box::new(Node {
                    path: path[index..wildcard_index].to_vec(),
                    value: None,
                    has_wildcard_child: true,
                    node_type: NodeType::Static,
                    child_indeces: Vec::new(),
                    children: Vec::new(),
                }));
                child_initial = path[index];
                index = wildcard_index;
            }
            // when wildcard starts
            // e.g. :param
            else {
                let end_index = wildcard_index + name_len;
                let child_value: Option<usize> = if end_index == path.len()
                                        { Some(value_index) }
                                    else
                                        { None };
                let node_type = if path[wildcard_index] == b':'
                                    { NodeType::Param }
                                else 
                                    { NodeType::CatchAll };
                child = Some(Box::new(Node {
                    path: path[wildcard_index..end_index].to_vec(),
                    value: child_value,
                    has_wildcard_child: false,
                    node_type: node_type,
                    child_indeces: Vec::new(),
                    children: Vec::new(),
                }));
                self.has_wildcard_child = true;
                index = end_index;
                child_initial = path[wildcard_index];
            }
        }
        // no wildcard exists.
        else {
            child = Some(Box::new(Node {
                path: path[index..].to_vec(),
                value: Some(value_index),
                has_wildcard_child: self.has_wildcard_child,
                node_type: NodeType::Static,
                child_indeces: Vec::new(),
                children: Vec::new(),
            }));
            child_initial = path[index];
            index = path.len();
        }        
        self.child_indeces.push(child_initial);
        self.children.push(child);
        index
    }

    // check if there is matching segment between the path of current node and the passed path.
    // return the starting index and length of matching segment.
    fn match_paths(&mut self, path: &[u8]) -> (usize, usize) {
        let key = &self.path;
        let limit = cmp::min(key.len(), path.len());
        let mut count: usize = 0;
        while count < limit && key[count] == path[count] {
            count += 1;
        }
        (count, limit)
    }

    // called only if a node has both of static component and wildcard.
    // return bool value. true if the child of static type matches. otherwise, false.
    fn match_static_child (&self, path: &[u8], child_index: usize) -> bool {
        let tmp = &self.children[child_index];
        let node = &**tmp.as_ref().unwrap();
        let mut idx = 0;
        let pth = &node.path;
        let len = pth.len();
        while idx < len {
            if pth[idx] != path[idx] {
                break;
            }
            idx += 1;
        }
        // the child is matching with path component.
        if idx == len {
            return true;
        }
        return false;
    }
 
}

// check if there is wildcard in the path from index position.
// return index of widlcard in the path and length of wildcard name.
fn find_wildcard(path: &[u8], index: usize) -> (usize, usize) {
    let path_len = path.len();
    // check if wildcard exists.
    let mut wildcard_index = index;
    while wildcard_index < path_len && path[wildcard_index] != b':' && path[wildcard_index] != b'*' {
        wildcard_index += 1;
    }
    // path starts with wildcard or
    // contains wildcard in between the start of current path segment and the end of it.
    let mut name_len = 0;
    let mut wildcard = false;
    if path[index] == b':' || path[index] == b'*' || (wildcard_index > index && wildcard_index < path_len) {
        wildcard = true;
        let mut idx = wildcard_index + 1;
        while idx < path_len && path[idx] != b'/' {
            if path[idx] == b':' || path[idx] == b'*' {
                panic!(format!("API endpoint error: wildcard cannot have another wildcard inside. - {:?}", str::from_utf8(path).as_ref().unwrap().to_string()));
            }
            idx += 1;
        }
        name_len = idx - wildcard_index;  
    }
    if wildcard && path[wildcard_index] != b'*' && name_len < 2 {
        panic!(format!("API endpoint error: wildcard name is reqired. - {:?}", str::from_utf8(path).as_ref().unwrap().to_string()));
    }
    (wildcard_index, name_len)
}

fn match_path (a: &[u8], b: &[u8]) -> bool {
    if a.len() == 0 || b.len() == 0 || a.len() != b.len() {
        return false;
    }
    let mut count: usize = 0;
    while count < a.len() && a[count] == b[count] {
        count += 1;
    }
    (count == a.len())
}

#[cfg(test)]
mod tests {
    use std::str;

    use router::radixtrie::Node;

    #[test]
    fn test_static_path() {
        let mut root = Node::new();
        let _ = root.insert("/v1/chain/outputs/byids".as_bytes(), 0);
        let _ = root.insert("/v1".as_bytes(), 1);
        let (index, params) = root.lookup("/v1".as_bytes()).unwrap();
        assert_eq!(1, index);
        assert_eq!(0, params.len());
        assert_eq!(root.lookup("/v1/chain/outputs/something".as_bytes()), None);
    }

    #[test]
    fn test_wildcard_path() {
        let mut root = Node::new();
        let _ = root.insert("/v1/peers/:ip/ban".as_bytes(), 0);
        let _ = root.insert("/v1/peers/:ip/unban".as_bytes(), 1);

        let (_index, params) = root.lookup("/v1/peers/127.0.0.1:13413/ban".as_bytes()).unwrap();
        assert_eq!(1, params.len());
        for (k, v) in &params {
            assert_eq!(str::from_utf8(k.as_bytes()).unwrap(), "ip".to_string());
            assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "127.0.0.1:13413".to_string());                
        }
        let (_index, params) = root.lookup("/v1/peers/127.0.0.2:13415/unban".as_bytes()).unwrap();
        assert_eq!(1, params.len());
        for (k, v) in &params {
            assert_eq!(str::from_utf8(k.as_bytes()).unwrap(), "ip".to_string());
            assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "127.0.0.2:13415".to_string());                
        }
    }

    #[test]
    fn test_static_param_mixed_path() {
        let mut root = Node::new();
        let _ = root.insert("/v1/peers/:ip/ban".as_bytes(), 0);
        let _ = root.insert("/v1/peers/:ip/unban".as_bytes(), 1);
        let _ = root.insert("/v1/peers/connected".as_bytes(), 2);

        let (_index, params) = root.lookup("/v1/peers/127.0.0.1:13413/ban".as_bytes()).unwrap();
        assert_eq!(1, params.len());

        let (_index2, params2) = root.lookup("/v1/peers/127.0.0.2:13415/unban".as_bytes()).unwrap();
        for (k, v) in &params2 {
            assert_eq!(str::from_utf8(k.as_bytes()).unwrap(), "ip".to_string());
            assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "127.0.0.2:13415".to_string());                
        }
        
        assert_eq!(root.lookup("/v1/peers/127.0.0.2:13415/other".as_bytes()), None);
    }

    #[test]
    fn test_static_catchall_mixed_path() {
        let mut root = Node::new();
        let _ = root.insert("/v1/peers/connected".as_bytes(), 0);
        let _ = root.insert("/v1/peers/*".as_bytes(), 1);

        let (index, params) = root.lookup("/v1/peers/127.0.0.1:13413/ban".as_bytes()).unwrap();
        assert_eq!(1, index);
        assert_eq!(1, params.len());
        for (k, v) in &params {
            assert_eq!(str::from_utf8(k.as_bytes()).unwrap(), "*".to_string());
            assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "127.0.0.1:13413/ban".to_string());                
        }

        let (index2, params2) = root.lookup("/v1/peers/connected".as_bytes()).unwrap();
        assert_eq!(0, index2);
        assert_eq!(0, params2.len());
    }

    #[test]
    fn test_static_param_catchall_mixed_path() {
        let mut root = Node::new();
        let _ = root.insert("/v1/peers/connected".as_bytes(), 0);
        let _ = root.insert("/v1/peers/:ip/ban".as_bytes(), 1);
        let _ = root.insert("/v1/peers/*".as_bytes(), 2);

        let (index, params) = root.lookup("/v1/peers/127.0.0.1:13413/ban".as_bytes()).unwrap();
        assert_eq!(1, index);
        for (k, v) in &params {
            assert_eq!(str::from_utf8(k.as_bytes()).unwrap(), "ip".to_string());
            assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "127.0.0.1:13413".to_string());                
        }
            
        let (index2, params2) = root.lookup("/v1/peers/connected".as_bytes()).unwrap();
        assert_eq!(0, index2);
        assert_eq!(0, params2.len());

        assert_eq!(root.lookup("/v1/peers/status".as_bytes()), None);

        assert_eq!(root.lookup("/v1/peers/127.0.0.2:13415/unban".as_bytes()), None);
    }

    #[test]
    fn test_multiple_param_path() {
        let mut root = Node::new();
        let _ = root.insert("/v1/peers/:name/:ip/ban".as_bytes(), 0);

        let (index, params) = root.lookup("/v1/peers/johndoe/127.0.0.1:13413/ban".as_bytes()).unwrap();
        assert_eq!(0, index);
        assert_eq!(2, params.len());
        for (k, v) in &params {
            if str::from_utf8(k.as_bytes()).unwrap() == "name".to_string() {
                assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "johndoe".to_string());
            }
            if str::from_utf8(k.as_bytes()).unwrap() == "ip".to_string() {
                assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "127.0.0.1:13413".to_string());
            }
        }
    }

    #[test]
    #[should_panic]
    fn test_wildcard_conflict() {
        let mut root = Node::new();
        let _ = root.insert("/v1/peers/:name/:ip/ban".as_bytes(), 1);
        let _ = root.insert("/v1/peers/:id/:ip/ban".as_bytes(), 2);
    }                    
}