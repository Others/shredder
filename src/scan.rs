use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use crate::collector::GcInternalHandle;
use crate::Gc;

// TODO: Add non-'static data as an option
//  Enhance Scan with distinction between options
//  Add flag, so we don't run destructors for non-'static data

// TODO: Expand this to explain the Scan/SendScan business
// Scan is unsafe, because all Scan types must satisfy the following requirements:
// 1) If no one else has a reference to a T, it's okay for the "scan" method to be called from any thread
// 2) T can be dropped from any thread safely
// If T is send + 'static, T satisfies these requirements
pub unsafe trait Scan {
    // Note: This could technically be a HashSet, but handles typically have one owner
    // (So using a HashSet is just extra overhead)
    // Unsafe, because safe code cannot have `GcInternalHandle`s
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>);
}

pub trait SendScan: Send + 'static {
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>);
}

unsafe impl<T: SendScan> Scan for T {
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        SendScan::scan(self, out)
    }
}

// Fundamental to the Scan system is that Gc<T> yields its underlying handle
unsafe impl<T: Scan> Scan for Gc<T> {
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        out.push(self.internal_handle())
    }
}

// Primitives do not hold any Gc<T>s
impl SendScan for usize {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

impl SendScan for isize {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

impl SendScan for u32 {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

impl SendScan for i32 {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

impl SendScan for f32 {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

impl SendScan for u64 {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

impl SendScan for i64 {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

impl SendScan for f64 {
    unsafe fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

// TODO: Add more Scan impls

// For collections that own their elements, Collection<T>: Scan iff T: Scan
unsafe impl<T: Scan> Scan for Vec<T> {
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        for v in self {
            v.scan(out);
        }
    }
}

unsafe impl<T: Scan, S: BuildHasher> Scan for HashSet<T, S> {
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        for v in self {
            v.scan(out);
        }
    }
}

unsafe impl<K: Scan, V: Scan, S: BuildHasher> Scan for HashMap<K, V, S> {
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        for (k, v) in self {
            k.scan(out);
            v.scan(out);
        }
    }
}

unsafe impl<T: Scan> Scan for RefCell<T> {
    unsafe fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        if let Ok(reference) = self.try_borrow() {
            reference.scan(out);
        }
    }
}

// TODO: Add a Scan auto-derive

// TODO: Consider what happens if there are reference cycles (like a Gc -> Arc<A> -> A -> Arc<B> -> B -> Arc<A>)
// This could lead to an infinite loop during scanning
// To fix this, we'd have to change how the scan type works, with broadly three options
// - Keep track of visited items during scanning internally
// - Return a vector of Scan children instead of GcInternalHandle
// - Make Rc/Arc not Scan-able
// For now we are going with the third option
