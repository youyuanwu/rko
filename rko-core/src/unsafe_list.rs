//! Intrusive doubly-linked circular list.
//!
//! Provides [`List<T>`] — an intrusive list where [`Links`] nodes are
//! embedded directly in the objects being tracked. The [`Adapter`] trait
//! maps between entry pointers and link pointers.
//!
//! # Safety
//!
//! This module is fundamentally `unsafe`. Callers must ensure that:
//! - Entries are not moved while linked.
//! - Entries are not dropped while linked.
//! - The adapter conversions are correct.

use core::cell::Cell;
use core::ptr;

/// Intrusive list links.
///
/// Embed this in your struct to make it a list node. Initialize with
/// [`Links::new`] before first use.
pub struct Links<T: ?Sized> {
    next: Cell<*const Links<T>>,
    prev: Cell<*const Links<T>>,
}

impl<T: ?Sized> Links<T> {
    /// Create new unlinked list links.
    pub fn new() -> Self {
        Self {
            next: Cell::new(ptr::null()),
            prev: Cell::new(ptr::null()),
        }
    }

    /// Returns `true` if this node is currently in a list.
    pub fn is_linked(&self) -> bool {
        !self.next.get().is_null()
    }
}

impl<T: ?Sized> Default for Links<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for converting between entry pointers and [`Links`] pointers.
///
/// # Safety
///
/// Implementations must return correct, consistent pointers. The links
/// returned by `entry_to_links` must be embedded within the entry, and
/// `links_to_entry` must be its exact inverse.
pub unsafe trait Adapter {
    /// The type of the entries in the list.
    type EntryType: ?Sized;

    /// Get a pointer to the [`Links`] embedded in `entry`.
    ///
    /// # Safety
    ///
    /// `entry` must point to a valid, aligned instance of `EntryType`.
    unsafe fn entry_to_links(entry: *const Self::EntryType) -> *const Links<Self::EntryType>;

    /// Get a pointer to the entry containing `links`.
    ///
    /// # Safety
    ///
    /// `links` must have been returned by [`entry_to_links`](Adapter::entry_to_links).
    unsafe fn links_to_entry(links: *const Links<Self::EntryType>) -> *const Self::EntryType;
}

/// An intrusive doubly-linked circular list.
///
/// The `A` type parameter selects the [`Adapter`] that maps entries to
/// their embedded [`Links`].
pub struct List<A: Adapter + ?Sized> {
    head: Links<A::EntryType>,
}

impl<A: Adapter + ?Sized> List<A> {
    /// Create a new empty list.
    pub fn new() -> Self
    where
        A: Sized,
    {
        let list = Self { head: Links::new() };
        // Point the sentinel at itself (empty-list invariant).
        let head_ptr: *const Links<A::EntryType> = &list.head;
        list.head.next.set(head_ptr);
        list.head.prev.set(head_ptr);
        list
    }

    /// Returns `true` if the list contains no entries.
    pub fn is_empty(&self) -> bool {
        let head_ptr: *const Links<A::EntryType> = &self.head;
        self.head.next.get() == head_ptr
    }

    /// Push an entry at the back of the list.
    ///
    /// # Safety
    ///
    /// - `entry` must point to a valid, aligned entry with initialized
    ///   [`Links`] that is **not** currently in any list.
    /// - The entry must remain valid and unmoved for as long as it is in
    ///   the list.
    pub unsafe fn push_back(&self, entry: *const A::EntryType) {
        // SAFETY: Caller guarantees entry is valid.
        let links = unsafe { A::entry_to_links(entry) };
        let head_ptr: *const Links<A::EntryType> = &self.head;
        let tail = self.head.prev.get();

        // Insert between tail and head (sentinel).
        // SAFETY: tail is valid (either &self.head or a valid node).
        unsafe {
            (*links).prev.set(tail);
            (*links).next.set(head_ptr);
            (*tail).next.set(links);
        }
        self.head.prev.set(links);
    }

    /// Pop the front entry from the list.
    ///
    /// Returns `None` if the list is empty.
    pub fn pop_front(&self) -> Option<*const A::EntryType> {
        if self.is_empty() {
            return None;
        }

        let head_ptr: *const Links<A::EntryType> = &self.head;
        let first = self.head.next.get();

        // SAFETY: The list is non-empty, so first is a valid node (not the sentinel).
        unsafe {
            let second = (*first).next.get();
            self.head.next.set(second);
            (*second).prev.set(head_ptr);

            // Clear the removed node's links.
            (*first).next.set(ptr::null());
            (*first).prev.set(ptr::null());

            Some(A::links_to_entry(first))
        }
    }

    /// Return a pointer to the front entry without removing it.
    ///
    /// Returns `None` if the list is empty.
    pub fn front(&self) -> Option<*const A::EntryType> {
        if self.is_empty() {
            return None;
        }

        let first = self.head.next.get();
        // SAFETY: The list is non-empty, so first is a valid node.
        Some(unsafe { A::links_to_entry(first) })
    }

    /// Remove a specific entry from the list.
    ///
    /// # Safety
    ///
    /// - `entry` must point to a valid entry that is currently in **this** list.
    pub unsafe fn remove(&self, entry: *const A::EntryType) {
        // SAFETY: Caller guarantees entry is in this list.
        let links = unsafe { A::entry_to_links(entry) };

        // SAFETY: The entry is in the list, so prev/next are valid.
        unsafe {
            let prev = (*links).prev.get();
            let next = (*links).next.get();
            (*prev).next.set(next);
            (*next).prev.set(prev);

            // Clear the removed node's links.
            (*links).next.set(ptr::null());
            (*links).prev.set(ptr::null());
        }
    }
}

impl<A: Adapter> Default for List<A> {
    fn default() -> Self {
        Self::new()
    }
}
