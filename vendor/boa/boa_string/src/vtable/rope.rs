//! Rope strings (Ironswift addition).
//!
//! `a + b` on long strings makes a rope: a small node that points at both halves instead of
//! copying them. The characters are copied into one buffer ("flattened") only the first time
//! someone reads them; the rope then keeps that buffer and lets go of the halves. This makes the
//! common `s += piece` loop linear instead of quadratic. Lengths are known without flattening.
//!
//! Ropes can be very deep (one level per `+=`), so flattening and dropping walk the tree with an
//! explicit stack instead of recursion.

use crate::iter::CodePointsIter;
use crate::vtable::JsStringVTable;
use crate::{JsStr, JsString, JsStringKind};
use std::cell::{Cell, UnsafeCell};
use std::process::abort;
use std::ptr::NonNull;

/// Concatenations shorter than this are copied right away; copying a short string is cheaper
/// than making (and later flattening) a rope node.
pub(crate) const ROPE_MIN_LEN: usize = 128;

/// A string made of two other strings.
#[repr(C)]
pub(crate) struct RopeString {
    /// Embedded `VTable` - must be the first field for vtable dispatch.
    vtable: JsStringVTable,
    refcount: Cell<usize>,
    /// The two halves, until the rope is flattened.
    parts: UnsafeCell<Option<(JsString, JsString)>>,
    /// The flattened characters, once read. Never replaced or cleared while the rope lives, so
    /// `JsStr`s handed out from it stay valid.
    flat: UnsafeCell<Option<JsString>>,
}

impl RopeString {
    pub(crate) fn new(left: JsString, right: JsString) -> Self {
        let len = left.len() + right.len();
        RopeString {
            vtable: JsStringVTable {
                clone: rope_clone,
                drop: rope_drop,
                as_str: rope_as_str,
                code_points: rope_code_points,
                refcount: rope_refcount,
                len,
                kind: JsStringKind::Rope,
            },
            refcount: Cell::new(1),
            parts: UnsafeCell::new(Some((left, right))),
            flat: UnsafeCell::new(None),
        }
    }

    /// The flattened string, flattening now if needed.
    fn flat(&self) -> &JsString {
        // SAFETY: `flat` is only written here, once, and never while a reference to it exists
        // (it is `None` until then). Strings are single-threaded.
        if let Some(flat) = unsafe { &*self.flat.get() } {
            return flat;
        }
        let flat = {
            // SAFETY: `parts` is only taken below, after this borrow ends.
            let parts = unsafe { &*self.parts.get() };
            let (left, right) = parts.as_ref().expect("an unflattened rope has its parts");
            let mut leaves: Vec<JsStr<'_>> = Vec::new();
            let mut stack: Vec<&JsString> = vec![right, left];
            while let Some(s) = stack.pop() {
                if s.kind() == JsStringKind::Rope {
                    // SAFETY: kind checked.
                    let rope: &RopeString = unsafe { s.as_inner() };
                    // SAFETY: as above; nested ropes are not being flattened right now.
                    if let Some(flat) = unsafe { &*rope.flat.get() } {
                        leaves.push(flat.as_str());
                    } else if let Some((l, r)) = unsafe { &*rope.parts.get() } {
                        stack.push(r);
                        stack.push(l);
                    }
                } else {
                    leaves.push(s.as_str());
                }
            }
            JsString::concat_array(&leaves)
        };
        // SAFETY: no references into `flat` or `parts` are alive here.
        unsafe {
            *self.flat.get() = Some(flat);
            let parts = (*self.parts.get()).take();
            if let Some((l, r)) = parts {
                drop_deep(vec![l, r]);
            }
            (*self.flat.get()).as_ref().expect("just set")
        }
    }
}

/// Drops strings without recursing through ropes we are the last owner of.
fn drop_deep(mut stack: Vec<JsString>) {
    while let Some(s) = stack.pop() {
        if s.kind() == JsStringKind::Rope && s.refcount() == Some(1) {
            // SAFETY: kind checked; we hold the only reference, so nobody else can see it.
            let rope: &RopeString = unsafe { s.as_inner() };
            if let Some((l, r)) = unsafe { (*rope.parts.get()).take() } {
                stack.push(l);
                stack.push(r);
            }
        }
        drop(s);
    }
}

#[inline]
fn rope_clone(vtable: NonNull<JsStringVTable>) -> JsString {
    // SAFETY: This is part of the correct vtable which is validated on construction.
    let this: &RopeString = unsafe { vtable.cast().as_ref() };
    let Some(strong) = this.refcount.get().checked_add(1) else {
        abort();
    };
    this.refcount.set(strong);
    // SAFETY: validated the string outside this function.
    unsafe { JsString::from_ptr(vtable) }
}

#[inline]
fn rope_drop(vtable: NonNull<JsStringVTable>) {
    // SAFETY: This is part of the correct vtable which is validated on construction.
    let this: &RopeString = unsafe { vtable.cast().as_ref() };
    let Some(new) = this.refcount.get().checked_sub(1) else {
        abort();
    };
    this.refcount.set(new);
    if new != 0 {
        return;
    }
    // SAFETY: last reference; take the children out so the box drop below is shallow.
    let parts = unsafe { (*this.parts.get()).take() };
    // SAFETY: This is the last reference, so we can deallocate.
    unsafe {
        drop(Box::from_raw(vtable.cast::<RopeString>().as_ptr()));
    }
    if let Some((l, r)) = parts {
        drop_deep(vec![l, r]);
    }
}

#[inline]
fn rope_as_str(vtable: NonNull<JsStringVTable>) -> JsStr<'static> {
    // SAFETY: This is part of the correct vtable which is validated on construction.
    let this: &RopeString = unsafe { vtable.cast().as_ref() };
    // SAFETY: the flat string lives as long as the rope (see `flat`).
    unsafe { this.flat().as_str().as_static() }
}

#[inline]
fn rope_code_points(vtable: NonNull<JsStringVTable>) -> CodePointsIter<'static> {
    CodePointsIter::new(rope_as_str(vtable))
}

#[inline]
#[allow(clippy::unnecessary_wraps)]
fn rope_refcount(vtable: NonNull<JsStringVTable>) -> Option<usize> {
    // SAFETY: This is part of the correct vtable which is validated on construction.
    let this: &RopeString = unsafe { vtable.cast().as_ref() };
    Some(this.refcount.get())
}
