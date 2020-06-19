use crate::{GcSafe, Scan, Scanner};

/// Helper trait for implementing an empty scan implementation
///
/// # Safety
/// This is safe since an empty `Scan` implementation is always safe. And we can always implement
/// `GcSafe` for a `Send` type.
pub trait EmptyScan: Send {}

unsafe impl<T: EmptyScan> GcSafe for T {}

unsafe impl<T: EmptyScan> Scan for T {
    #[inline(always)]
    fn scan(&self, _: &mut Scanner<'_>) {}
}
