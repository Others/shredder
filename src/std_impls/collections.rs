use crate::marker::{GcDeref, GcDrop, GcSafe};
use crate::{Finalize, Scan, Scanner};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::BuildHasher;
use std::mem::forget;
use std::prelude::v1::*;
use std::ptr::read;

// For pretty much all simple collections, the collection inherets the properites of what it contains
// (with respect to GcDeref, GcDrop and GcSafe)

// BTREEMAP
unsafe impl<K, V> GcDeref for BTreeMap<K, V>
where
    K: GcDeref,
    V: GcDeref,
{
}
unsafe impl<K, V> GcDrop for BTreeMap<K, V>
where
    K: GcDrop,
    V: GcDrop,
{
}
unsafe impl<K, V> GcSafe for BTreeMap<K, V>
where
    K: GcSafe,
    V: GcSafe,
{
}

unsafe impl<K: Scan, V: Scan> Scan for BTreeMap<K, V> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for (k, v) in self {
            scanner.scan(k);
            scanner.scan(v);
        }
    }
}

unsafe impl<K: Finalize, V: Finalize> Finalize for BTreeMap<K, V> {
    unsafe fn finalize(&mut self) {
        let map = read(self);
        for mut e in map {
            e.finalize();
            forget(e);
        }
    }
}

// BTREESET
unsafe impl<T> GcDeref for BTreeSet<T> where T: GcDeref {}
unsafe impl<T> GcDrop for BTreeSet<T> where T: GcDrop {}
unsafe impl<T> GcSafe for BTreeSet<T> where T: GcSafe {}

unsafe impl<T: Scan> Scan for BTreeSet<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for e in self {
            scanner.scan(e)
        }
    }
}

unsafe impl<T: Finalize> Finalize for BTreeSet<T> {
    unsafe fn finalize(&mut self) {
        let set = read(self);
        for mut e in set {
            e.finalize();
            forget(e);
        }
    }
}

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

// TUPLES
unsafe impl<A, B> GcDeref for (A, B)
where
    A: GcDeref,
    B: GcDeref,
{
}
unsafe impl<A, B> GcDrop for (A, B)
where
    A: GcDrop,
    B: GcDrop,
{
}
unsafe impl<A, B> GcSafe for (A, B)
where
    A: GcSafe,
    B: GcSafe,
{
}

unsafe impl<A: Scan, B: Scan> Scan for (A, B) {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        self.0.scan(scanner);
        self.1.scan(scanner);
    }
}

unsafe impl<A: Finalize, B: Finalize> Finalize for (A, B) {
    unsafe fn finalize(&mut self) {
        let (mut a, mut b) = read(self);
        a.finalize();
        b.finalize();
        forget(a);
        forget(b);
    }
}

// VEC
unsafe impl<T> GcDeref for Vec<T> where T: GcDeref {}
unsafe impl<T> GcDrop for Vec<T> where T: GcDrop {}
unsafe impl<T> GcSafe for Vec<T> where T: GcSafe {}

unsafe impl<T: Scan> Scan for Vec<T> {
    #[inline]
    fn scan(&self, scanner: &mut Scanner<'_>) {
        for e in self {
            scanner.scan(e)
        }
    }
}

unsafe impl<T: Finalize> Finalize for Vec<T> {
    unsafe fn finalize(&mut self) {
        let set = read(self);
        for mut e in set {
            e.finalize();
            forget(e);
        }
    }
}
