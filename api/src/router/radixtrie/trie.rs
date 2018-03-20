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

use std::ascii::AsciiExt;
use std::{mem, str};
use fnv::FnvHashMap;

use super::TrieLookup;
use super::node::Node;

error_chain!{}

/// Trie Builder
///
/// Contains the reference to the root of radix trie and vec of values of type T. 
#[derive(Debug)]
pub struct TrieBuilder<T> {
    root: Option<Node>,
    values: Vec<T>,
}

impl<T> Default for TrieBuilder<T> {
    fn default() -> Self {
        TrieBuilder {
            root: None,
            values: vec![],
        }
    }
}

impl<T> TrieBuilder<T> {
    /// Inserts a new endpoint into radix trie.
    ///
    /// Call insert method of the root of radix trie to inserts a new endpoint.
    /// When calling insert method of the root, the entire path of endpoint and
    /// the index of vec of values for the endpoint are passed.
    ///
    /// On success of insertion, value is stored at the index position of vec of values.
    /// Refer to Builder for specific data structure for T type.
    pub fn insert(&mut self, endpoint: &str, value: T) -> Result<()> {
        if !endpoint.is_ascii() {
            panic!("Router error: url of endpoint contains invalid characters. {:?}", endpoint);
        }

        if self.root.is_none() {
            self.root = Some(Node::new());
        }

        // to avoid re-assignment error on node at this level
        // inserting is done at the node level.
        let node = self.root.as_mut().unwrap();
        let value_index = self.values.len();
        if let Err(e) = node.insert(endpoint.as_bytes(), value_index) {
            return Err(e.to_string().into());
        }
        self.values.push(value);
        Ok(())
    }

    /// Converts TrieBuilder data into Trie.
    ///
    /// Clones and diassociate TrieBuilder, and
    /// creates and returns a new Trie with cloned data from TrieBuilder.
    pub fn into_trie(&mut self) -> Result<Trie<T>> {
        // Clones and disassociate TrieBuilder.
        let TrieBuilder {
            root,
            values,
        } = mem::replace(self, Default::default());
        Ok(Trie {
            root,
            values,
        })
    }
      
}

/// Radix Trie for router.
///
/// Trie contains the root and the values.
/// The root is the first node of radix trie and
/// the values contain data of type T that are used to retrieve Handlers for endpoints.
#[derive(Debug)]
pub struct Trie<T> {
    root: Option<Node>,
    values: Vec<T>,
}

impl<T> Trie<T> {
    pub fn builder() -> TrieBuilder<T> {
        Default::default()
    }
}

impl<'a, T> TrieLookup<'a, T> for Trie<T> {
    type Params = FnvHashMap<String, String>;

    fn lookup(&self, path: &str) -> Option<(&T, Self::Params)> {
        if self.root.is_some() {
            let (index, params) = match self.root.as_ref().unwrap().lookup(path.as_bytes()) {
                Some((index, params)) => (index, params),
                None => return None,
            };
            if self.values.len() >= index {
                return Some((&self.values[index], params));
            }
            return None;
        }
        return None;        
    }
}

#[cfg(test)]
mod tests {
    use fnv::FnvHashMap;
    use hyper::Method;
    use std::str;

    use router::radixtrie::trie::Trie;
    use router::radixtrie::TrieLookup;

    #[test]
    fn test_trie_builder() {
        let mut trie_builder = Trie::<FnvHashMap<Method, usize>>::builder();
        let endpoint = "/v1/peers/:ip";
        let mut methods: FnvHashMap<Method, usize> = FnvHashMap::with_hasher(Default::default());

        methods.insert(Method::Get, 1);
        let _ = trie_builder.insert(&endpoint, methods);

        let trie = trie_builder.into_trie().unwrap();
        let (methods, params) = trie.lookup("/v1/peers/127.0.0.1:13413").unwrap();
        for (k, v) in methods {
            assert_eq!(Method::Get, *k);
            assert_eq!(1, *v);                
        }
        assert_eq!(1, params.len());
        for (k, v) in &params {
            assert_eq!(str::from_utf8(k.as_bytes()).unwrap(), "ip".to_string());
            assert_eq!(str::from_utf8(v.as_bytes()).unwrap(), "127.0.0.1:13413".to_string());                
        }
    }

    #[test]
    #[should_panic]
    fn test_invalid_format() {
        let mut trie_builder = Trie::<FnvHashMap<Method, usize>>::builder();
        let endpoint = "Grüße, Jürgen ❤";
        let mut methods: FnvHashMap<Method, usize> = FnvHashMap::with_hasher(Default::default());

        methods.insert(Method::Get, 1);
        let _ = trie_builder.insert(&endpoint, methods);
    }                    
}
