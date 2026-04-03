// SPDX-License-Identifier: GPL-2.0

//! Kernel time types.
//!
//! Provides `Time` for inode timestamps and `Ktime` for monotonic kernel time.

/// Timestamp with second + nanosecond resolution.
///
/// Used for inode timestamps (atime, mtime, ctime).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Time {
    /// Seconds since the Unix epoch.
    pub secs: u64,
    /// Nanoseconds within the second (0..999_999_999).
    pub nsecs: u32,
}

impl Time {
    /// The Unix epoch (1970-01-01 00:00:00 UTC).
    pub const ZERO: Self = Self { secs: 0, nsecs: 0 };

    /// Create a timestamp from seconds only.
    pub const fn from_secs(secs: u64) -> Self {
        Self { secs, nsecs: 0 }
    }
}

/// Monotonic kernel time in nanoseconds (wraps `ktime_t`).
///
/// This is a duration-like value for measuring intervals, not a
/// wall-clock timestamp. Corresponds to `CLOCK_MONOTONIC`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ktime(i64);

impl Ktime {
    /// Zero duration.
    pub const ZERO: Self = Self(0);

    /// Create from raw nanoseconds.
    pub const fn from_ns(ns: i64) -> Self {
        Self(ns)
    }

    /// Create from milliseconds.
    pub const fn from_ms(ms: i64) -> Self {
        Self(ms * 1_000_000)
    }

    /// Create from seconds.
    pub const fn from_secs(secs: i64) -> Self {
        Self(secs * 1_000_000_000)
    }

    /// Returns the raw nanosecond value.
    pub const fn to_ns(self) -> i64 {
        self.0
    }

    /// Returns the value in milliseconds (truncated).
    pub const fn to_ms(self) -> i64 {
        self.0 / 1_000_000
    }
}
