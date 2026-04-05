use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::sync::Arc;
use rko_core::types::ForeignOwnable;

#[rko_core::rko_tests]
pub mod foreign_ownable_tests {
    use super::*;

    #[test]
    fn arc_round_trip() -> Result<(), Error> {
        let arc = Arc::new(42u32, Flags::GFP_KERNEL)?;
        let ptr = arc.into_foreign();
        assert!(!ptr.is_null());
        let recovered: Arc<u32> = unsafe { Arc::from_foreign(ptr) };
        assert_eq!(*recovered, 42);
        Ok(())
    }

    #[test]
    fn arc_borrow() -> Result<(), Error> {
        let arc = Arc::new(99u64, Flags::GFP_KERNEL)?;
        let ptr = arc.into_foreign();

        // Borrow without taking ownership.
        let borrowed = unsafe { <Arc<u64> as ForeignOwnable>::borrow(ptr) };
        assert_eq!(*borrowed, 99);

        // Still valid — borrow didn't consume.
        let recovered: Arc<u64> = unsafe { Arc::from_foreign(ptr) };
        assert_eq!(*recovered, 99);
        Ok(())
    }

    #[test]
    fn arc_borrow_multiple() -> Result<(), Error> {
        let arc = Arc::new(7u32, Flags::GFP_KERNEL)?;
        let ptr = arc.into_foreign();

        // Multiple borrows are fine.
        let b1 = unsafe { <Arc<u32> as ForeignOwnable>::borrow(ptr) };
        let b2 = unsafe { <Arc<u32> as ForeignOwnable>::borrow(ptr) };
        assert_eq!(*b1, 7);
        assert_eq!(*b2, 7);

        let _recovered: Arc<u32> = unsafe { Arc::from_foreign(ptr) };
        Ok(())
    }

    #[test]
    fn unit_round_trip() -> Result<(), Error> {
        let ptr = ().into_foreign();
        assert!(ptr.is_null());
        let _: () = unsafe { <() as ForeignOwnable>::from_foreign(ptr) };
        Ok(())
    }
}
