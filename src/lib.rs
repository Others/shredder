// I'd like the most pedantic warning level
#![warn(
    clippy::cargo,
    clippy::needless_borrow,
    clippy::pedantic,
    clippy::redundant_clone
)]
// But I don't care about these ones
#![allow(
    clippy::cast_precision_loss,     // There is no way to avoid this precision loss
    clippy::module_name_repetitions, // Sometimes clear naming calls for repetition
    clippy::multiple_crate_versions  // There is no way to easily fix this without modifying our dependencies
)]

#[macro_use]
extern crate lazy_static;

mod collector;

use std::ops::Deref;

use crate::collector::{GcInternalHandle, COLLECTOR};

#[derive(Debug)]
pub struct Gc<T: Scan> {
    backing_handle: GcInternalHandle,
    direct_ptr: *const T,
}

impl<T: Scan> Gc<T> {
    pub fn new(v: T) -> Self
    where
        T: 'static,
    {
        let (handle, ptr) = COLLECTOR.lock().track_data(v);
        Self {
            backing_handle: handle,
            direct_ptr: ptr,
        }
    }

    pub fn get(&self) -> GcGuard<T> {
        COLLECTOR.lock().inc_held_references();
        GcGuard { gc_ptr: self }
    }
}

impl<T: Scan> Clone for Gc<T> {
    fn clone(&self) -> Self {
        let new_handle = COLLECTOR.lock().clone_handle(self.backing_handle);

        Self {
            backing_handle: new_handle,
            direct_ptr: self.direct_ptr,
        }
    }
}

unsafe impl<T: Scan> Send for Gc<T> where T: Send {}

impl<T: Scan> Drop for Gc<T> {
    fn drop(&mut self) {
        COLLECTOR.lock().drop_handle(self.backing_handle);
    }
}

pub struct GcGuard<'a, T: Scan> {
    gc_ptr: &'a Gc<T>,
}

impl<'a, T: Scan> Drop for GcGuard<'a, T> {
    fn drop(&mut self) {
        COLLECTOR.lock().dec_held_references();
    }
}

impl<'a, T: Scan> Deref for GcGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.gc_ptr.direct_ptr }
    }
}

pub unsafe trait Scan {
    // TODO: Consider if a HashSet would be a better fit
    fn scan(&self, out: &mut Vec<GcInternalHandle>);
}

unsafe impl Scan for i32 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl Scan for u32 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl Scan for i64 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl Scan for u64 {
    fn scan(&self, _: &mut Vec<GcInternalHandle>) {}
}

unsafe impl<T: Scan> Scan for Gc<T> {
    fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        out.push(self.backing_handle.clone())
    }
}

unsafe impl<T: Scan> Scan for Vec<T> {
    fn scan(&self, out: &mut Vec<GcInternalHandle>) {
        for v in self {
            v.scan(out);
        }
    }
}

pub fn tracked_data_count() -> usize {
    COLLECTOR.lock().tracked_data_count()
}

pub fn active_gc_handle_count() -> usize {
    COLLECTOR.lock().handle_count()
}

// TODO: Add many more tests
// TODO: Run tests under valgrind
// TODO: Add asserts to tests
// TODO: Make tests work when run in parallel
#[cfg(test)]
mod test {
    use crate::collector::{GcInternalHandle, COLLECTOR};

    use super::{Gc, Scan};

    #[derive(Debug)]
    struct DirectedGraphNode {
        label: String,
        edges: Vec<Gc<DirectedGraphNode>>,
    }

    unsafe impl Scan for DirectedGraphNode {
        fn scan(&self, out: &mut Vec<GcInternalHandle>) {
            self.edges.scan(out);
        }
    }

    #[test]
    fn alloc_u32_gc() {
        println!(
            "Test 1 START: tracking {} pieces of data",
            COLLECTOR.lock().tracked_data_count()
        );
        let val = 7;

        let gc_ptr = Gc::new(val);
        assert_eq!(*gc_ptr.get(), val);
        std::mem::drop(gc_ptr);

        println!(
            "Test 1 END: tracking {} pieces of data",
            COLLECTOR.lock().tracked_data_count()
        );
    }

    #[test]
    fn alloc_directed_graph_node_gc() {
        println!(
            "Test 2 START: tracking {} pieces of data",
            COLLECTOR.lock().tracked_data_count()
        );

        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr = Gc::new(node);
        assert_eq!(gc_ptr.get().label, "A");

        std::mem::drop(gc_ptr);
        println!(
            "Test 2 END: tracking {} pieces of data",
            COLLECTOR.lock().tracked_data_count()
        );
    }

    #[test]
    fn clone_directed_graph_node_gc() {
        println!(
            "Test 3 START: tracking {} pieces of data",
            COLLECTOR.lock().tracked_data_count()
        );
        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr_one = Gc::new(node);
        let gc_ptr_two = gc_ptr_one.clone();

        assert_eq!(gc_ptr_one.get().label, "A");
        assert_eq!(gc_ptr_two.get().label, "A");
        std::mem::drop(gc_ptr_one);
        std::mem::drop(gc_ptr_two);

        println!(
            "Test 3 END: tracking {} pieces of data",
            COLLECTOR.lock().tracked_data_count()
        );
    }
}
