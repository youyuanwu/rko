//! Safe array layout calculation.

use core::alloc::Layout;

use super::AllocError;

/// Computes `Layout` for `[T; n]`, returning `AllocError` on overflow.
pub(crate) fn array_layout<T>(n: usize) -> Result<Layout, AllocError> {
    let size = core::mem::size_of::<T>().checked_mul(n).ok_or(AllocError)?;
    // SAFETY: align_of::<T>() is always a valid alignment.
    Layout::from_size_align(size, core::mem::align_of::<T>()).map_err(|_| AllocError)
}
