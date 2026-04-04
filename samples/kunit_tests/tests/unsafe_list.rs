use rko_core::unsafe_list::{Adapter, Links, List};

struct Entry {
    value: i32,
    links: Links<Entry>,
}

impl Entry {
    fn new(value: i32) -> Self {
        Self {
            value,
            links: Links::new(),
        }
    }
}

struct EntryAdapter;

// SAFETY: Entry::links is at a fixed offset within Entry.
unsafe impl Adapter for EntryAdapter {
    type EntryType = Entry;

    unsafe fn entry_to_links(entry: *const Entry) -> *const Links<Entry> {
        // SAFETY: Caller guarantees entry is valid.
        unsafe { &(*entry).links as *const _ }
    }

    unsafe fn links_to_entry(links: *const Links<Entry>) -> *const Entry {
        // SAFETY: links is embedded at a known offset in Entry.
        let offset = core::mem::offset_of!(Entry, links);
        unsafe { (links as *const u8).sub(offset) as *const Entry }
    }
}

#[rko_core::rko_tests]
pub mod list_tests {
    use super::*;

    #[test]
    fn links_default_unlinked() {
        let links = Links::<Entry>::new();
        assert!(!links.is_linked());
    }

    #[test]
    fn push_pop_single() {
        // Use List in-place via a local — List::new() sets sentinel
        // pointers to &self.head, so the list must not move after creation.
        let list = &List::<EntryAdapter>::new();
        let e = Entry::new(42);
        unsafe { list.push_back(&e) };
        assert!(!list.is_empty());
        assert!(e.links.is_linked());
        let front = list.pop_front().unwrap();
        assert_eq!(unsafe { (*front).value }, 42);
        assert!(!e.links.is_linked());
        assert!(list.is_empty());
    }

    #[test]
    fn push_pop_fifo_order() {
        let list = &List::<EntryAdapter>::new();
        let e1 = Entry::new(1);
        let e2 = Entry::new(2);
        let e3 = Entry::new(3);
        unsafe {
            list.push_back(&e1);
            list.push_back(&e2);
            list.push_back(&e3);
        }
        assert_eq!(unsafe { (*list.pop_front().unwrap()).value }, 1);
        assert_eq!(unsafe { (*list.pop_front().unwrap()).value }, 2);
        assert_eq!(unsafe { (*list.pop_front().unwrap()).value }, 3);
        assert!(list.is_empty());
    }

    #[test]
    fn remove_middle() {
        let list = &List::<EntryAdapter>::new();
        let e1 = Entry::new(1);
        let e2 = Entry::new(2);
        let e3 = Entry::new(3);
        unsafe {
            list.push_back(&e1);
            list.push_back(&e2);
            list.push_back(&e3);
            list.remove(&e2);
        }
        assert!(!e2.links.is_linked());
        assert_eq!(unsafe { (*list.pop_front().unwrap()).value }, 1);
        assert_eq!(unsafe { (*list.pop_front().unwrap()).value }, 3);
        assert!(list.is_empty());
    }

    #[test]
    fn front_peek() {
        let list = &List::<EntryAdapter>::new();
        let e = Entry::new(99);
        unsafe { list.push_back(&e) };
        let front = list.front().unwrap();
        assert_eq!(unsafe { (*front).value }, 99);
        // front() doesn't remove — list still non-empty
        assert!(!list.is_empty());
        list.pop_front();
    }
}
