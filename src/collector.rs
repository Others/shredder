use std::alloc::{alloc, dealloc, Layout};
use std::collections::{HashMap, HashSet};
use std::mem::ManuallyDrop;
use std::panic::{catch_unwind, UnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::spawn;

use once_cell::sync::Lazy;
use parking_lot::{Mutex, MutexGuard, RwLock, RwLockReadGuard};

use crate::lockout::{ExclusiveWarrant, Lockout, Warrant};
use crate::{Scan, Scanner};

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct GcDataPtr(*const dyn Scan);

// We need this for the drop thread. By that point we have exclusive access to the data
// It also, by contract of Scan, cannot have a Drop method that is unsafe in any thead
unsafe impl Send for GcDataPtr {}
// Therefore, GcDataPtr is also UnwindSafe in the context we need it to be
impl UnwindSafe for GcDataPtr {}
// We use the lockout to ensure that `GcDataPtr`s are not shared
unsafe impl Sync for GcDataPtr {}

impl GcDataPtr {
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
        let scan_ptr: *const dyn Scan = self.0;

        // This calls the destructor of the Scan data
        {
            // Safe type shift: the contract of this method is that the scan_ptr doesn't alias
            // + ManuallyDrop is repr(transparent)
            let droppable_ptr: *mut ManuallyDrop<dyn Scan> =
                scan_ptr as *mut ManuallyDrop<dyn Scan>;
            let droppable_ref = &mut *droppable_ptr;
            ManuallyDrop::drop(droppable_ref);
        }

        let dealloc_layout = Layout::for_value(&*scan_ptr);
        let heap_ptr = scan_ptr as *mut u8;
        dealloc(heap_ptr, dealloc_layout);
    }

    fn scan(&self) -> Vec<GcInternalHandle> {
        unsafe {
            let mut scanner = Scanner::new();
            let to_scan = &*self.0;
            to_scan.scan(&mut scanner);

            scanner.extract_found_handles()
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GcInternalHandle(u64);
impl GcInternalHandle {
    pub(crate) fn new(n: u64) -> Self {
        Self(n)
    }
}

struct TriggerData {
    // Percent more allocations needed to trigger garbage collection
    gc_trigger_percent: f32,
    data_count_at_last_collection: usize,
}

struct TrackedGcData {
    data: HashMap<GcDataPtr, Arc<Lockout>>,
    handles: HashMap<GcInternalHandle, (GcDataPtr, Arc<Lockout>)>,
}

pub struct Collector {
    handle_idx_count: AtomicU64,
    trigger_data: Mutex<TriggerData>,
    drop_thread_chan: Mutex<Sender<GcDataPtr>>,
    async_gc_chan: Mutex<Sender<()>>,
    gc_data: RwLock<TrackedGcData>,
}

// TODO(issue): https://github.com/Others/shredder/issues/8
const DEFAULT_TRIGGER_PERCENT: f32 = 0.75;
const MIN_ALLOCATIONS_FOR_COLLECTION: f32 = 512.0 * 1.3;

// TODO(issue): https://github.com/Others/shredder/issues/7

impl Collector {
    fn new() -> Arc<Self> {
        let (drop_sender, drop_receiver) = mpsc::channel::<GcDataPtr>();

        // The drop thread deals with doing all the Drops this collector needs to do
        spawn(move || {
            // An Err value means the stream will never recover
            while let Ok(ptr) = drop_receiver.recv() {
                // Deallocate / Run Drop
                let res = catch_unwind(move || unsafe {
                    ptr.deallocate();
                });
                if let Err(e) = res {
                    eprintln!("Gc background drop failed: {:?}", e);
                }
            }
        });

        let (async_gc_trigger, async_gc_receiver) = mpsc::channel::<()>();

        let res = Arc::new(Self {
            handle_idx_count: AtomicU64::new(1),
            trigger_data: Mutex::new(TriggerData {
                gc_trigger_percent: DEFAULT_TRIGGER_PERCENT,
                data_count_at_last_collection: 0,
            }),
            async_gc_chan: Mutex::new(async_gc_trigger),
            drop_thread_chan: Mutex::new(drop_sender),
            gc_data: RwLock::new(TrackedGcData {
                data: HashMap::new(),
                handles: HashMap::new(),
            }),
        });

        // The async Gc thread deals with background Gc'ing
        let async_collector_ref = Arc::downgrade(&res);
        spawn(move || {
            // An Err value means the stream will never recover
            while let Ok(_) = async_gc_receiver.recv() {
                if let Some(collector) = async_collector_ref.upgrade() {
                    collector.check_then_collect();
                }
            }
        });

        res
    }

    fn synthesize_handle(&self) -> GcInternalHandle {
        let n = self.handle_idx_count.fetch_add(1, Ordering::SeqCst);
        GcInternalHandle::new(n)
    }

    pub fn track_data<T: Scan + 'static>(&self, data: T) -> (GcInternalHandle, *const T) {
        let (gc_data_ptr, heap_ptr) = GcDataPtr::allocate(data);
        let handle = self.synthesize_handle();
        let lockout = Lockout::new();

        let mut gc_data = self.gc_data.write();
        gc_data.data.insert(gc_data_ptr, lockout.clone());
        assert!(!gc_data.handles.contains_key(&handle));
        gc_data
            .handles
            .insert(handle.clone(), (gc_data_ptr, lockout));
        drop(gc_data);

        let res = (handle, heap_ptr);

        // When we allocate, the heuristic for whether we need to GC might change
        self.async_gc_chan
            .lock()
            .send(())
            .expect("We should always be able to");

        res
    }

    pub fn drop_handle(&self, handle: &GcInternalHandle) {
        let mut gc_data = self.gc_data.write();

        gc_data.handles.remove(handle);

        // NOTE: We probably don't want to collect here since it can happen while we are dropping from a previous collection
        // self.async_gc_chan.lock().send(());
    }

    pub fn clone_handle(&self, handle: &GcInternalHandle) -> GcInternalHandle {
        // Note: On panic, the lock is freed normally -- which is what we want
        let mut gc_data = self.gc_data.write();

        let (data_ptr, data_lock) = gc_data
            .handles
            .get(handle)
            .expect("Tried to clone a Gc, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");

        let data_ptr = *data_ptr;
        let data_lock = data_lock.clone();

        let new_handle = self.synthesize_handle();
        gc_data
            .handles
            .insert(new_handle.clone(), (data_ptr, data_lock));

        new_handle
    }

    pub fn get_data_warrant(&self, handle: &GcInternalHandle) -> Warrant {
        // Note: On panic, the lock is freed normally -- which is what we want
        let gc_data = self.gc_data.read();

        let (_, lockout) = gc_data.handles.get(handle)
            .expect("Tried to access Gc data, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");

        lockout.get_warrant()
    }

    pub fn handle_valid(&self, handle: &GcInternalHandle) -> bool {
        let gc_data = self.gc_data.read();
        gc_data.handles.get(handle).is_some()
    }

    pub fn tracked_data_count(&self) -> usize {
        let gc_data = self.gc_data.read();
        gc_data.data.len()
    }

    pub fn handle_count(&self) -> usize {
        let gc_data = self.gc_data.read();
        gc_data.handles.len()
    }

    pub fn set_gc_trigger_percent(&self, new_trigger_percent: f32) {
        self.trigger_data.lock().gc_trigger_percent = new_trigger_percent;
    }

    pub fn check_then_collect(&self) -> bool {
        let trigger_data = self.trigger_data.lock();
        let gc_data = self.gc_data.read();

        let tracked_data_count = gc_data.data.len();
        let new_data_count = tracked_data_count - trigger_data.data_count_at_last_collection;
        let percent_more_data =
            new_data_count as f32 / trigger_data.data_count_at_last_collection as f32;

        let at_min_allocations = tracked_data_count as f32 > MIN_ALLOCATIONS_FOR_COLLECTION;
        let infinite_percent_extra_data = !percent_more_data.is_finite();
        let above_trigger_percent = percent_more_data >= trigger_data.gc_trigger_percent;
        if at_min_allocations && (infinite_percent_extra_data || above_trigger_percent) {
            self.do_collect(trigger_data, gc_data);
            true
        } else {
            false
        }
    }

    pub fn collect(&self) {
        let trigger_data = self.trigger_data.lock();
        let gc_data = self.gc_data.read();
        self.do_collect(trigger_data, gc_data);
    }

    // TODO(issue): https://github.com/Others/shredder/issues/13
    fn do_collect(
        &self,
        mut trigger_data: MutexGuard<TriggerData>,
        gc_data: RwLockReadGuard<TrackedGcData>,
    ) {
        trace!("Beginning collection");

        // First create a graph of all the data
        let mut data_graph = HashMap::with_capacity(gc_data.data.len());
        // At this point we can also create a "root" list, of handles outside the managed heap
        let mut roots: HashSet<GcInternalHandle> = gc_data.handles.keys().cloned().collect();
        // We need a coherent "moment in time", so we can't release any guard till the end of scanning
        let mut warrants: Vec<ExclusiveWarrant> = Vec::with_capacity(gc_data.data.len());
        for (gc_data_ptr, lockout) in &gc_data.data {
            data_graph.insert(
                gc_data_ptr.clone(),
                if let Some(warrant) = lockout.get_exclusive_warrant() {
                    warrants.push(warrant);

                    let handles_for_data = gc_data_ptr.scan();
                    for h in &handles_for_data {
                        roots.remove(h);
                    }

                    handles_for_data
                } else {
                    Vec::new()
                },
            );
        }
        let handle_data_mapping: HashMap<GcInternalHandle, GcDataPtr> = gc_data
            .handles
            .iter()
            .map(|(handle, (data, _))| (handle.clone(), *data))
            .collect();

        // Now we drop the `gc_data` lock, and operate on "old data"
        // Intuition: If data was unreachable then, it's unreachable now
        drop(gc_data);

        // During scanning we need data about which handles are reachable
        let mut handle_reachable: HashMap<GcInternalHandle, bool> = handle_data_mapping
            .keys()
            .map(|v| (v.clone(), roots.contains(v)))
            .collect();

        // Now perform DFS on the handles in order to populate the "handle_reachable" data correctly
        let mut dfs_queue: Vec<GcInternalHandle> = roots.into_iter().collect();
        while let Some(handle) = dfs_queue.pop() {
            let data = handle_data_mapping
                .get(&handle)
                .expect("All handles must have data");
            let adj = data_graph.get(data).expect("All data must be in graph");

            for neighbor in adj {
                // If the handle was not yet marked as reachable
                if !handle_reachable.get(neighbor).cloned().unwrap_or(false) {
                    // Mark as reachable
                    handle_reachable.insert(neighbor.clone(), true);
                    // Add to queue
                    dfs_queue.push(neighbor.clone());
                }
            }
        }

        // Now we know which handles are unreachable. Use that to calculate what data is unreachable
        let mut data_reachable: HashMap<GcDataPtr, bool> =
            data_graph.keys().map(|data| (*data, false)).collect();
        for (handle, &reachable) in &handle_reachable {
            if reachable {
                let data = handle_data_mapping
                    .get(handle)
                    .expect("All handles must have data");
                data_reachable.insert(data.clone(), true);
            }
        }

        // Now cleanup unreachable data
        let mut gc_data = self.gc_data.write();
        let drop_thread_chan = self.drop_thread_chan.lock();

        for (handle, reachable) in handle_reachable {
            if !reachable {
                gc_data.handles.remove(&handle);
            }
        }

        for (ptr, reachable) in data_reachable {
            if !reachable {
                gc_data.data.remove(&ptr);
                if let Err(e) = drop_thread_chan.send(ptr) {
                    error!("Error sending to drop thread {}", e);
                }
            }
        }
        drop(gc_data);

        trigger_data.data_count_at_last_collection = self.tracked_data_count();

        trace!("Collection finished");
    }
}

pub static COLLECTOR: Lazy<Arc<Collector>> = Lazy::new(Collector::new);
