use std::collections::{HashMap, HashSet};

use std::alloc::{alloc, dealloc, Layout};
use std::iter::FromIterator;
use std::ptr;

use parking_lot::Mutex;

use crate::Scan;

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct GcDataPtr(*const dyn Scan);

impl GcDataPtr {
    // TODO: I'm pretty sure we can drop the 'static bound here thru careful lifetime manipulation
    // Basically we make T parametric on a lifetime 'a, then track that throughout
    // The subtly is that we sometimes scan objects after their lifetime ends
    // I think it's possible to make this work, but needs some examination
    fn allocate<T: Scan + 'static>(v: T) -> (Self, *const T) {
        // This is a straightforward use of alloc/write -- it should be undef free
        let data_ptr = unsafe {
            let heap_space = alloc(Layout::new::<T>()) as *mut T;
            ptr::write(heap_space, v);
            // NOTE: Write moves the data into the heap

            // Heap space is now a pointer to a T
            heap_space as *const T
        };

        let fat_ptr: *const dyn Scan = data_ptr;

        (Self(fat_ptr), data_ptr)
    }

    // This is unsafe, since we must externally guarantee that no-one still holds a pointer to the data
    // (Luckily this is the point of the garbage collector!)
    unsafe fn deallocate(self) {
        let dealloc_layout = Layout::for_value(&*self.0);
        let heap_ptr = self.0 as *mut u8;

        // TODO: Implement finalization / call finalizers here

        dealloc(heap_ptr, dealloc_layout);
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GcInternalHandle(u64);

pub struct Collector {
    // TODO: Replace "held_references" with a setup that keeps track of the state of each thread
    // (Then we could sometimes collect by pausing threads on command)
    held_references: u32,

    handle_idx_count: u64,

    // Percent more allocations needed to trigger garbage collection
    gc_trigger_percent: f32,
    data_count_at_last_collection: usize,

    data: HashSet<GcDataPtr>,
    handles: HashMap<GcInternalHandle, GcDataPtr>,
}

const DEFAULT_TRIGGER_PERCENT: f32 = 0.75;

unsafe impl Send for Collector {}

// Overall design
// Stop the world when we get get everyone out of the GC
//   (AKA, no-one has a reference to a GC'd object)
//   To this end, we keep a "held_references" count, incremented when a guard is taken, decremented when it's dropped
//   If an allocation happens or a guard is dropped, and "held_references" is zero, we consider a GC
//   If we start a GC, we stop everyone else from taking references
//
// After stopping we need to find the roots
//   To do this, we find all the handles held by any piece of GC data
//   If a handle is not held by any GC data, it must be held by non GC'd data, and is a root!
//   (Care must be taken to flag BackingGcHandles that no one holds)
//
// With a stopped world + roots, then we can simply mark and sweep

impl Collector {
    fn new() -> Self {
        Self {
            held_references: 0,

            handle_idx_count: 0,

            gc_trigger_percent: DEFAULT_TRIGGER_PERCENT,
            data_count_at_last_collection: 0,

            data: HashSet::new(),
            handles: HashMap::new(),
        }
    }

    fn synthesize_handle(&mut self) -> GcInternalHandle {
        let handle = GcInternalHandle(self.handle_idx_count);
        self.handle_idx_count += 1;
        handle
    }

    pub fn track_data<T: Scan + 'static>(&mut self, data: T) -> (GcInternalHandle, *const T) {
        let (gc_data_ptr, heap_ptr) = GcDataPtr::allocate(data);
        let handle = self.synthesize_handle();

        self.data.insert(gc_data_ptr);
        assert!(!self.handles.contains_key(&handle));
        self.handles.insert(handle.clone(), gc_data_ptr);

        let res = (handle, heap_ptr);

        // When we allocate, the heuristic for whether we need to GC might change
        self.check_then_collect();

        res
    }

    pub fn drop_handle(&mut self, handle: &GcInternalHandle) {
        self.handles.remove(handle);

        // TODO: Consider if this is useful
        // This will only trigger a collection if `gc_trigger_percent` == 0.0
        // (It's in for now since it eases testing)
        self.check_then_collect();
    }

    pub fn clone_handle(&mut self, handle: &GcInternalHandle) -> GcInternalHandle {
        let data = *self
            .handles
            .get(handle)
            .expect("Can only copy real handles!");
        let new_handle = self.synthesize_handle();
        self.handles.insert(new_handle.clone(), data);

        new_handle
    }

    pub fn inc_held_references(&mut self) {
        self.held_references += 1;
    }

    pub fn dec_held_references(&mut self) {
        self.held_references -= 1;
        // Dropping a reference might put us in a Gc-able state
        self.check_then_collect();
    }

    pub fn tracked_data_count(&self) -> usize {
        self.data.len()
    }

    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    pub fn set_gc_trigger_percent(&mut self, new_trigger_percent: f32) {
        self.gc_trigger_percent = new_trigger_percent;
    }

    pub fn check_then_collect(&mut self) -> bool {
        let percent_more_data = (self.tracked_data_count() - self.data_count_at_last_collection)
            as f32
            / self.data_count_at_last_collection as f32;

        if !percent_more_data.is_finite() || percent_more_data >= self.gc_trigger_percent {
            self.collect()
        } else {
            false
        }
    }

    pub fn collect(&mut self) -> bool {
        if self.held_references > 0 {
            return false;
        }

        let mut roots: HashSet<GcInternalHandle> = self.handles.keys().cloned().collect();

        let mut gc_managed_handles = Vec::new();
        for GcDataPtr(d) in &self.data {
            let v = unsafe { &**d };
            v.scan(&mut gc_managed_handles);
        }

        // The roots are those handles not managed by the garbage collector
        roots.retain(|handle| !gc_managed_handles.contains(handle));

        // Now let's basically do DFS
        let mut frontier_stack: Vec<GcInternalHandle> = Vec::from_iter(roots.iter().cloned());
        let mut marked = roots;

        let mut scan_buf: Vec<GcInternalHandle> = Vec::new();
        while let Some(handle) = frontier_stack.pop() {
            // Clear the scan buffer
            scan_buf.clear();
            // Then populate the scan buffer
            let data_to_scan = self
                .handles
                .get(&handle)
                .expect("This handle came from this map's keys!")
                .0;
            unsafe {
                (&*data_to_scan).scan(&mut scan_buf);
            }

            // Now mark all data
            for h in &scan_buf {
                // If we haven't marked this yet, we need to add it frontier
                if !marked.contains(h) {
                    frontier_stack.push(h.clone());
                    marked.insert(h.clone());
                }
            }
        }

        // Now delete all handles that are not reachable
        self.handles.retain(|k, _| marked.contains(k));

        // Now deallocate all unreachable values
        let reachable_data: HashSet<GcDataPtr> = self.handles.values().cloned().collect();
        let mut unreachable_data: Vec<GcDataPtr> = Vec::new();
        for v in &self.data {
            if !reachable_data.contains(v) {
                unreachable_data.push(v.clone());
            }
        }

        trace!("In collection, reachable data {:?}", reachable_data);
        self.data = reachable_data.into_iter().collect();

        for d in unreachable_data {
            unsafe {
                d.deallocate();
            }
        }

        self.data_count_at_last_collection = self.tracked_data_count();

        true
    }
}

lazy_static! {
    pub static ref COLLECTOR: Mutex<Collector> = Mutex::new(Collector::new());
}
