// Bitcoin secp256k1 bindings
// Written in 2014 by
//   Dawid Ciężarkiewicz
//   Andrew Poelstra
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

// This is a macro that routinely comes in handy
macro_rules! impl_array_newtype {
    ($thing:ident, $ty:ty, $len:expr) => {
        impl Copy for $thing {}

        impl $thing {
            #[inline]
            /// Converts the object to a raw pointer for FFI interfacing
            pub fn as_ptr(&self) -> *const $ty {
                let &$thing(ref dat) = self;
                dat.as_ptr()
            }

            #[inline]
            /// Converts the object to a mutable raw pointer for FFI interfacing
            pub fn as_mut_ptr(&mut self) -> *mut $ty {
                let &mut $thing(ref mut dat) = self;
                dat.as_mut_ptr()
            }

            #[inline]
            /// Returns the length of the object as an array
            pub fn len(&self) -> usize { $len }

            #[inline]
            /// Returns whether the object as an array is empty
            pub fn is_empty(&self) -> bool { false }
        }

        impl PartialEq for $thing {
            #[inline]
            fn eq(&self, other: &$thing) -> bool {
                &self[..] == &other[..]
            }
        }

        impl Eq for $thing {}

        impl Clone for $thing {
            #[inline]
            fn clone(&self) -> $thing {
                unsafe {
                    use std::intrinsics::copy_nonoverlapping;
                    use std::mem;
                    let mut ret: $thing = mem::uninitialized();
                    copy_nonoverlapping(self.as_ptr(),
                                        ret.as_mut_ptr(),
                                        mem::size_of::<$thing>());
                    ret
                }
            }
        }

        impl ::std::ops::Index<usize> for $thing {
            type Output = $ty;

            #[inline]
            fn index(&self, index: usize) -> &$ty {
                let &$thing(ref dat) = self;
                &dat[index]
            }
        }

        impl ::std::ops::Index<::std::ops::Range<usize>> for $thing {
            type Output = [$ty];

            #[inline]
            fn index(&self, index: ::std::ops::Range<usize>) -> &[$ty] {
                let &$thing(ref dat) = self;
                &dat[index]
            }
        }

        impl ::std::ops::Index<::std::ops::RangeTo<usize>> for $thing {
            type Output = [$ty];

            #[inline]
            fn index(&self, index: ::std::ops::RangeTo<usize>) -> &[$ty] {
                let &$thing(ref dat) = self;
                &dat[index]
            }
        }

        impl ::std::ops::Index<::std::ops::RangeFrom<usize>> for $thing {
            type Output = [$ty];

            #[inline]
            fn index(&self, index: ::std::ops::RangeFrom<usize>) -> &[$ty] {
                let &$thing(ref dat) = self;
                &dat[index]
            }
        }

        impl ::std::ops::Index<::std::ops::RangeFull> for $thing {
            type Output = [$ty];

            #[inline]
            fn index(&self, _: ::std::ops::RangeFull) -> &[$ty] {
                let &$thing(ref dat) = self;
                &dat[..]
            }
        }

        impl ::std::hash::Hash for $thing {
          fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
            state.write(&self.0)
            // for n in 0..self.len() {
            //   state.write_u8(self.0[n]);
            // }
          }
        }

        impl ::serialize::Decodable for $thing {
            fn decode<D: ::serialize::Decoder>(d: &mut D) -> Result<$thing, D::Error> {
                use serialize::Decodable;

                d.read_seq(|d, len| {
                    if len != $len {
                        Err(d.error("Invalid length"))
                    } else {
                        unsafe {
                            use std::mem;
                            let mut ret: [$ty; $len] = mem::uninitialized();
                            for i in 0..len {
                                ret[i] = try!(d.read_seq_elt(i, |d| Decodable::decode(d)));
                            }
                            Ok($thing(ret))
                        }
                    }
                })
            }
        }

        impl ::serialize::Encodable for $thing {
            fn encode<S: ::serialize::Encoder>(&self, s: &mut S)
                                               -> Result<(), S::Error> {
                self[..].encode(s)
            }
        }

        impl ::serde::Deserialize for $thing {
            fn deserialize<D>(d: &mut D) -> Result<$thing, D::Error>
                where D: ::serde::Deserializer
            {
                // We have to define the Visitor struct inside the function
                // to make it local ... all we really need is that it's
                // local to the macro, but this works too :)
                struct Visitor {
                    marker: ::std::marker::PhantomData<$thing>,
                }
                impl ::serde::de::Visitor for Visitor {
                    type Value = $thing;

                    #[inline]
                    fn visit_seq<V>(&mut self, mut v: V) -> Result<$thing, V::Error>
                        where V: ::serde::de::SeqVisitor
                    {
                        unsafe {
                            use std::mem;
                            let mut ret: [$ty; $len] = mem::uninitialized();
                            for i in 0..$len {
                                ret[i] = match try!(v.visit()) {
                                    Some(c) => c,
                                    None => return Err(::serde::de::Error::end_of_stream())
                                };
                            }
                            try!(v.end());
                            Ok($thing(ret))
                        }
                    }
                }

                // Begin actual function
                d.visit(Visitor { marker: ::std::marker::PhantomData })
            }
        }

        impl ::serde::Serialize for $thing {
            fn serialize<S>(&self, s: &mut S) -> Result<(), S::Error>
                where S: ::serde::Serializer
            {
                (&self.0[..]).serialize(s)
            }
        }
    }
}

macro_rules! impl_pretty_debug {
    ($thing:ident) => {
        impl ::std::fmt::Debug for $thing {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                try!(write!(f, "{}(", stringify!($thing)));
                for i in self[..].iter().cloned() {
                    try!(write!(f, "{:02x}", i));
                }
                write!(f, ")")
            }
        }
     }
}

macro_rules! impl_raw_debug {
    ($thing:ident) => {
        impl ::std::fmt::Debug for $thing {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                for i in self[..].iter().cloned() {
                    try!(write!(f, "{:02x}", i));
                }
                Ok(())
            }
        }
     }
}

macro_rules! map_vec {
  ($thing:expr, $mapfn:expr ) => {
    $thing.iter()
      .map($mapfn)
      .collect::<Vec<_>>();
  }
}
