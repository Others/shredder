use std::alloc::{alloc, dealloc, Layout};
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::panic::{catch_unwind, UnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::spawn;

use once_cell::sync::Lazy;
use parking_lot::{Mutex, MutexGuard, RwLock, RwLockWriteGuard, RwLockUpgradableReadGuard};

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

    fn scan<F: FnMut(GcInternalHandle)>(&self, callback: F) {
        unsafe {
            let mut scanner = Scanner::new(callback);
            let to_scan = &*self.0;
            to_scan.scan(&mut scanner);
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

#[derive(Debug)]
struct GcMetadata {
    lockout: Arc<Lockout>,
    last_marked: u64,
}

#[derive(Debug)]
struct HandleMetadata {
    underlying_ptr: GcDataPtr,
    lockout: Arc<Lockout>,
    last_non_rooted: u64,
}

#[derive(Debug)]
struct TrackedGcData {
    collection_number: u64,
    data: HashMap<GcDataPtr, GcMetadata>,
    handles: HashMap<GcInternalHandle, HandleMetadata>,
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
                collection_number: 1,
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
        gc_data.data.insert(gc_data_ptr, GcMetadata {
            lockout: lockout.clone(),
            last_marked: 0
        });
        assert!(!gc_data.handles.contains_key(&handle));
        gc_data
            .handles
            .insert(handle.clone(), HandleMetadata{
                underlying_ptr: gc_data_ptr,
                lockout,
                last_non_rooted: 0
            });
        drop(gc_data);

        let res = (handle, heap_ptr);

        // When we allocate, the heuristic for whether we need to GC might change
        self.async_gc_chan
            .lock()
            .send(())
            .expect("notifying the async gc thread should always succeed");

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

        let new_metadata = {
            let handle_metadata = gc_data
                .handles
                .get(handle)
                .expect("Tried to clone a Gc, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");
            HandleMetadata{
                underlying_ptr: handle_metadata.underlying_ptr,
                lockout: handle_metadata.lockout.clone(),
                last_non_rooted: 0
            }
        };

        let new_handle = self.synthesize_handle();
        gc_data
            .handles
            .insert(new_handle.clone(), new_metadata);

        new_handle
    }

    pub fn get_data_warrant(&self, handle: &GcInternalHandle) -> Warrant {
        // Note: On panic, the lock is freed normally -- which is what we want
        let gc_data = self.gc_data.read();

        let handle_metadata = gc_data.handles.get(handle)
            .expect("Tried to access Gc data, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");

        handle_metadata.lockout.get_warrant()
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
        let gc_data = self.gc_data.upgradable_read();

        let tracked_data_count = gc_data.data.len();
        let new_data_count = tracked_data_count - trigger_data.data_count_at_last_collection;
        let percent_more_data =
            new_data_count as f32 / trigger_data.data_count_at_last_collection as f32;

        let at_min_allocations = tracked_data_count as f32 > MIN_ALLOCATIONS_FOR_COLLECTION;
        let infinite_percent_extra_data = !percent_more_data.is_finite();
        let above_trigger_percent = percent_more_data >= trigger_data.gc_trigger_percent;
        if at_min_allocations && (infinite_percent_extra_data || above_trigger_percent) {
            self.do_collect(trigger_data, RwLockUpgradableReadGuard::upgrade(gc_data));
            true
        } else {
            false
        }
    }

    pub fn collect(&self) {
        let trigger_data = self.trigger_data.lock();
        let gc_data = self.gc_data.write();
        self.do_collect(trigger_data, gc_data);
    }

    // TODO(issue): https://github.com/Others/shredder/issues/13
    // TODO: Remove the vectors we allocate here with an intrusive linked list
    // TODO: Reconsider the lockout mechanism (is the memory usage too high?)
    #[allow(clippy::shadow_unrelated)]
    fn do_collect(
        &self,
        mut trigger_data: MutexGuard<TriggerData>,
        mut gc_data_guard: RwLockWriteGuard<TrackedGcData>,
    ) {
        trace!("Beginning collection");

        gc_data_guard.collection_number += 1;
        let current_collection = gc_data_guard.collection_number;

        // The warrant system prevents us from
        let mut warrants: Vec<ExclusiveWarrant> = Vec::new();

        let gc_data = &mut *gc_data_guard;
        let tracked_data = &mut gc_data.data;
        let tracked_handles = &mut gc_data.handles;

        // eprintln!("tracked data {:?}", tracked_data);
        // eprintln!("tracked handles {:?}", tracked_handles);

        for (gc_data_ptr, metadata) in &mut *tracked_data {
            if let Some(warrant) = metadata.lockout.get_exclusive_warrant() {
                // Save that warrant so things can't shift around under us
                warrants.push(warrant);

                // Now figure out what handles are not rooted
                gc_data_ptr.scan(|h| {
                    let found_handle_metadata = tracked_handles.get_mut(&h).expect("should always find a handle if it's present in data");
                    found_handle_metadata.last_non_rooted = current_collection;
                });
            } else {
                // eprintln!("failed to get warrant!");
                // If we can't get the warrant, then this data must be in use, so we can mark it
                metadata.last_marked = current_collection;
            }
        }

        let tracked_handles = &gc_data.handles;
        let mut roots = Vec::new();
        for (handle, handle_metadata) in tracked_handles {
            // If the `last_non_rooted` number was not now, then it is a root
            if handle_metadata.last_non_rooted != current_collection {
                roots.push((handle.clone(), handle_metadata));
            }
        }

        // eprintln!("roots {:?}", roots);

        let mut dfs_stack = roots;
        while let Some((_, handle_metadata)) = dfs_stack.pop() {
            let data_ptr = &handle_metadata.underlying_ptr;
            let ptr_metadata = tracked_data.get_mut(data_ptr).expect("all data must have associated metadata");

            // Essential note! Since all non warranted data is automatically marked, we will never accidently scan non-warranted data here
            if ptr_metadata.last_marked != current_collection {
                ptr_metadata.last_marked = current_collection;

                data_ptr.scan(|h| {
                    let metadata = tracked_handles.get(&h).expect("all handles must have metadata");
                    dfs_stack.push((h, metadata));
                });
            }
        }

        let tracked_handles = &mut gc_data.handles;
        let drop_thread_chan = self.drop_thread_chan.lock();
        tracked_data.retain(|data_ptr, ptr_metadata| {
            // If this is true, we just marked this data
            if ptr_metadata.last_marked == current_collection {
                // so retain it
                true
            } else {
                // eprintln!("deallocating {:?}", data_ptr);

                data_ptr.scan(|h| {
                    tracked_handles.remove(&h);
                });

                // Otherwise set this data up to be deallocated
                if let Err(e) = drop_thread_chan.send(data_ptr.clone()) {
                    error!("Error sending to drop thread {}", e);
                }

                // Note: It's okay to send all the data before we've completely removed it from the map
                // The cross check will stall till we release the data lock

                // Don't retain this data
                false
            }
        });
        drop(gc_data_guard);

        trigger_data.data_count_at_last_collection = self.tracked_data_count();

        trace!("Collection finished");
    }
}

pub static COLLECTOR: Lazy<Arc<Collector>> = Lazy::new(Collector::new);
