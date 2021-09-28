use std::collections::hash_map::RandomState;
use std::ptr::drop_in_place;
use std::time::{Duration, Instant};

/// mark as value type
#[macro_export]
macro_rules! sync_value_type {
    ($t: ty) => {
        unsafe impl crate::marker::GcDeref for $t {}
        unsafe impl crate::marker::GcDrop for $t {}
        unsafe impl crate::marker::GcSafe for $t {}
        unsafe impl crate::Scan for $t {
            #[inline(always)]
            fn scan(&self, _: &mut crate::Scanner<'_>) {}
        }

        unsafe impl crate::Finalize for $t {
            unsafe fn finalize(&mut self) {
                drop_in_place(self);
            }
        }
    };
}

sync_value_type!(());
sync_value_type!(bool);

sync_value_type!(u8);
sync_value_type!(i8);
sync_value_type!(u16);
sync_value_type!(i16);
sync_value_type!(u32);
sync_value_type!(i32);
sync_value_type!(u64);
sync_value_type!(i64);
sync_value_type!(u128);
sync_value_type!(i128);
sync_value_type!(usize);
sync_value_type!(isize);
sync_value_type!(f32);
sync_value_type!(f64);

sync_value_type!(char);
sync_value_type!(String);
sync_value_type!(Instant);
sync_value_type!(Duration);

sync_value_type!(RandomState);

#[cfg(test)]
mod test {
    use std::mem::forget;
    use std::time::Instant;

    use crate::Finalize;

    macro_rules! test_no_panic_finalize {
        ($t:ident, $v:expr) => {
            paste::item! {
                #[test]
                #[allow(non_snake_case, clippy::forget_copy)]
                fn [<finalize_no_panic_ $t>]() {
                    let mut v: $t = $v;
                    unsafe {
                        v.finalize();
                    }
                    forget(v);
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

    test_no_panic_finalize!(f32, 1.0);
    test_no_panic_finalize!(f64, 1.0);

    test_no_panic_finalize!(String, String::from("hello"));
    test_no_panic_finalize!(Instant, Instant::now());
}
