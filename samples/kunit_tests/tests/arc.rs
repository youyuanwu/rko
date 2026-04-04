use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::sync::Arc;

#[rko_core::rko_tests]
pub mod arc_tests {
    use super::*;

    #[test]
    fn new_and_deref() -> Result<(), Error> {
        let a = Arc::new(42i32, Flags::GFP_KERNEL)?;
        assert_eq!(*a, 42);
        Ok(())
    }

    #[test]
    fn clone_and_deref() -> Result<(), Error> {
        let a = Arc::new(7u64, Flags::GFP_KERNEL)?;
        let b = a.clone();
        assert_eq!(*a, 7);
        assert_eq!(*b, 7);
        Ok(())
    }

    #[test]
    fn ptr_eq() -> Result<(), Error> {
        let a = Arc::new(1u8, Flags::GFP_KERNEL)?;
        let b = a.clone();
        assert!(Arc::ptr_eq(&a, &b));
        Ok(())
    }

    #[test]
    fn ptr_ne_different_arcs() -> Result<(), Error> {
        let a = Arc::new(1u8, Flags::GFP_KERNEL)?;
        let b = Arc::new(1u8, Flags::GFP_KERNEL)?;
        assert!(!Arc::ptr_eq(&a, &b));
        Ok(())
    }

    #[test]
    fn into_raw_from_raw() -> Result<(), Error> {
        let a = Arc::new(99i32, Flags::GFP_KERNEL)?;
        let ptr = Arc::into_raw(a);
        let b = unsafe { Arc::from_raw(ptr) };
        assert_eq!(*b, 99);
        Ok(())
    }
}
