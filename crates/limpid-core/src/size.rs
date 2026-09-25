//! Byte accounting.
//!
//! Every size in Limpid is a pair. `apparent` is what `ls` reports;
//! `on_disk` is `st_blocks * 512`, which is what actually comes back when the
//! file goes away. On a compressed btrfs — the default on Omarchy, which
//! mounts `/` with `compress=zstd:3` — the two differ by a lot, and it is the
//! second one that moves `df`.

use std::fmt;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;

/// Size of something, measured both ways.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Size {
    /// Sum of file lengths, as `ls -l` would report them.
    pub apparent: u64,
    /// Sum of allocated blocks. This is what deleting actually reclaims.
    pub on_disk: u64,
}

impl Size {
    /// A size of nothing.
    pub const ZERO: Self = Self {
        apparent: 0,
        on_disk: 0,
    };

    /// Build a size from both measurements.
    pub const fn new(apparent: u64, on_disk: u64) -> Self {
        Self { apparent, on_disk }
    }

    /// Read both measurements off a file's metadata.
    ///
    /// `st_blocks` is defined in 512-byte units regardless of the
    /// filesystem's own block size, so the multiplier is not a guess.
    pub fn of(metadata: &Metadata) -> Self {
        Self {
            apparent: metadata.len(),
            on_disk: metadata.blocks() * 512,
        }
    }

    /// Whether this size is zero on both counts.
    pub fn is_zero(self) -> bool {
        self.apparent == 0 && self.on_disk == 0
    }
}

impl std::ops::Add for Size {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            apparent: self.apparent.saturating_add(other.apparent),
            on_disk: self.on_disk.saturating_add(other.on_disk),
        }
    }
}

impl std::ops::AddAssign for Size {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl std::iter::Sum for Size {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |total, size| total + size)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", human(self.on_disk))
    }
}

/// Format a byte count the way a file manager would: base 1024, at most one
/// decimal, and no decimal at all for plain bytes.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    // Three significant figures reads better than a fixed decimal count:
    // "1.02 GiB" and "999 MiB" rather than "1.0 GiB" and "999.0 MiB".
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_below_a_kibibyte_are_not_scaled() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
    }

    #[test]
    fn larger_values_carry_three_significant_figures() {
        assert_eq!(human(1024), "1.00 KiB");
        assert_eq!(human(10 * 1024), "10.0 KiB");
        assert_eq!(human(100 * 1024), "100 KiB");
        assert_eq!(human(1024 * 1024 * 1024), "1.00 GiB");
    }

    #[test]
    fn sizes_add_on_both_axes() {
        let total = Size::new(100, 4096) + Size::new(50, 4096);
        assert_eq!(total, Size::new(150, 8192));
    }

    #[test]
    fn addition_saturates_rather_than_overflowing() {
        let total = Size::new(u64::MAX, u64::MAX) + Size::new(1, 1);
        assert_eq!(total, Size::new(u64::MAX, u64::MAX));
    }
}
