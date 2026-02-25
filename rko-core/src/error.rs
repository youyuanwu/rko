//! Kernel error type wrapping errno codes.

/// A kernel error code (negative errno value).
pub struct Error(core::ffi::c_int);

impl Error {
    /// Create an `Error` from a kernel errno constant.
    ///
    /// The value is stored as its negative: `Error::new(EINVAL)` → `-EINVAL`.
    pub const fn new(errno: core::ffi::c_int) -> Self {
        Error(-errno)
    }

    /// Return the negative errno value for passing back to the kernel.
    pub const fn to_errno(self) -> core::ffi::c_int {
        self.0
    }

    pub const EINVAL: Self = Error::new(rko_sys::rko::err::EINVAL);
    pub const ENOMEM: Self = Error::new(rko_sys::rko::err::ENOMEM);
    pub const ENOENT: Self = Error::new(rko_sys::rko::err::ENOENT);
    pub const EBUSY: Self = Error::new(rko_sys::rko::err::EBUSY);
    pub const EEXIST: Self = Error::new(rko_sys::rko::err::EEXIST);
    pub const EIO: Self = Error::new(rko_sys::rko::err::EIO);
    pub const EPERM: Self = Error::new(rko_sys::rko::err::EPERM);
}

impl core::fmt::Debug for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Error({})", self.0)
    }
}

impl From<crate::alloc::AllocError> for Error {
    fn from(_: crate::alloc::AllocError) -> Self {
        Error::ENOMEM
    }
}
