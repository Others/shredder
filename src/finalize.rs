use crate::{Gc, Scan};

/// A trait implementing an alternative to `Drop`, useful for non-`'static` data.
///
/// Usually when you have data in a `Gc` you just want its destructor to be called when the data is
/// collected. However, the collector can't naively run the `drop` method of non-`'static` data,
/// since it could access data with an elapsed lifetime. (It's even UB to create a reference into
/// a struct holding an invalid reference!) We address this in two parts. The `R` and `RMut` structs
/// provide a safe alternative to holding a direct reference with a non-'static lifetime. Then the
/// `Finalize` trait let's you opt-in to writing unsafe code at deallocation time.
///
/// # Safety
/// When implementing this trait, you must guarantee that your data does not contain any
/// non-`'static` references. (You may use `R` and `RMut` instead!)
///
/// Furthermore, you must guarantee that `finalize` does not access any data with a non-`'static`
/// lifetime. In particular you may not call any methods on `R` or `RMut` other than `finalize`.
pub unsafe trait Finalize {
    /// Do cleanup on this data, potentially leaving it in an invalid state.
    /// (See trait documentation for the rules for implementing this method.)
    ///
    /// Please ensure your `finalize` implementations delegate properly and call your fields
    /// `finalize` methods after doing cleanup.
    ///
    /// # Safety
    /// After calling this method, no further operations may be performed with this object. You
    /// may not even drop this object! You must `mem::forget` it or otherwise force its destructor
    /// not to run.
    unsafe fn finalize(&mut self);
}

unsafe impl<T: Scan> Finalize for Gc<T> {
    unsafe fn finalize(&mut self) {
        self.internal_handle().invalidate();
    }
}

// FIXME: Github issue for missing `Finalize` implementations
macro_rules! impl_empty_finalize_for_static_type {
    ($t:ty) => {
        unsafe impl Finalize for $t
        where
            $t: 'static,
        {
            unsafe fn finalize(&mut self) {}
        }
    };
}

// Primitives need no finalization logic
impl_empty_finalize_for_static_type!(isize);
impl_empty_finalize_for_static_type!(usize);

impl_empty_finalize_for_static_type!(i8);
impl_empty_finalize_for_static_type!(u8);

impl_empty_finalize_for_static_type!(i16);
impl_empty_finalize_for_static_type!(u16);

impl_empty_finalize_for_static_type!(i32);
impl_empty_finalize_for_static_type!(u32);

impl_empty_finalize_for_static_type!(i64);
impl_empty_finalize_for_static_type!(u64);

impl_empty_finalize_for_static_type!(i128);
impl_empty_finalize_for_static_type!(u128);

#[cfg(test)]
mod test {
    use crate::Finalize;

    macro_rules! test_no_panic_finalize {
        ($t:ident, $v:expr) => {
            paste::item! {
                #[test]
                fn [<finalize_no_panic_ $t>]() {
                    let mut v: $t = $v;
                    unsafe {
                        v.finalize();
                    }
                }
            }
        };
    }

    test_no_panic_finalize!(isize, 1);
    test_no_panic_finalize!(usize, 1);

    test_no_panic_finalize!(i8, 1);
    test_no_panic_finalize!(u8, 1);

    test_no_panic_finalize!(i16, 1);
    test_no_panic_finalize!(u16, 1);

    test_no_panic_finalize!(i32, 1);
    test_no_panic_finalize!(u32, 1);

    test_no_panic_finalize!(i64, 1);
    test_no_panic_finalize!(u64, 1);

    test_no_panic_finalize!(i128, 1);
    test_no_panic_finalize!(u128, 1);
}
