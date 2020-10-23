use crate::marker::{GcDeref, GcDrop, GcSafe};

/// A secret little function to check if a field is `GcDeref`
#[doc(hidden)]
#[inline(always)]
pub fn check_gc_deref<T: GcDeref>(_: &T) {}

/// A secret little function to check if a field is `GcDrop`
#[doc(hidden)]
#[inline(always)]
pub fn check_gc_drop<T: GcDrop>(_: &T) {}

/// A secret little function to check if a field is `GcSafe`
#[doc(hidden)]
#[inline(always)]
pub fn check_gc_safe<T: GcSafe>(_: &T) {}

// FIXME: This macro can be removed once we have overlapping marker traits
//        (https://github.com/rust-lang/rust/issues/29864)
/// A `Send + 'static` type can be safely marked as `GcSafe`, and this macro eases that
/// implementation
#[macro_export]
macro_rules! mark_send_static_gc_safe {
    ( $t:ty ) => {
        unsafe impl GcSafe for $t where $t: Send + 'static {}
    };
}

/// Implements an empty `Scan` implementation for a `Send + 'static` type
#[macro_export]
macro_rules! impl_empty_scan_for_send_static {
    ( $t:ty ) => {
        unsafe impl GcSafe for $t where $t: Send + 'static {}
        unsafe impl Scan for $t
        where
            $t: Send + 'static,
        {
            #[inline(always)]
            fn scan(&self, _: &mut crate::Scanner<'_>) {}
        }
    };
}
