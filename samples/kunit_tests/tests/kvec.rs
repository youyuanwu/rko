use rko_core::alloc::{Flags, KVec};
use rko_core::error::Error;

#[rko_core::rko_tests]
pub mod kvec_tests {
    use super::*;

    #[test]
    fn push_and_len() {
        let mut v = KVec::new();
        v.push(10i32, Flags::GFP_KERNEL).unwrap();
        v.push(20, Flags::GFP_KERNEL).unwrap();
        v.push(30, Flags::GFP_KERNEL).unwrap();
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn indexing() {
        let mut v = KVec::new();
        v.push(10i32, Flags::GFP_KERNEL).unwrap();
        v.push(20, Flags::GFP_KERNEL).unwrap();
        v.push(30, Flags::GFP_KERNEL).unwrap();
        assert_eq!(v[0], 10);
        assert_eq!(v[1], 20);
        assert_eq!(v[2], 30);
    }

    #[test]
    fn pop() {
        let mut v = KVec::new();
        v.push(10i32, Flags::GFP_KERNEL).unwrap();
        v.push(20, Flags::GFP_KERNEL).unwrap();
        v.push(30, Flags::GFP_KERNEL).unwrap();
        assert_eq!(v.pop(), Some(30));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn with_capacity() -> Result<(), Error> {
        let v = KVec::<u64>::with_capacity(16, Flags::GFP_KERNEL)?;
        assert!(v.capacity() >= 16);
        Ok(())
    }

    #[test]
    fn extend_from_slice() -> Result<(), Error> {
        let mut v = KVec::new();
        v.extend_from_slice(&[1u8, 2, 3, 4, 5], Flags::GFP_KERNEL)?;
        assert_eq!(v.len(), 5);
        assert_eq!(v[4], 5);
        Ok(())
    }

    #[test]
    fn clear() -> Result<(), Error> {
        let mut v = KVec::new();
        v.extend_from_slice(&[1u8, 2, 3], Flags::GFP_KERNEL)?;
        v.clear();
        assert!(v.is_empty());
        Ok(())
    }

    #[test]
    fn into_iter() -> Result<(), Error> {
        let mut v = KVec::new();
        v.push(100u32, Flags::GFP_KERNEL)?;
        v.push(200, Flags::GFP_KERNEL)?;
        let mut sum = 0u32;
        for x in v {
            sum += x;
        }
        assert_eq!(sum, 300);
        Ok(())
    }

    #[test]
    fn truncate() -> Result<(), Error> {
        let mut v = KVec::new();
        v.extend_from_slice(&[1u8, 2, 3, 4, 5], Flags::GFP_KERNEL)?;
        v.truncate(3);
        assert_eq!(v.len(), 3);
        assert_eq!(v[2], 3);
        Ok(())
    }

    #[test]
    fn truncate_beyond_len() -> Result<(), Error> {
        let mut v = KVec::new();
        v.push(1u32, Flags::GFP_KERNEL)?;
        v.truncate(100);
        assert_eq!(v.len(), 1);
        Ok(())
    }

    #[test]
    fn reserve() -> Result<(), Error> {
        let mut v = KVec::<u8>::new();
        v.reserve(64, Flags::GFP_KERNEL)?;
        assert!(v.capacity() >= 64);
        assert_eq!(v.len(), 0);
        Ok(())
    }

    #[test]
    fn empty_vec() {
        let v = KVec::<i32>::new();
        assert!(v.is_empty());
        assert_eq!(v.len(), 0);
        assert_eq!(v.capacity(), 0);
    }
}
