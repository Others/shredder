use std::alloc::{alloc, dealloc, Layout};
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
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
        // We want to get a snapshot of what the handles and the data look like
        let data_snapshot: HashMap<GcDataPtr, Arc<Lockout>> = gc_data.data.clone();
        let handle_snapshot: HashMap<GcInternalHandle, (GcDataPtr, Arc<Lockout>)> =
            gc_data.handles.clone();

        let mut scanables: Vec<(GcInternalHandle, &GcDataPtr, ExclusiveWarrant)> = Vec::new();
        let mut unscanables: Vec<GcInternalHandle> = Vec::new();
        for (handle, (data_ptr, lockout)) in &handle_snapshot {
            if let Some(guard) = lockout.get_exclusive_warrant() {
                scanables.push((handle.clone(), data_ptr, guard));
            } else {
                unscanables.push(handle.clone());
            }
        }
        drop(gc_data);

        // Note: We now are operating on an old copy of the data. However, that's okay
        // Intuition: If data was unreachable then, it's unreachable now

        // Now do scan, since we'll need that information
        let mut roots: HashSet<GcInternalHandle> = handle_snapshot.keys().cloned().collect();
        let mut scan_results: HashMap<GcInternalHandle, Vec<GcInternalHandle>> = HashMap::new();
        for (handle, &data_ptr, _) in &scanables {
            let mut scanner = Scanner::new();
            let to_scan = unsafe { &*data_ptr.0 };
            to_scan.scan(&mut scanner);

            let results = scanner.extract_found_handles();
            for h in &results {
                roots.remove(h);
            }

            scan_results.insert(handle.clone(), results);
        }
        drop(scanables);
        // If something couldn't be scanned, it must be in use
        roots.extend(unscanables.into_iter());

        // Now let's basically do DFS
        let mut frontier_stack: Vec<GcInternalHandle> = Vec::from_iter(roots.iter().cloned());
        let mut marked_data: HashSet<GcDataPtr> = roots
            .iter()
            .map(|k| {
                handle_snapshot
                    .get(k)
                    .expect("We got the roots from this snapshot!")
                    .0
            })
            .collect();
        let mut marked_handles = roots;

        let empty_vec = Vec::new();
        while let Some(handle) = frontier_stack.pop() {
            // Now mark all data
            for h in scan_results.get(&handle).unwrap_or(&empty_vec) {
                // If we haven't marked this yet, we need to add it frontier
                if !marked_handles.contains(h) {
                    frontier_stack.push(h.clone());

                    let v = handle_snapshot
                        .get(h)
                        .expect("We got the handles from this snapshot!")
                        .0;
                    marked_data.insert(v);
                    marked_handles.insert(h.clone());
                }
            }
        }

        let unreachable_handles: HashSet<GcInternalHandle> = handle_snapshot
            .keys()
            .filter(|&v| !marked_handles.contains(v))
            .cloned()
            .collect();

        let unreachable_data: HashSet<GcDataPtr> = data_snapshot
            .keys()
            .filter(|&v| !marked_data.contains(v))
            .cloned()
            .collect();

        let mut gc_data = self.gc_data.write();
        for h in &unreachable_handles {
            gc_data.handles.remove(h);
        }

        let drop_thread_chan = self.drop_thread_chan.lock();
        for d in &unreachable_data {
            if let Some((ptr, _)) = gc_data.data.remove_entry(d) {
                drop_thread_chan
                    .send(ptr)
                    .expect("drop thread should be infallible");
            }
        }
        drop(gc_data);

        trigger_data.data_count_at_last_collection = self.tracked_data_count();

        trace!("Collection finished");
    }
}

pub static COLLECTOR: Lazy<Arc<Collector>> = Lazy::new(Collector::new);
