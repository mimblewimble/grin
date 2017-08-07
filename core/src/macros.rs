// Copyright 2016 The Grin Developers
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

//! Generic macros used here and there to simplify and make code more
//! readable.

/// Eliminates some of the verbosity in having iter and collect
/// around every map call.
#[macro_export]
macro_rules! map_vec {
  ($thing:expr, $mapfn:expr ) => {
    $thing.iter()
      .map($mapfn)
      .collect::<Vec<_>>();
  }
}

/// Same as map_vec when the map closure returns Results. Makes sure the
/// results are "pushed up" and wraps with a try.
#[macro_export]
macro_rules! try_map_vec {
  ($thing:expr, $mapfn:expr ) => {
    try!($thing.iter()
      .map($mapfn)
      .collect::<Result<Vec<_>, _>>());
  }
}

/// Eliminates some of the verbosity in having iter and collect
/// around every fitler_map call.
#[macro_export]
macro_rules! filter_map_vec {
  ($thing:expr, $mapfn:expr ) => {
    $thing.iter()
      .filter_map($mapfn)
      .collect::<Vec<_>>();
  }
}

/// Allows the conversion of an expression that doesn't return anything to one
/// that returns the provided identifier.
/// Example:
///   let foo = vec![1,2,3]
///   println!(tee!(foo, foo.append(vec![3,4,5]))
#[macro_export]
macro_rules! tee {
  ($thing:ident, $thing_expr:expr) => {
    {
    $thing_expr;
    $thing
    }
  }
}

#[macro_export]
macro_rules! try_to_o {
  ($trying:expr) => {{
    let tried = $trying;
    if let Err(e) = tried {
      return Some(e);
    }
    tried.unwrap()
  }}
}

/// Eliminate some of the boilerplate of deserialization (package ser) by
/// passing just the list of reader function.
/// Example before:
///   let foo = try!(reader.read_u64());
///   let bar = try!(reader.read_u32());
/// Example after:
///   let (foo, bar) = ser_multiread!(reader, read_u64, read_u32);
#[macro_export]
macro_rules! ser_multiread {
  ($rdr:ident, $($read_call:ident),*) => {
    ( $(try!($rdr.$read_call())),* )
  }
}

/// Eliminate some of the boilerplate of serialization (package ser) by
/// passing directly pairs of writer function and data to write.
/// Example before:
///   try!(reader.write_u64(42));
///   try!(reader.write_u32(100));
/// Example after:
///   ser_multiwrite!(writer, [write_u64, 42], [write_u32, 100]);
#[macro_export]
macro_rules! ser_multiwrite {
  ($wrtr:ident, $([ $write_call:ident, $val:expr ]),* ) => {
    $( try!($wrtr.$write_call($val)) );*
  }
}
