//! Ironswift change: an `ArrayBuffer`'s bytes are either its own or memory the embedder owns
//! (a WebAssembly memory's `buffer`), so script and wasm see the same bytes.

use std::ops::{Deref, DerefMut};

use super::AlignedVec;

/// The bytes of an `ArrayBuffer`.
#[derive(Debug)]
pub(crate) enum Block {
    /// Bytes the buffer owns.
    Owned(AlignedVec<u8>),
    /// Bytes owned by the embedder (see [`crate::object::builtins::JsArrayBuffer::from_external`]).
    External { ptr: *mut u8, len: usize },
}

impl Block {
    /// The owned bytes, if they are owned.
    pub(crate) fn into_owned(self) -> Option<AlignedVec<u8>> {
        match self {
            Self::Owned(v) => Some(v),
            Self::External { .. } => None,
        }
    }
}

impl Clone for Block {
    /// A copy of external bytes is owned (it doesn't follow the embedder's memory).
    fn clone(&self) -> Self {
        match self {
            Self::Owned(v) => Self::Owned(v.clone()),
            Self::External { .. } => Self::Owned(AlignedVec::from_slice(64, self)),
        }
    }
}

impl Deref for Block {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(v) => v,
            // SAFETY: `from_external`'s caller promised `len` valid bytes at `ptr` until the
            // buffer is detached (and external buffers can only be detached, never resized or
            // transferred).
            Self::External { ptr, len } => unsafe { std::slice::from_raw_parts(*ptr, *len) },
        }
    }
}

impl DerefMut for Block {
    fn deref_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Owned(v) => v,
            // SAFETY: as in `deref`.
            Self::External { ptr, len } => unsafe { std::slice::from_raw_parts_mut(*ptr, *len) },
        }
    }
}
