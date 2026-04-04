use rko_core::revocable::AsyncRevocable;

#[rko_core::rko_tests]
pub mod revocable_tests {
    use super::*;

    #[test]
    fn access_before_revoke() {
        let r = AsyncRevocable::new(42i32);
        let guard = r.try_access();
        assert!(guard.is_some());
        assert_eq!(*guard.unwrap(), 42);
    }

    #[test]
    fn access_after_revoke() {
        let r = AsyncRevocable::new(42i32);
        let revoked = r.revoke();
        assert!(revoked);
        assert!(r.try_access().is_none());
    }

    #[test]
    fn is_revoked() {
        let r = AsyncRevocable::new(0u8);
        assert!(!r.is_revoked());
        r.revoke();
        assert!(r.is_revoked());
    }

    #[test]
    fn double_revoke() {
        let r = AsyncRevocable::new(0u8);
        assert!(r.revoke());
        assert!(!r.revoke());
    }
}
