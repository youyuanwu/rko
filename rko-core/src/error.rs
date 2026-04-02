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
    pub const ECONNRESET: Self = Error::new(rko_sys::rko::err::ECONNRESET);
    pub const ECONNABORTED: Self = Error::new(rko_sys::rko::err::ECONNABORTED);
    pub const ENXIO: Self = Error::new(rko_sys::rko::err::ENXIO);
    pub const ENOSYS: Self = Error::new(rko_sys::rko::err::ENOSYS);
    pub const EOPNOTSUPP: Self = Error::new(rko_sys::rko::err::EOPNOTSUPP);
    pub const ESTALE: Self = Error::new(rko_sys::rko::err::ESTALE);
    pub const ENODATA: Self = Error::new(rko_sys::rko::err::ENODATA);
    pub const ENOTDIR: Self = Error::new(rko_sys::rko::err::ENOTDIR);
    pub const ERANGE: Self = Error::new(rko_sys::rko::err::ERANGE);
    pub const E2BIG: Self = Error::new(rko_sys::rko::err::E2BIG);
    pub const ENAMETOOLONG: Self = Error::new(rko_sys::rko::err::ENAMETOOLONG);
    pub const EDOM: Self = Error::new(rko_sys::rko::err::EDOM);
    pub const EISDIR: Self = Error::new(rko_sys::rko::err::EISDIR);

    /// Create an `Error` from a raw negative errno returned by a kernel function.
    ///
    /// If `errno` is zero or positive, returns `EINVAL` as a fallback.
    pub const fn from_errno(errno: core::ffi::c_int) -> Self {
        if errno < 0 {
            Error(errno)
        } else {
            Self::EINVAL
        }
    }
}

impl core::fmt::Debug for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Error({})", self.0)
    }
}

/// Converts a `Result<()>` to a C `int` (0 on success, negative errno on failure).
///
/// Use in C callback trampolines:
/// ```ignore
/// from_result(|| {
///     T::some_callback(args)?;
///     Ok(())
/// })
/// ```
#[inline]
pub fn from_result(f: impl FnOnce() -> Result<(), Error>) -> core::ffi::c_int {
    match f() {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}

/// Converts a possibly-error kernel pointer to a `Result`.
///
/// If `ptr` is an `ERR_PTR`, extracts the errno. Otherwise returns
/// the pointer as `Ok`.
///
/// # Safety
///
/// `ptr` must be a valid kernel pointer or an `ERR_PTR` value.
#[inline]
pub unsafe fn from_err_ptr<T>(ptr: *mut T) -> Result<*mut T, Error> {
    use rko_sys::rko::helpers as h;
    if unsafe { h::rust_helper_IS_ERR(ptr.cast()) } {
        Err(Error::from_errno(unsafe {
            h::rust_helper_PTR_ERR(ptr.cast()) as i32
        }))
    } else {
        Ok(ptr)
    }
}

/// Converts a `Result<*mut T>` to an `ERR_PTR` or valid pointer for
/// returning to C.
#[inline]
pub fn to_err_ptr<T>(result: Result<*mut T, Error>) -> *mut T {
    match result {
        Ok(ptr) => ptr,
        Err(e) => unsafe { rko_sys::rko::helpers::rust_helper_ERR_PTR(e.to_errno() as i64).cast() },
    }
}

impl From<crate::alloc::AllocError> for Error {
    fn from(_: crate::alloc::AllocError) -> Self {
        Error::ENOMEM
    }
}
