mod alloc;
mod collect_impl;
mod data;
mod dropper;
mod trigger;

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::spawn;

use crossbeam::Sender;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::collector::alloc::GcAllocation;
use crate::collector::dropper::{BackgroundDropper, DropMessage};
use crate::collector::trigger::GcTrigger;
use crate::concurrency::atomic_protection::{APSInclusiveGuard, AtomicProtectingSpinlock};
use crate::concurrency::lockout::{ExclusiveWarrant, Lockout, Warrant};
use crate::{Finalize, Scan};

pub use crate::collector::data::{GcData, GcHandle, UnderlyingData};
use crate::concurrency::chunked_ll::{CLLItem, ChunkedLinkedList};

/// Intermediate struct. `Gc<T>` holds a `InternalGcRef`, which references a `GcHandle`
/// There should be one `GcHandle` per `Gc<T>`
#[derive(Clone, Debug)]
pub struct InternalGcRef {
    handle_ref: CLLItem<GcHandle>,
}

impl InternalGcRef {
    pub(crate) fn new(handle_ref: CLLItem<GcHandle>) -> Self {
        Self { handle_ref }
    }

    pub(crate) fn invalidate(&self) {
        COLLECTOR.drop_handle(self);
    }

    pub(crate) fn data(&self) -> Arc<GcData> {
        if let UnderlyingData::Fixed(data) = &self.handle_ref.v.underlying_data {
            data.clone()
        } else {
            panic!("Only fixed data has a usable `data` method")
        }
    }
}

/// We don't want to expose what specific warrant provider we're using
/// (this struct should be optimized away)
pub struct GcGuardWarrant {
    /// stores the internal warrant. only the drop being run is relevant
    _warrant: Warrant<Arc<GcData>>,
}
type GcExclusiveWarrant = ExclusiveWarrant<Arc<GcData>>;

pub struct Collector {
    /// shredder only allows one collection to proceed at a time
    gc_lock: Mutex<()>,
    /// this prevents atomic operations from happening during collection time
    atomic_spinlock: AtomicProtectingSpinlock,
    /// trigger decides when we should run a collection
    trigger: GcTrigger,
    /// dropping happens in a background thread. This struct lets us communicate with that thread
    dropper: BackgroundDropper,
    /// we run automatic gc in a background thread
    /// sending to this channel indicates that thread should check the trigger, then collect if the
    /// trigger indicates it should
    async_gc_notifier: Sender<()>,
    /// all the data we are managing plus metadata about what `Gc<T>`s exist
    tracked_data: TrackedData,
}

/// Stores metadata about each piece of tracked data, plus metadata about each handle
#[derive(Debug)]
struct TrackedData {
    /// we increment this whenever we collect
    current_collection_number: AtomicU64,
    /// a set storing metadata on the live data the collector is managing
    data: ChunkedLinkedList<GcData>,
    /// a set storing metadata on each live handle (`Gc<T>`) the collector is managing
    handles: ChunkedLinkedList<GcHandle>,
}

// TODO(issue): https://github.com/Others/shredder/issues/7

impl Collector {
    fn new() -> Arc<Self> {
        let (async_gc_notifier, async_gc_receiver) = crossbeam::bounded(1);

        let res = Arc::new(Self {
            gc_lock: Mutex::default(),
            atomic_spinlock: AtomicProtectingSpinlock::default(),
            trigger: GcTrigger::default(),
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
                data: ChunkedLinkedList::new(),
                handles: ChunkedLinkedList::new(),
            },
        });

        // The async Gc thread deals with background Gc'ing
        let async_collector_ref = Arc::downgrade(&res);
        spawn(move || {
            // An Err value means the stream will never recover
            while async_gc_receiver.recv().is_ok() {
                if let Some(collector) = async_collector_ref.upgrade() {
                    collector.check_then_collect();
                }
            }
        });

        res
    }

    #[inline]
    fn notify_async_gc_thread(&self) {
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
        let new_data_arc = Arc::new(GcData {
            underlying_allocation: gc_data_ptr,
            lockout: Lockout::new(),
            deallocated: AtomicBool::new(false),
            last_marked: AtomicU64::new(0),
        });

        let new_handle_arc = Arc::new(GcHandle {
            underlying_data: UnderlyingData::Fixed(new_data_arc.clone()),
            last_non_rooted: AtomicU64::new(0),
        });

        // Insert handle before data -- don't want the data to be observable before there is a relevant handle
        let new_handle = self.tracked_data.handles.insert(new_handle_arc);

        self.tracked_data.data.insert(new_data_arc);

        let res = (InternalGcRef::new(new_handle), heap_ptr);

        // When we allocate, the heuristic for whether we need to GC might change
        self.notify_async_gc_thread();

        res
    }

    pub fn drop_handle(&self, handle: &InternalGcRef) {
        self.tracked_data.handles.remove(&handle.handle_ref);

        // NOTE: This is worth experimenting with
        // self.notify_async_gc_thread();
    }

    pub fn clone_handle(&self, handle: &InternalGcRef) -> InternalGcRef {
        let new_handle_arc = Arc::new(GcHandle {
            underlying_data: UnderlyingData::Fixed(handle.data()),
            last_non_rooted: AtomicU64::new(0),
        });

        let new_handle = self.tracked_data.handles.insert(new_handle_arc);

        InternalGcRef {
            handle_ref: new_handle,
        }
    }

    pub fn handle_from_data(&self, underlying_data: Arc<GcData>) -> InternalGcRef {
        let new_handle_arc = Arc::new(GcHandle {
            underlying_data: UnderlyingData::Fixed(underlying_data),
            last_non_rooted: AtomicU64::new(0),
        });

        let new_handle = self.tracked_data.handles.insert(new_handle_arc);

        InternalGcRef {
            handle_ref: new_handle,
        }
    }

    pub fn new_handle_for_atomic(&self, atomic_ptr: Arc<AtomicPtr<GcData>>) -> InternalGcRef {
        let new_handle_arc = Arc::new(GcHandle {
            underlying_data: UnderlyingData::DynamicForAtomic(atomic_ptr),
            last_non_rooted: AtomicU64::new(0),
        });

        let new_handle = self.tracked_data.handles.insert(new_handle_arc);

        InternalGcRef {
            handle_ref: new_handle,
        }
    }

    #[allow(clippy::unused_self)]
    pub fn get_data_warrant(&self, handle: &InternalGcRef) -> GcGuardWarrant {
        // This check is only necessary in the destructors
        // The destructor thread will always set the `deallocated` flag before deallocating data
        if let UnderlyingData::Fixed(fixed) = &handle.handle_ref.v.underlying_data {
            let data_deallocated = fixed.deallocated.load(Ordering::SeqCst);

            if data_deallocated {
                panic!("Tried to access into a Gc, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");
            }

            GcGuardWarrant {
                _warrant: Lockout::get_warrant(fixed.clone()),
            }
        } else {
            panic!("Cannot get data warrant for atomic data!")
        }
    }

    pub fn tracked_data_count(&self) -> usize {
        self.tracked_data.data.estimate_len()
    }

    pub fn handle_count(&self) -> usize {
        self.tracked_data.handles.estimate_len()
    }

    pub fn set_gc_trigger_percent(&self, new_trigger_percent: f32) {
        self.trigger.set_trigger_percent(new_trigger_percent);
    }

    pub fn synchronize_destructors(&self) {
        // We send a channel to the drop thread and wait for it to respond
        // This has the effect of synchronizing this thread with the drop thread

        let (sender, receiver) = crossbeam::bounded(1);
        let drop_msg = DropMessage::SyncUp(sender);
        {
            self.dropper
                .send_msg(drop_msg)
                .expect("drop thread should be infallible!");
        }
        receiver.recv().expect("drop thread should be infallible!");
    }

    #[inline]
    pub fn get_collection_blocker_spinlock(&self) -> APSInclusiveGuard<'_> {
        loop {
            if let Some(inclusive_guard) = self.atomic_spinlock.lock_inclusive() {
                return inclusive_guard;
            }
            // block on the collector if we can't get the APS guard
            let collector_block = self.gc_lock.lock();
            drop(collector_block);
        }
    }

    pub fn check_then_collect(&self) -> bool {
        let gc_guard = self.gc_lock.lock();

        let current_data_count = self.tracked_data.data.estimate_len();
        let current_handle_count = self.tracked_data.handles.estimate_len();
        if self
            .trigger
            .should_collect(current_data_count, current_handle_count)
        {
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
}

pub static COLLECTOR: Lazy<Arc<Collector>> = Lazy::new(Collector::new);

#[cfg(test)]
pub(crate) fn get_mock_handle() -> InternalGcRef {
    use crate::{GcSafe, Scanner};

    pub(crate) struct MockAllocation;
    unsafe impl Scan for MockAllocation {
        fn scan(&self, _: &mut Scanner<'_>) {}
    }
    unsafe impl GcSafe for MockAllocation {}

    let mock_scannable: Box<dyn Scan> = Box::new(MockAllocation);

    // This leaks some memory...
    let mock_master_list = ChunkedLinkedList::new();

    // Note: Here we assume a random u64 is unique. That's hacky, but is fine for testing :)
    let handle_arc = Arc::new(GcHandle {
        underlying_data: UnderlyingData::Fixed(Arc::new(GcData {
            underlying_allocation: unsafe { GcAllocation::raw(Box::into_raw(mock_scannable)) },
            lockout: Lockout::new(),
            deallocated: AtomicBool::new(false),
            last_marked: AtomicU64::new(0),
        })),
        last_non_rooted: AtomicU64::new(0),
    });

    InternalGcRef::new(mock_master_list.insert(handle_arc))
}
