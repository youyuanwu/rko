use rko_core::sync::Refcount;

#[rko_core::rko_tests]
pub mod refcount_tests {
    use super::*;

    #[test]
    fn new_and_get() {
        let rc = Refcount::new(1);
        assert_eq!(rc.get(), 1);
    }

    #[test]
    fn set_and_get() {
        let rc = Refcount::new(1);
        rc.set(5);
        assert_eq!(rc.get(), 5);
    }

    #[test]
    fn inc() {
        let rc = Refcount::new(1);
        rc.inc();
        assert_eq!(rc.get(), 2);
        rc.inc();
        assert_eq!(rc.get(), 3);
    }

    #[test]
    fn dec_not_zero() {
        let rc = Refcount::new(3);
        assert!(!rc.dec_and_test());
        assert_eq!(rc.get(), 2);
        assert!(!rc.dec_and_test());
        assert_eq!(rc.get(), 1);
    }

    #[test]
    fn dec_to_zero() {
        let rc = Refcount::new(1);
        assert!(rc.dec_and_test());
        assert_eq!(rc.get(), 0);
    }

    #[test]
    fn inc_dec_roundtrip() {
        let rc = Refcount::new(1);
        rc.inc();
        rc.inc();
        assert_eq!(rc.get(), 3);
        assert!(!rc.dec_and_test());
        assert!(!rc.dec_and_test());
        assert!(rc.dec_and_test());
    }
}
