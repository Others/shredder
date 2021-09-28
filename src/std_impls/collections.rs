use crate::marker::{GcDeref, GcDrop, GcSafe};
use crate::{Finalize, Scan, Scanner};
// all 7 types in `std::collections` has been implemented
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, LinkedList, VecDeque};
use std::hash::BuildHasher;
use std::mem::forget;
use std::ptr::read;

// For pretty much all simple collections, the collection inherets the properites of what it contains
// (with respect to GcDeref, GcDrop and GcSafe)

// HASHMAP
unsafe impl<K, V, S: BuildHasher> GcDeref for HashMap<K, V, S>
where
    K: GcDeref,
    V: GcDeref,
    S: GcDeref,
{
}

unsafe impl<K, V, S: BuildHasher> GcDrop for HashMap<K, V, S>
where
    K: GcDrop,
    V: GcDrop,
    S: GcDrop,
{
}

unsafe impl<K, V, S: BuildHasher> GcSafe for HashMap<K, V, S>
where
    K: GcSafe,
    V: GcSafe,
    S: GcSafe,
{
}

unsafe impl<K: Scan, V: Scan, S: BuildHasher + GcSafe> Scan for HashMap<K, V, S> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for (k, v) in self {
            scanner.scan(k);
            scanner.scan(v);
        }
    }
}

unsafe impl<K: Finalize, V: Finalize, S: BuildHasher> Finalize for HashMap<K, V, S> {
    unsafe fn finalize(&mut self) {
        let map = read(self);
        for mut e in map {
            e.finalize();
            forget(e);
        }
    }
}

// HASHSET
unsafe impl<T, S: BuildHasher> GcDeref for HashSet<T, S>
where
    T: GcDeref,
    S: GcDeref,
{
}

unsafe impl<T, S: BuildHasher> GcDrop for HashSet<T, S>
where
    T: GcDrop,
    S: GcDrop,
{
}

unsafe impl<T, S: BuildHasher> GcSafe for HashSet<T, S>
where
    T: GcSafe,
    S: GcSafe,
{
}

unsafe impl<T: Scan, S: BuildHasher + GcSafe> Scan for HashSet<T, S> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for e in self {
            scanner.scan(e)
        }
    }
}

unsafe impl<T: Finalize, S: BuildHasher> Finalize for HashSet<T, S> {
    unsafe fn finalize(&mut self) {
        let set = read(self);
        for mut e in set {
            e.finalize();
            forget(e);
        }
    }
}

/// Vec like structure means that it implemented `Iter<T>`
#[macro_export]
macro_rules! sync_vec_like {
    ($t:ty) => {
        unsafe impl<T> GcDeref for $t where T: GcDeref {}
        unsafe impl<T> GcDrop for $t where T: GcDrop {}
        unsafe impl<T> GcSafe for $t where T: GcSafe {}

        unsafe impl<T: Scan> Scan for $t {
            #[inline]
            fn scan(&self, scanner: &mut Scanner<'_>) {
                for e in self {
                    scanner.scan(e)
                }
            }
        }

        unsafe impl<T: Finalize> Finalize for $t {
            unsafe fn finalize(&mut self) {
                let set = read(self);
                for mut e in set {
                    e.finalize();
                    forget(e);
                }
            }
        }
    };
    {$($t:ty,)*} => {
        $(sync_vec_like!($t);)*
    };
    {$($t:ty), *} => {
        sync_vec_like!($($t,)*);
    };
}

sync_vec_like![
    Vec<T>,
    VecDeque<T>, // avoid format
    LinkedList<T>,
    BTreeSet<T>,
    BinaryHeap<T>,
];

/// Map like structure means that it implemented `Iter<K, V>`
#[macro_export]
macro_rules! sync_map_like {
    ($t:ty) => {
        unsafe impl<K, V> GcDeref for $t where K: GcDeref, V: GcDeref { }
        unsafe impl<K, V> GcDrop for $t where K: GcDrop, V: GcDrop { }
        unsafe impl<K, V> GcSafe for $t where K: GcSafe, V: GcSafe { }

        unsafe impl<K: Scan, V: Scan> Scan for $t {
            #[inline]
            fn scan(&self, scanner: &mut Scanner<'_>) {
                for (k, v) in self {
                    scanner.scan(k);
                    scanner.scan(v);
                }
            }
        }

        unsafe impl<K: Finalize, V: Finalize> Finalize for $t {
            unsafe fn finalize(&mut self) {
                let map = read(self);
                for mut e in map {
                    e.finalize();
                    forget(e);
                }
            }
        }
    };
    {$($t:ty,)*} => {
        $(sync_map_like!($t);)*
    };
    {$($t:ty), *} => {
        sync_map_like!($($t,)*);
    };
}

sync_map_like![
    BTreeMap<K,V>  // avoid format
];

macro_rules! for_each_tuple_ {
    ($m:ident !!) => {
        $m! { }
    };
    ($m:ident !! $h:ident, $($t:ident,)*) => {
        $m! { $h $($t)* }
        for_each_tuple_! { $m !! $($t,)* }
    }
}

macro_rules! for_each_tuple {
    ($m:ident) => {
        for_each_tuple_! { $m !! A, B, C, D, E, F, G, H, I, J, K, L, M, N, }
    };
}

// See: https://github.com/rust-lang/rust/blob/8f5b5f94dcdb9884737dfbc8efd893d1d70f0b14/src/libcore/hash/mod.rs#L239-L271
macro_rules! sync_tuple {
    () => (
        // unsafe impl GcDeref for () {};
        // unsafe impl GcDrop for () {};
        // unsafe impl GcSafe for () {};
        // unsafe impl Scan for () {};
        // unsafe impl Finalize for () {};
    );

    ($($name:ident)+) => (
        unsafe impl<$($name: GcDeref),*> GcDeref for ($($name,)*) {}
        unsafe impl<$($name: GcDrop),*> GcDrop for ($($name,)*) {}
        unsafe impl<$($name: GcSafe),*> GcSafe for ($($name,)*) {}

        unsafe impl<$($name: Scan),*> Scan for ($($name,)*) {
            #[inline]
            #[allow(non_snake_case)]
            fn scan(&self, scanner: &mut Scanner<'_>) {
                let ($(ref $name,)*) = *self;
                $($name.scan(scanner);)*
            }
        }

        unsafe impl<$($name: Finalize),*> Finalize for ($($name,)*) {
            #[allow(non_snake_case)]
            unsafe fn finalize(&mut self) {
                let ($(mut $name,)*) = read(self);
                $($name.finalize();)*
                $(forget($name);)*
            }
        }
    );
}

// gc tuples up to 14, may enough
for_each_tuple!(sync_tuple);
