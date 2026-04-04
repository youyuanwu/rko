use rko_core::error::{self, Error};

#[rko_core::rko_tests]
pub mod error_tests {
    use super::*;

    #[test]
    fn new_to_errno_roundtrip() {
        let e = Error::EINVAL;
        let code = e.to_errno();
        assert!(code < 0);
    }

    #[test]
    fn from_errno_negative() {
        let e = Error::from_errno(-22);
        assert_eq!(e.to_errno(), -22);
    }

    #[test]
    fn from_errno_zero_falls_back() {
        let e = Error::from_errno(0);
        assert_eq!(e.to_errno(), Error::EINVAL.to_errno());
    }

    #[test]
    fn from_errno_positive_falls_back() {
        let e = Error::from_errno(1);
        assert_eq!(e.to_errno(), Error::EINVAL.to_errno());
    }

    #[test]
    fn from_result_ok() {
        let ret = error::from_result(|| Ok(()));
        assert_eq!(ret, 0);
    }

    #[test]
    fn from_result_err() {
        let ret = error::from_result(|| Err(Error::ENOMEM));
        assert!(ret < 0);
    }

    #[test]
    fn error_constants_are_negative() {
        assert!(Error::EINVAL.to_errno() < 0);
        assert!(Error::ENOMEM.to_errno() < 0);
        assert!(Error::ENOENT.to_errno() < 0);
        assert!(Error::EIO.to_errno() < 0);
        assert!(Error::EPERM.to_errno() < 0);
    }
}
