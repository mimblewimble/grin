# Macros for Array Newtypes

The `grin_util` crate provides several macros for working with array newtypes - wrapper types around fixed-size arrays. These macros help implement common traits and functionality for these types.

## Available Macros

### `impl_array_newtype`

Implements standard array traits and behavior for newtype wrappers around fixed-size arrays. This includes:

- Methods like `as_ptr()`, `as_mut_ptr()`, `len()`, etc.
- Indexing via `Index` traits
- Comparison traits (`PartialEq`, `Eq`, `PartialOrd`, `Ord`)
- Common traits like `Clone`, `Copy`, and `Hash`

### `impl_array_newtype_encodable`

Implements serialization and deserialization support via Serde for newtype wrappers.

### `impl_array_newtype_show`

Implements the `Debug` trait for pretty-printing the array newtype.

### `impl_index_newtype`

Implements various indexing operations for the newtype. This is automatically called by `impl_array_newtype`.

## Usage Examples

```rust
// Define a newtype for a 32-byte array
pub struct ChainCode([u8; 32]);

// Implement standard array traits
impl_array_newtype!(ChainCode, u8, 32);

// Implement Debug formatting
impl_array_newtype_show!(ChainCode);

// Implement Serde serialization/deserialization
impl_array_newtype_encodable!(ChainCode, u8, 32);
```

## Notes on Feature Flags

With recent Rust versions, conditional compilation within macros is handled differently. The `serde` and other features are now defined at the crate level rather than inside the macros themselves, which prevents warnings about unexpected `cfg` conditions.
