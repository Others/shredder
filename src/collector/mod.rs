mod alloc;
mod trigger;

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::panic::catch_unwind;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::spawn;

use once_cell::sync::Lazy;
use parking_lot::{Mutex, MutexGuard, RwLock, RwLockUpgradableReadGuard, RwLockWriteGuard};

use crate::collector::alloc::GcAllocation;
use crate::collector::trigger::TriggerData;
use crate::lockout::{ExclusiveWarrant, Lockout, Warrant};
use crate::Scan;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InternalGcRef {
    handle_ref: Arc<GcHandle>,
}
impl InternalGcRef {
    pub(crate) fn new(handle_ref: Arc<GcHandle>) -> Self {
        Self { handle_ref }
    }
}

pub struct Collector {
    monotonic_counter: AtomicU64,
    trigger_data: Mutex<TriggerData>,
    drop_thread_chan: Mutex<Sender<Arc<GcData>>>,
    async_gc_chan: Mutex<Sender<()>>,
    tracked_data: RwLock<TrackedData>,
}

#[derive(Debug)]
struct TrackedData {
    collection_number: u64,
    data: HashSet<Arc<GcData>>,
    handles: HashSet<Arc<GcHandle>>,
}

#[derive(Debug)]
struct GcData {
    unique_id: u64,
    underlying_allocation: GcAllocation,
    lockout: Arc<Lockout>,
    deallocated: AtomicBool,
    last_marked: AtomicU64,
}

impl Hash for GcData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.unique_id.hash(state);
    }
}

impl PartialEq for GcData {
    fn eq(&self, other: &Self) -> bool {
        self.unique_id == other.unique_id
    }
}

impl Eq for GcData {}

#[derive(Debug)]
pub(crate) struct GcHandle {
    unique_id: u64,
    underlying_data: Arc<GcData>,
    lockout: Arc<Lockout>,
    last_non_rooted: AtomicU64,
}

impl Hash for GcHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.unique_id.hash(state);
    }
}

impl PartialEq for GcHandle {
    fn eq(&self, other: &Self) -> bool {
        self.unique_id == other.unique_id
    }
}

impl Eq for GcHandle {}

// TODO(issue): https://github.com/Others/shredder/issues/7

impl Collector {
    fn new() -> Arc<Self> {
        let (drop_sender, drop_receiver) = mpsc::channel::<Arc<GcData>>();

        // The drop thread deals with doing all the Drops this collector needs to do
        spawn(move || {
            // An Err value means the stream will never recover
            while let Ok(ptr) = drop_receiver.recv() {
                // Mark this data as in the process of being deallocated and unsafe to access
                // NOTE: All drops must be linearly in one thread, otherwise there could be a race around this flag being set
                ptr.deallocated.store(true, Ordering::SeqCst);

                // Deallocate / Run Drop
                let underlying_allocation = ptr.underlying_allocation;
                let res = catch_unwind(move || unsafe {
                    underlying_allocation.deallocate();
                });
                if let Err(e) = res {
                    eprintln!("Gc background drop failed: {:?}", e);
                }
            }
        });

        let (async_gc_trigger, async_gc_receiver) = mpsc::channel::<()>();

        let res = Arc::new(Self {
            monotonic_counter: AtomicU64::new(1),
            trigger_data: Mutex::default(),
            async_gc_chan: Mutex::new(async_gc_trigger),
            drop_thread_chan: Mutex::new(drop_sender),
            tracked_data: RwLock::new(TrackedData {
                collection_number: 1,
                data: HashSet::new(),
                handles: HashSet::new(),
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

    fn get_unique_id(&self) -> u64 {
        self.monotonic_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub fn track_data<T: Scan + 'static>(&self, data: T) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::allocate(data);
        let lockout = Lockout::new();

        let mut tracked_data = self.tracked_data.write();
        let new_data = Arc::new(GcData {
            unique_id: self.get_unique_id(),
            underlying_allocation: gc_data_ptr,
            lockout: lockout.clone(),
            deallocated: AtomicBool::new(false),
            last_marked: AtomicU64::new(0),
        });
        tracked_data.data.insert(new_data.clone());

        let new_handle = Arc::new(GcHandle {
            unique_id: self.get_unique_id(),
            underlying_data: new_data,
            lockout,
            last_non_rooted: AtomicU64::new(0),
        });
        tracked_data.handles.insert(new_handle.clone());
        drop(tracked_data);

        let res = (InternalGcRef::new(new_handle), heap_ptr);

        // When we allocate, the heuristic for whether we need to GC might change
        // FIXME: Use a smarter notification strategy
        self.async_gc_chan
            .lock()
            .send(())
            .expect("notifying the async gc thread should always succeed");

        res
    }

    pub fn drop_handle(&self, handle: &InternalGcRef) {
        // FIXME: Break up locks so we can take a read lock here
        let mut tracked_data = self.tracked_data.write();
        tracked_data.handles.remove(&handle.handle_ref);

        // NOTE: We probably don't want to collect here since it can happen while we are dropping from a previous collection
        // self.async_gc_chan.lock().send(());
    }

    pub fn clone_handle(&self, handle: &InternalGcRef) -> InternalGcRef {
        // FIXME: Break up locks so we can take a read lock here
        // Note: On panic, the lock is freed normally -- which is what we want
        let mut gc_data = self.tracked_data.write();

        // Technically this safety check is unnecessary, but it's pretty fast and will catch some bad behavior
        if !gc_data.handles.contains(&handle.handle_ref) {
            panic!("Tried to clone a Gc, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");
        }

        let new_handle = Arc::new(GcHandle {
            unique_id: self.get_unique_id(),
            underlying_data: handle.handle_ref.underlying_data.clone(),
            lockout: handle.handle_ref.lockout.clone(),
            last_non_rooted: AtomicU64::new(0),
        });

        gc_data.handles.insert(new_handle.clone());

        InternalGcRef {
            handle_ref: new_handle,
        }
    }

    #[allow(clippy::unused_self)]
    pub fn get_data_warrant(&self, handle: &InternalGcRef) -> Warrant {
        // Note: We do not take the lock here
        // This check is only necessary in the destructor thread, and it will always set a flag before deallocating data
        let data_deallocated = handle
            .handle_ref
            .underlying_data
            .deallocated
            .load(Ordering::SeqCst);
        if data_deallocated {
            panic!("Tried to access into a Gc, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");
        }
        handle.handle_ref.lockout.get_warrant()
    }

    pub fn tracked_data_count(&self) -> usize {
        let gc_data = self.tracked_data.read();
        gc_data.data.len()
    }

    pub fn handle_count(&self) -> usize {
        let gc_data = self.tracked_data.read();
        gc_data.handles.len()
    }

    pub fn set_gc_trigger_percent(&self, new_trigger_percent: f32) {
        self.trigger_data
            .lock()
            .set_trigger_percent(new_trigger_percent);
    }

    pub fn check_then_collect(&self) -> bool {
        let trigger = self.trigger_data.lock();
        let gc_data = self.tracked_data.upgradable_read();
        if trigger.should_collect(gc_data.data.len()) {
            self.do_collect(trigger, RwLockUpgradableReadGuard::upgrade(gc_data));
            true
        } else {
            false
        }
    }

    pub fn collect(&self) {
        let trigger_data = self.trigger_data.lock();
        let gc_data = self.tracked_data.write();
        self.do_collect(trigger_data, gc_data);
    }

    // TODO(issue): https://github.com/Others/shredder/issues/13
    // TODO: Remove the vectors we allocate here with an intrusive linked list
    // TODO: Reconsider the lockout mechanism (is the memory usage too high?)
    #[allow(clippy::shadow_unrelated)]
    fn do_collect(
        &self,
        mut trigger_data: MutexGuard<TriggerData>,
        mut tracked_data_guard: RwLockWriteGuard<TrackedData>,
    ) {
        trace!("Beginning collection");

        tracked_data_guard.collection_number += 1;
        let current_collection = tracked_data_guard.collection_number;

        // The warrant system prevents us from scanning in-use data
        let mut warrants: Vec<ExclusiveWarrant> = Vec::new();

        let tracked_data = &mut *tracked_data_guard;
        let tracked_items = &mut tracked_data.data;

        // eprintln!("tracked data {:?}", tracked_data);
        // eprintln!("tracked handles {:?}", tracked_handles);

        for data in tracked_items.iter() {
            if let Some(warrant) = data.lockout.get_exclusive_warrant() {
                // Save that warrant so things can't shift around under us
                warrants.push(warrant);

                // Now figure out what handles are not rooted
                data.underlying_allocation.scan(|h| {
                    h.handle_ref
                        .last_non_rooted
                        .store(current_collection, Ordering::SeqCst);
                });
            } else {
                // eprintln!("failed to get warrant!");
                // If we can't get the warrant, then this data must be in use, so we can mark it
                data.last_marked.store(current_collection, Ordering::SeqCst);
            }
        }

        let tracked_handles = &tracked_data.handles;
        let mut roots = Vec::new();
        for handle in tracked_handles {
            // If the `last_non_rooted` number was not now, then it is a root
            if handle.last_non_rooted.load(Ordering::SeqCst) != current_collection {
                roots.push(handle.clone());
            }
        }

        // eprintln!("roots {:?}", roots);

        let mut dfs_stack = roots;
        while let Some(handle) = dfs_stack.pop() {
            let data = &handle.underlying_data;

            // Essential note! Since all non warranted data is automatically marked, we will never accidently scan non-warranted data here
            if data.last_marked.load(Ordering::SeqCst) != current_collection {
                data.last_marked.store(current_collection, Ordering::SeqCst);

                data.underlying_allocation.scan(|h| {
                    dfs_stack.push(h.handle_ref);
                });
            }
        }

        let tracked_handles = &mut tracked_data.handles;
        let drop_thread_chan = self.drop_thread_chan.lock();
        tracked_items.retain(|data| {
            // If this is true, we just marked this data
            if data.last_marked.load(Ordering::SeqCst) == current_collection {
                // so retain it
                true
            } else {
                // Otherwise we didn't mark it and it should be deallocated

                // eprintln!("deallocating {:?}", data_ptr);

                data.underlying_allocation.scan(|h| {
                    tracked_handles.remove(&h.handle_ref);
                });

                // Send it to the drop thread to be dropped
                if let Err(e) = drop_thread_chan.send(data.clone()) {
                    error!("Error sending to drop thread {}", e);
                }

                // Note: It's okay to send all the data before we've removed it from the map
                // The destructor manages the `destructed` flag so we can never access free'd data

                // Don't retain this data
                false
            }
        });
        drop(tracked_data_guard);

        trigger_data.set_data_count_after_collection(self.tracked_data_count());

        trace!("Collection finished");
    }
}

pub static COLLECTOR: Lazy<Arc<Collector>> = Lazy::new(Collector::new);

#[cfg(test)]
pub(crate) fn get_mock_handle() -> InternalGcRef {
    use crate::{GcSafe, Scanner};

    pub(crate) struct MockAllocation;
    unsafe impl Scan for MockAllocation {
        fn scan(&self, _: &mut Scanner) {}
    }
    unsafe impl GcSafe for MockAllocation {}

    let lockout = Lockout::new();

    let mock_scannable: Box<dyn Scan> = Box::new(MockAllocation);

    // Note: Here we assume a random u64 is unique. That's hacky, but is fine for testing :)
    InternalGcRef::new(Arc::new(GcHandle {
        unique_id: rand::random(),
        underlying_data: Arc::new(GcData {
            unique_id: rand::random(),
            underlying_allocation: unsafe { GcAllocation::raw(Box::into_raw(mock_scannable)) },
            lockout: lockout.clone(),
            deallocated: AtomicBool::new(false),
            last_marked: AtomicU64::new(0),
        }),
        lockout,
        last_non_rooted: AtomicU64::new(0),
    }))
}
