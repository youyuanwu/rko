use rko_core::time::Ktime;

#[rko_core::rko_tests]
pub mod ktime_tests {
    use super::*;

    #[test]
    fn zero() {
        assert_eq!(Ktime::ZERO.to_ns(), 0);
        assert_eq!(Ktime::ZERO.to_ms(), 0);
    }

    #[test]
    fn from_ns_to_ns() {
        let t = Ktime::from_ns(12345);
        assert_eq!(t.to_ns(), 12345);
    }

    #[test]
    fn from_ms_to_ms() {
        let t = Ktime::from_ms(42);
        assert_eq!(t.to_ms(), 42);
    }

    #[test]
    fn from_secs_to_ns() {
        let t = Ktime::from_secs(3);
        assert_eq!(t.to_ns(), 3_000_000_000);
    }

    #[test]
    fn ms_to_ns_conversion() {
        let t = Ktime::from_ms(1);
        assert_eq!(t.to_ns(), 1_000_000);
    }

    #[test]
    fn negative_values() {
        let t = Ktime::from_ns(-100);
        assert_eq!(t.to_ns(), -100);
    }
}
