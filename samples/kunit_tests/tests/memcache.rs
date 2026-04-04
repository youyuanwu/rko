use rko_core::alloc::{Flags, MemCache};
use rko_core::error::Error;

#[rko_core::rko_tests]
pub mod memcache_tests {
    use super::*;

    #[test]
    fn create_and_drop() -> Result<(), Error> {
        let _cache = MemCache::try_new(c"rko_test_cache", 64, 8)?;
        // Drop cleans up via kmem_cache_destroy
        Ok(())
    }

    #[test]
    fn alloc_and_free() -> Result<(), Error> {
        let cache = MemCache::try_new(c"rko_test_alloc", 128, 8)?;
        let ptr = cache.alloc(Flags::GFP_KERNEL);
        assert!(!ptr.is_null());
        unsafe { cache.free(ptr) };
        Ok(())
    }

    #[test]
    fn multiple_allocs() -> Result<(), Error> {
        let cache = MemCache::try_new(c"rko_test_multi", 32, 8)?;
        let p1 = cache.alloc(Flags::GFP_KERNEL);
        let p2 = cache.alloc(Flags::GFP_KERNEL);
        let p3 = cache.alloc(Flags::GFP_KERNEL);
        assert!(!p1.is_null());
        assert!(!p2.is_null());
        assert!(!p3.is_null());
        // All pointers should be different
        assert_ne!(p1, p2);
        assert_ne!(p2, p3);
        unsafe {
            cache.free(p3);
            cache.free(p2);
            cache.free(p1);
        }
        Ok(())
    }
}
