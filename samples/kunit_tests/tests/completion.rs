use core::mem::MaybeUninit;
use rko_core::error::Error;
use rko_core::sync::Completion;

#[rko_core::rko_tests]
pub mod completion_tests {
    use super::*;

    #[test]
    fn already_complete() -> Result<(), Error> {
        let mut comp = MaybeUninit::<Completion>::uninit();
        unsafe { Completion::init(comp.as_mut_ptr()) };
        let comp = unsafe { comp.assume_init_mut() };
        comp.complete();
        let remaining = comp.wait_timeout(1000);
        assert!(remaining > 0);
        Ok(())
    }
}
