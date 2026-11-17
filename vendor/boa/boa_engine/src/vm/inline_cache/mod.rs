use arrayvec::ArrayVec;
use itertools::Itertools;
use std::{cell::Cell, fmt};

use boa_gc::GcRefCell;
use boa_macros::{Finalize, Trace};

use crate::{
    JsString,
    object::shape::{Shape, WeakShape, slot::Slot},
};

#[cfg(test)]
mod tests;

pub(crate) const PIC_CAPACITY: usize = 4;

/// A cached shape-to-slot mapping for a polymorphic inline cache.
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct CacheEntry {
    /// A weak reference is kept to the shape to avoid the shape preventing deallocation.
    pub(crate) shape: WeakShape,
    /// Ironswift: the shape's address when cached, so a miss needs no pointer chasing.
    #[unsafe_ignore_trace]
    pub(crate) addr: usize,
    #[unsafe_ignore_trace]
    pub(crate) slot: Slot,
}

/// Ironswift: one link of a cached prototype chain: an object's shape when the lookup was cached.
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct ChainLink {
    pub(crate) shape: WeakShape,
    #[unsafe_ignore_trace]
    pub(crate) addr: usize,
}

impl ChainLink {
    pub(crate) fn new(shape: &Shape) -> Self {
        Self { shape: shape.into(), addr: shape.to_addr_usize() }
    }

    /// Whether `shape` is still the shape this link recorded.
    #[inline]
    pub(crate) fn matches(&self, shape: &Shape) -> bool {
        self.addr == shape.to_addr_usize() && self.shape.points_to(shape) == Some(true)
    }
}

/// Ironswift: a cached lookup that found the property two or more prototypes up the chain.
/// Boa's own entries only cover the object itself and its direct prototype. Valid while the
/// receiver and every prototype up to the holder have the same shapes and no unique shape has
/// changed in place (`shape_epoch`).
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct DeepEntry {
    pub(crate) receiver: ChainLink,
    /// Shapes of the prototypes, nearest first; the last one holds the property.
    pub(crate) chain: Vec<ChainLink>,
    #[unsafe_ignore_trace]
    pub(crate) slot: Slot,
    #[unsafe_ignore_trace]
    pub(crate) epoch: u64,
}

/// An inline cache entry for a property access.
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct InlineCache {
    /// The property that is accessed.
    pub(crate) name: JsString,

    /// Multiple cached shape-to-slot entries.
    pub(crate) entries: GcRefCell<ArrayVec<CacheEntry, PIC_CAPACITY>>,

    /// Whether this access site has seen too many shapes and should no longer be cached.
    #[unsafe_ignore_trace]
    pub(crate) megamorphic: Cell<bool>,

    /// Ironswift: a deep prototype lookup (see [`DeepEntry`]).
    pub(crate) deep: GcRefCell<Option<Box<DeepEntry>>>,
}

impl fmt::Display for InlineCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(name:{} entries:", self.name.display_escaped())?;

        if self.megamorphic.get() {
            return write!(f, "(megamorphic))");
        }

        let entries = self.entries.borrow();
        let entries = entries.iter().map(|e| e.shape.to_addr_usize()).format(", ");

        write!(f, "({entries:#x}))")
    }
}

impl InlineCache {
    pub(crate) fn new(name: JsString) -> Self {
        Self {
            name,
            entries: GcRefCell::new(ArrayVec::new()),
            megamorphic: Cell::new(false),
            deep: GcRefCell::new(None),
        }
    }

    pub(crate) fn set(&self, shape: &Shape, slot: Slot) {
        if self.megamorphic.get() {
            return;
        }

        let mut entries = self.entries.borrow_mut();
        // Ironswift: misses no longer remove collected shapes, so do it before giving up.
        if entries.is_full() {
            entries.retain(|e| e.shape.upgrade().is_some());
        }

        // Add a new entry if there's space.
        if entries
            .try_push(CacheEntry {
                shape: shape.into(),
                addr: shape.to_addr_usize(),
                slot,
            })
            .is_err()
        {
            // Polymorphic cache is full, transition to megamorphic.
            self.megamorphic.set(true);
            entries.clear();
        }
    }

    /// Looks up `shape` in the cache. Ironswift: compares the cached weak shapes with `shape`
    /// directly instead of upgrading each one (which cost two reference count changes per
    /// entry), and returns only the slot; the caller already holds the shape.
    pub(crate) fn get(&self, shape: &Shape) -> Option<Slot> {
        if self.megamorphic.get() {
            return None;
        }

        let addr = shape.to_addr_usize();
        let entries = self.entries.borrow();
        let found = entries.iter().position(|e| e.addr == addr)?;
        // Same address: check it is still the same shape and not a new one that reused the
        // memory of a collected one.
        if entries[found].shape.points_to(shape) == Some(true) {
            return Some(entries[found].slot);
        }
        drop(entries);
        self.entries.borrow_mut().swap_remove(found);
        None
    }
}
