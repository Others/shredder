mod alloc;
mod dropper;
mod trigger;

use std::cmp;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::spawn;

use crossbeam::queue::SegQueue;
use crossbeam::Sender;
use dashmap::DashMap;
use dynqueue::DynQueue;
use once_cell::sync::Lazy;
use parking_lot::{Mutex, MutexGuard};
use rayon::iter::{IntoParallelIterator, ParallelBridge, ParallelIterator};

use crate::collector::alloc::GcAllocation;
use crate::collector::dropper::{BackgroundDropper, DropMessage};
use crate::collector::trigger::Trigger;
use crate::lockout::{ExclusiveWarrant, Lockout, Warrant};
use crate::{Finalize, Scan};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InternalGcRef {
    handle_ref: Arc<GcHandle>,
}
impl InternalGcRef {
    pub(crate) fn new(handle_ref: Arc<GcHandle>) -> Self {
        Self { handle_ref }
    }

    pub(crate) fn invalidate(&self) {
        COLLECTOR.drop_handle(self);
    }
}

pub struct Collector {
    monotonic_counter: AtomicU64,
    gc_lock: Mutex<()>,
    trigger: Trigger,
    dropper: BackgroundDropper,
    async_gc_notifier: Sender<()>,
    tracked_data: TrackedData,
}

#[derive(Debug)]
struct TrackedData {
    current_collection_number: AtomicU64,
    data: DashMap<Arc<GcData>, ()>,
    handles: DashMap<Arc<GcHandle>, ()>,
}

#[derive(Debug)]
pub(crate) struct GcData {
    unique_id: u64,
    underlying_allocation: GcAllocation,
    lockout: Arc<Lockout>,
    deallocated: AtomicBool,
    // During what collection was this last marked?
    //     0 if this is a new piece of data
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

impl Ord for GcData {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        Ord::cmp(&self.unique_id, &other.unique_id)
    }
}

impl PartialOrd for GcData {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        PartialOrd::partial_cmp(&self.unique_id, &other.unique_id)
    }
}

#[derive(Debug)]
pub(crate) struct GcHandle {
    unique_id: u64,
    underlying_data: Arc<GcData>,
    lockout: Arc<Lockout>,
    // During what collection was this last found in a piece of GcData?
    //     0 if this is a new piece of data
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

impl Ord for GcHandle {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        Ord::cmp(&self.unique_id, &other.unique_id)
    }
}

impl PartialOrd for GcHandle {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        PartialOrd::partial_cmp(&self.unique_id, &other.unique_id)
    }
}

// TODO(issue): https://github.com/Others/shredder/issues/7

impl Collector {
    fn new() -> Arc<Self> {
        let (async_gc_notifier, async_gc_receiver) = crossbeam::bounded(1);

        let res = Arc::new(Self {
            monotonic_counter: AtomicU64::new(1),
            gc_lock: Mutex::default(),
            trigger: Trigger::default(),
            dropper: BackgroundDropper::new(),
            async_gc_notifier,
            tracked_data: TrackedData {
                // This is janky, but we subtract one from the collection number
                // to get a previous collection number in `do_collect`
                //
                // We also use 0 as a sentinel value for newly allocated data
                //
                // Together that implies we need to start the collection number sequence at 2, not 1
                current_collection_number: AtomicU64::new(2),
                data: DashMap::new(),
                handles: DashMap::new(),
            },
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

    pub fn track_with_drop<T: Scan + 'static>(&self, data: T) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::allocate_with_drop(data);
        self.track(gc_data_ptr, heap_ptr)
    }

    pub fn track_with_no_drop<T: Scan>(&self, data: T) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::allocate_no_drop(data);
        self.track(gc_data_ptr, heap_ptr)
    }

    pub fn track_with_finalization<T: Finalize + Scan>(
        &self,
        data: T,
    ) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::allocate_with_finalization(data);
        self.track(gc_data_ptr, heap_ptr)
    }

    fn track<T: Scan>(
        &self,
        gc_data_ptr: GcAllocation,
        heap_ptr: *const T,
    ) -> (InternalGcRef, *const T) {
        let lockout = Lockout::new();

        let new_data = Arc::new(GcData {
            unique_id: self.get_unique_id(),
            underlying_allocation: gc_data_ptr,
            lockout: lockout.clone(),
            deallocated: AtomicBool::new(false),
            last_marked: AtomicU64::new(0),
        });

        let new_handle = Arc::new(GcHandle {
            unique_id: self.get_unique_id(),
            underlying_data: new_data.clone(),
            lockout,
            last_non_rooted: AtomicU64::new(0),
        });

        {
            // Insert handle before data -- don't want the data to be observable before there is a relevant handle
            // TODO: Ensure our map really promises these will appear in order
            self.tracked_data.handles.insert(new_handle.clone(), ());

            self.tracked_data.data.insert(new_data, ());
        }

        let res = (InternalGcRef::new(new_handle), heap_ptr);

        // When we allocate, the heuristic for whether we need to GC might change
        // Note: We only send if there is room in the channel
        // If there's already a notification there the async thread is already notified
        select! {
            send(self.async_gc_notifier, ()) -> res => {
                if let Err(e) = res {
                    error!("Could not notify async gc thread: {}", e);
                }
            },
            default => (),
        };

        res
    }

    pub fn drop_handle(&self, handle: &InternalGcRef) {
        self.tracked_data.handles.remove(&handle.handle_ref);

        // NOTE: We probably don't want to collect here since it can happen while we are dropping from a previous collection
        // self.async_gc_chan.lock().send(());
    }

    pub fn clone_handle(&self, handle: &InternalGcRef) -> InternalGcRef {
        let new_handle = Arc::new(GcHandle {
            unique_id: self.get_unique_id(),
            underlying_data: handle.handle_ref.underlying_data.clone(),
            lockout: handle.handle_ref.lockout.clone(),
            last_non_rooted: AtomicU64::new(0),
        });

        self.tracked_data.handles.insert(new_handle.clone(), ());

        InternalGcRef {
            handle_ref: new_handle,
        }
    }

    #[allow(clippy::unused_self)]
    pub fn get_data_warrant(&self, handle: &InternalGcRef) -> Warrant {
        // This check is only necessary in the destructors
        // The destructor thread will always set the `deallocated` flag before deallocating data
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
        self.tracked_data.data.len()
    }

    pub fn handle_count(&self) -> usize {
        self.tracked_data.handles.len()
    }

    pub fn set_gc_trigger_percent(&self, new_trigger_percent: f32) {
        self.trigger.set_trigger_percent(new_trigger_percent);
    }

    pub fn synchronize_destructors(&self) {
        let (sender, reciever) = crossbeam::bounded(1);
        let drop_msg = DropMessage::SyncUp(sender);
        {
            self.dropper
                .send_msg(drop_msg)
                .expect("drop thread should be infallible!");
        }
        reciever.recv().expect("drop thread should be infallible!");
    }

    pub fn check_then_collect(&self) -> bool {
        let gc_guard = self.gc_lock.lock();

        let current_data_count = self.tracked_data.data.len();
        if self.trigger.should_collect(current_data_count) {
            self.do_collect(gc_guard);
            true
        } else {
            false
        }
    }

    pub fn collect(&self) {
        let gc_guard = self.gc_lock.lock();
        self.do_collect(gc_guard);
    }

    // TODO(issue): https://github.com/Others/shredder/issues/13
    // TODO: Remove the vectors we allocate here with an intrusive linked list
    // TODO: Optimize memory overhead
    #[allow(clippy::shadow_unrelated)]
    fn do_collect(&self, gc_guard: MutexGuard<()>) {
        // Be careful modifying this method. The tracked data and tracked handles can change underneath us
        // Currently the state is this, as far as I can tell:
        // - New handles are conservatively seen as roots if seen at all while we are touching handles
        // (there is nowhere a new "secret root" can be created and then the old root stashed and seen as non-rooted)
        // - New data is treated as a special case, and only deallocated if it existed at the start of collection
        // - Deleted handles cannot make the graph "more connected" if the deletion was not observed

        trace!("Beginning collection");

        let current_collection = self
            .tracked_data
            .current_collection_number
            .load(Ordering::SeqCst);

        // Here we synchronize destructors: this ensures that handles in objects in the background thread are dropped
        // Otherwise we'd see those handles as rooted and keep them around.
        // This makes a lot of sense in the background thread (since it's totally async),
        // but may slow direct calls to `collect`.
        self.synchronize_destructors();

        // The warrant system prevents us from scanning in-use data
        let warrants: SegQueue<ExclusiveWarrant> = SegQueue::new();

        // eprintln!("tracked data {:?}", tracked_data);
        // eprintln!("tracked handles {:?}", tracked_handles);
        self.tracked_data.data.iter().par_bridge().for_each(|ele| {
            let data = ele.key();

            // If data.last_marked == 0, then it is new data. Update that we've seen this data
            if data.last_marked.load(Ordering::SeqCst) == 0 {
                data.last_marked
                    .store(current_collection - 1, Ordering::SeqCst);
            }

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
        });

        let mut roots = Vec::new();
        for ele in self.tracked_data.handles.iter() {
            let handle = ele.key();
            // If the `last_non_rooted` number was not now, then it is a root
            if handle.last_non_rooted.load(Ordering::SeqCst) != current_collection {
                roots.push(handle.clone());
            }
        }

        // eprintln!("roots {:?}", roots);

        let dfs_stack = DynQueue::new(roots);
        dfs_stack.into_par_iter().for_each(|(queue, handle)| {
            let data = &handle.underlying_data;

            // If this data is new, we don't want to `Scan` it, since we may not have its Lockout
            // Any handles inside this could not of been seen in step 1, so they'll be rooted anyway
            if data.last_marked.load(Ordering::SeqCst) != 0 {
                // Essential note! All non-new non-warranted data is automatically marked
                // Thus we will never accidently scan non-warranted data here
                let previous_mark = data.last_marked.swap(current_collection, Ordering::SeqCst);

                // Since we've done an automic swap, we know we've already scanned this iff it was marked
                // (excluding data marked because we couldn't get its warrant, who's handles would be seen as roots)
                // This stops us for scanning data more than once and, crucially, concurrently scanning the same data
                if previous_mark != current_collection {
                    data.last_marked.store(current_collection, Ordering::SeqCst);

                    data.underlying_allocation.scan(|h| {
                        if h.handle_ref
                            .underlying_data
                            .last_marked
                            .load(Ordering::SeqCst)
                            != current_collection
                        {
                            queue.enqueue(h.handle_ref);
                        }
                    });
                }
            }
        });
        // We're done scanning things, and have established what is marked. Release the warrants
        drop(warrants);

        // FIXME: Is par_retain  a thing that could exist?
        // Note: we cannot just do a parallel iteration with a remove inside the loop
        // Why? Deadlocks in dashmap :D
        self.tracked_data.data.retain(|data, _| {
            // Mark the new data as in use for now
            if data.last_marked.load(Ordering::SeqCst) == 0 {
                data.last_marked.store(current_collection, Ordering::SeqCst);
            }

            // If this is true, we just marked this data
            if data.last_marked.load(Ordering::SeqCst) == current_collection {
                // so retain it
                true
            } else {
                // Otherwise we didn't mark it and it should be deallocated
                // eprintln!("deallocating {:?}", data_ptr);
                // Send it to the drop thread to be dropped
                let drop_msg = DropMessage::DataToDrop(data.clone());
                if let Err(e) = self.dropper.send_msg(drop_msg) {
                    error!("Error sending to drop thread {}", e);
                }

                // Note: It's okay to send all the data before we've removed it from the map
                // The destructor manages the `destructed` flag so we can never access free'd data

                // Don't retain this data
                false
            }
        });

        self.trigger
            .set_data_count_after_collection(self.tracked_data_count());

        self.tracked_data
            .current_collection_number
            .fetch_add(1, Ordering::SeqCst);

        drop(gc_guard);

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
