//! Contextual relaxation of the [`crate::MAX_ARRAY_LENGTH`] decode guard.
//!
//! The guard protects decoders from oversized allocations: hostile data can
//! declare a huge array length and force the reader to allocate before any
//! payload is validated. Files from trusted sources, such as local schematics
//! the server wrote itself, legitimately exceed the limit; decoding them runs
//! inside [`unbounded`], which suspends the guard for the current thread only
//! and restores it afterwards.

use std::cell::Cell;

thread_local! {
    static SCOPE_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Runs `f` with the [`crate::MAX_ARRAY_LENGTH`] decode guard disabled.
///
/// Only use this for bytes from a trusted source (a local file the server
/// wrote itself), never with network or client-controlled data: while active,
/// a declared array length is trusted and allocated as-is.
pub fn unbounded<R>(f: impl FnOnce() -> R) -> R {
    SCOPE_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _scope = Scope;
    f()
}

struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        SCOPE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Whether `len` currently exceeds the decode guard.
pub(crate) fn exceeds(len: usize) -> bool {
    SCOPE_DEPTH.with(Cell::get) == 0 && len > crate::MAX_ARRAY_LENGTH
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::nbt_compress::{read_gzip_compound_tag, write_gzip_compound_tag_to_bytes};
    use crate::{NbtCompound, NbtTag};

    fn large_payload_root() -> NbtCompound {
        let mut root = NbtCompound::new();
        root.put(
            "data",
            NbtTag::ByteArray(vec![0i8; crate::MAX_ARRAY_LENGTH + 1].into_boxed_slice()),
        );
        root
    }

    fn read_back(root: NbtCompound) -> Result<NbtCompound, crate::Error> {
        let bytes = write_gzip_compound_tag_to_bytes(root).expect("serializes");
        read_gzip_compound_tag(&mut Cursor::new(bytes))
    }

    #[test]
    fn rejects_large_arrays_by_default() {
        let error = read_back(large_payload_root()).expect_err("guard stays on by default");
        assert!(error.to_string().contains("Length too large"));
        assert!(super::exceeds(crate::MAX_ARRAY_LENGTH + 1));
    }

    #[test]
    fn accepts_large_arrays_inside_scope() {
        let compound = super::unbounded(|| read_back(large_payload_root())).expect("reads back");
        assert_eq!(
            compound.get_byte_array("data").map(<[i8]>::len),
            Some(crate::MAX_ARRAY_LENGTH + 1)
        );

        // The scope must not leak after the closure returns.
        assert!(super::exceeds(crate::MAX_ARRAY_LENGTH + 1));
        let error = read_back(large_payload_root()).expect_err("guard is restored");
        assert!(error.to_string().contains("Length too large"));
    }
}
