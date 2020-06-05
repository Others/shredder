use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::{GcSafe, Scan, Scanner};
use std::cmp::Ordering;

// Only straight up `'static` references can be `Scan` or `GcSafe`, since other references may
// become invalid after their lifetime ends
unsafe impl<T> Scan for &'static T {
    #[inline(always)]
    fn scan(&self, _: &mut Scanner<'_>) {}
}
unsafe impl<T> GcSafe for &'static T {}

unsafe impl<T> Scan for &'static mut T {
    #[inline(always)]
    fn scan(&self, _: &mut Scanner<'_>) {}
}
unsafe impl<T> GcSafe for &'static mut T {}

// But other references can become safe through careful manipulation!

/// A `GcSafe` version of `&T`
///
/// This lets you store non-`'static` references inside a `Gc`!
#[derive(Debug)]
pub struct R<'a, T> {
    raw_ptr: *const T,
    _marker: PhantomData<&'a T>,
}

/// A `GcSafe` version of `&mut T`
///
/// This lets you store non-`'static` mutable references inside a `Gc`!
#[derive(Debug)]
pub struct RMut<'a, T> {
    raw_ptr: *mut T,
    _marker: PhantomData<&'a mut T>,
}

// Impl `GcSafe` and `Scan`!
unsafe impl<'a, T> GcSafe for R<'a, T> {}
unsafe impl<'a, T> GcSafe for RMut<'a, T> {}

unsafe impl<'a, T> Scan for R<'a, T> {
    #[inline(always)]
    fn scan(&self, _: &mut Scanner<'_>) {}
}
unsafe impl<'a, T> Scan for RMut<'a, T> {
    #[inline(always)]
    fn scan(&self, _: &mut Scanner<'_>) {}
}

// Fixup the concurrency marker traits
unsafe impl<'a, T> Send for R<'a, T> where &'a T: Send {}
unsafe impl<'a, T> Sync for R<'a, T> where &'a T: Sync {}

unsafe impl<'a, T> Send for RMut<'a, T> where &'a mut T: Send {}
unsafe impl<'a, T> Sync for RMut<'a, T> where &'a mut T: Sync {}

// The critical impls! The derefs!
impl<'a, T> Deref for R<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.raw_ptr }
    }
}

impl<'a, T> Deref for RMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.raw_ptr }
    }
}

impl<'a, T> DerefMut for RMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.raw_ptr }
    }
}

// Clone + Copy for `R`
impl<'a, T> Clone for R<'a, T> {
    fn clone(&self) -> Self {
        Self {
            raw_ptr: self.raw_ptr,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> Copy for R<'a, T> {}

// Lots of nice helpful traits for wrapper types to implement :)

impl<'a, T> Hash for R<'a, T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        let raw: &T = self.deref();
        raw.hash(state);
    }
}

impl<'a, T> Hash for RMut<'a, T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        let raw: &T = self.deref();
        raw.hash(state);
    }
}

// TODO: Ord, PartialOrd

impl<'a, T> PartialEq for R<'a, T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(self.deref() as &T, other.deref() as &T)
    }

    #[allow(clippy::partialeq_ne_impl)]
    fn ne(&self, other: &Self) -> bool {
        PartialEq::ne(self.deref() as &T, other.deref() as &T)
    }
}

impl<'a, T> Eq for R<'a, T> where T: Eq {}

impl<'a, T> PartialEq for RMut<'a, T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(self.deref() as &T, other.deref() as &T)
    }

    #[allow(clippy::partialeq_ne_impl)]
    fn ne(&self, other: &Self) -> bool {
        PartialEq::ne(self.deref() as &T, other.deref() as &T)
    }
}

impl<'a, T> Eq for RMut<'a, T> where T: Eq {}

impl<'a, T> PartialOrd for R<'a, T>
where
    T: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        PartialOrd::partial_cmp(self.deref() as &T, other.deref() as &T)
    }

    fn lt(&self, other: &Self) -> bool {
        PartialOrd::lt(self.deref() as &T, other.deref() as &T)
    }

    fn le(&self, other: &Self) -> bool {
        PartialOrd::le(self.deref() as &T, other.deref() as &T)
    }

    fn gt(&self, other: &Self) -> bool {
        PartialOrd::gt(self.deref() as &T, other.deref() as &T)
    }

    fn ge(&self, other: &Self) -> bool {
        PartialOrd::ge(self.deref() as &T, other.deref() as &T)
    }
}

impl<'a, T> Ord for R<'a, T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(self.deref() as &T, other.deref() as &T)
    }
}

impl<'a, T> PartialOrd for RMut<'a, T>
where
    T: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        PartialOrd::partial_cmp(self.deref() as &T, other.deref() as &T)
    }

    fn lt(&self, other: &Self) -> bool {
        PartialOrd::lt(self.deref() as &T, other.deref() as &T)
    }

    fn le(&self, other: &Self) -> bool {
        PartialOrd::le(self.deref() as &T, other.deref() as &T)
    }

    fn gt(&self, other: &Self) -> bool {
        PartialOrd::gt(self.deref() as &T, other.deref() as &T)
    }

    fn ge(&self, other: &Self) -> bool {
        PartialOrd::ge(self.deref() as &T, other.deref() as &T)
    }
}

impl<'a, T> Ord for RMut<'a, T>
where
    T: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(self.deref() as &T, other.deref() as &T)
    }
}
