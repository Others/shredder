mod alloc;
mod collect_impl;
mod data;
mod dropper;
mod ref_cnt;
mod trigger;

use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::spawn;

use crossbeam::channel::{self, Sender};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::collector::alloc::GcAllocation;
use crate::collector::dropper::{BackgroundDropper, DropMessage};
use crate::collector::trigger::GcTrigger;
use crate::concurrency::atomic_protection::{APSInclusiveGuard, AtomicProtectingSpinlock};
use crate::concurrency::chunked_ll::ChunkedLinkedList;
use crate::concurrency::lockout::{Lockout, Warrant};
use crate::marker::GcDrop;
use crate::{Finalize, Scan, ToScan};

pub use crate::collector::data::GcData;
use crate::collector::ref_cnt::GcRefCount;

/// Intermediate struct. `Gc<T>` holds a `InternalGcRef`, which owns incrementing and decrementing
/// the reference count of the stored data.
#[derive(Debug)]
pub struct InternalGcRef {
    data_ref: Arc<GcData>,
    invalidated: AtomicBool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum RefCountPolicy {
    // A transient handle doesn't increment or decrement the reference count
    TransientHandle,
    // On initial creation, we assume the reference count is exactly one, and only increment the handle count
    // Then on deallocation, we decrement both the data and handle count
    InitialCreation,
    // If a handle already exists, we manage both the data reference and handle counts
    FromExistingHandle,
    // The reference counts are being inherited from another source (used mostly by `AtomicGc`)
    InheritExistingCounts,
}

impl InternalGcRef {
    #[inline]
    #[allow(clippy::match_same_arms)]
    pub(crate) fn new(data_ref: Arc<GcData>, ref_cnt_policy: RefCountPolicy) -> Self {
        match &ref_cnt_policy {
            RefCountPolicy::TransientHandle => {
                // No action: this is a transient handle
            }
            RefCountPolicy::InheritExistingCounts => {
                // No action: we are inheriting the reference counts from another source
            }
            RefCountPolicy::InitialCreation => {
                debug_assert_eq!(data_ref.ref_cnt.snapshot_ref_count(), 1);
                // Increment handle count only
                COLLECTOR.increment_handle_count();
            }
            RefCountPolicy::FromExistingHandle => {
                COLLECTOR.increment_reference_count(&data_ref);
            }
        }

        let pre_invalidated = matches!(ref_cnt_policy, RefCountPolicy::TransientHandle);

        Self {
            data_ref,
            invalidated: AtomicBool::new(pre_invalidated),
        }
    }

    #[inline]
    pub(crate) fn invalidate(&self) {
        COLLECTOR.drop_handle(self);
    }

    #[inline]
    pub(crate) fn is_invalidated(&self) -> bool {
        self.invalidated.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn invalidate_without_touching_reference_counts(&self) {
        self.invalidated.store(true, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn data(&self) -> &Arc<GcData> {
        &self.data_ref
    }
}

/// We don't want to expose what specific warrant provider we're using
/// (this struct should be optimized away)
pub struct GcGuardWarrant {
    /// stores the internal warrant. only the drop being run is relevant
    _warrant: Warrant<Arc<GcData>>,
}

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
    /// a set storing metadata on the live data the collector is managing
    tracked_data: ChunkedLinkedList<GcData>,
    /// a count of how many handles are live
    live_handle_count: AtomicUsize,
}

// TODO(issue): https://github.com/Others/shredder/issues/7

impl Collector {
    fn new() -> Arc<Self> {
        let (async_gc_notifier, async_gc_receiver) = channel::bounded(1);

        let res = Arc::new(Self {
            gc_lock: Mutex::default(),
            atomic_spinlock: AtomicProtectingSpinlock::default(),
            trigger: GcTrigger::default(),
            dropper: BackgroundDropper::new(),
            async_gc_notifier,
            tracked_data: ChunkedLinkedList::new(),
            live_handle_count: AtomicUsize::new(0),
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

    pub fn track_with_drop<T: Scan + GcDrop>(&self, data: T) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::allocate_with_drop(data);
        (self.track(gc_data_ptr), heap_ptr)
    }

    pub fn track_with_no_drop<T: Scan>(&self, data: T) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::allocate_no_drop(data);
        (self.track(gc_data_ptr), heap_ptr)
    }

    pub fn track_with_finalization<T: Finalize + Scan>(
        &self,
        data: T,
    ) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::allocate_with_finalization(data);
        (self.track(gc_data_ptr), heap_ptr)
    }

    pub fn track_boxed_value<T: Scan + ToScan + GcDrop + ?Sized>(
        &self,
        data: Box<T>,
    ) -> (InternalGcRef, *const T) {
        let (gc_data_ptr, heap_ptr) = GcAllocation::from_box(data);
        (self.track(gc_data_ptr), heap_ptr)
    }

    pub unsafe fn track_with_initializer<T, F>(&self, init_function: F) -> (InternalGcRef, *const T)
    where
        T: Scan + GcDrop,
        F: FnOnce(InternalGcRef, *const T) -> T,
    {
        let (gc_allocation, uninit_ptr) = GcAllocation::allocate_uninitialized_with_drop();
        self.initialize_and_track(init_function, gc_allocation, uninit_ptr)
    }

    pub unsafe fn track_with_initializer_and_finalize<T, F>(
        &self,
        init_function: F,
    ) -> (InternalGcRef, *const T)
    where
        T: Finalize + Scan,
        F: FnOnce(InternalGcRef, *const T) -> T,
    {
        let (gc_allocation, uninit_ptr) = GcAllocation::allocate_uninitialized_with_finalization();
        self.initialize_and_track(init_function, gc_allocation, uninit_ptr)
    }

    unsafe fn initialize_and_track<T, F>(
        &self,
        init_function: F,
        gc_allocation: GcAllocation,
        uninit_ptr: *const T,
    ) -> (InternalGcRef, *const T)
    where
        T: Scan,
        F: FnOnce(InternalGcRef, *const T) -> T,
    {
        let gc_data = Arc::new(GcData {
            underlying_allocation: gc_allocation,
            lockout: Lockout::new(),
            deallocated: AtomicBool::new(false),
            // Must start count at 1 to avoid a race condition between inserting the data and creating the handle
            ref_cnt: GcRefCount::new(1),
        });

        // Take a warrant to prevent the collector from accessing the data while we're initializing it
        let warrant = Lockout::try_take_exclusive_warrant(gc_data.clone())
            .expect("lockout just created, so should be avaliable for locking");

        // Setup our data structures to handle this data
        let gc_handle = self.tracked_data.insert(gc_data);
        let reference = InternalGcRef::new(gc_handle.v, RefCountPolicy::InitialCreation);

        // Initialize
        let t = init_function(self.clone_handle(&reference), uninit_ptr);
        ptr::write(uninit_ptr as *mut T, t);

        // We've written the data, so drop the warrant
        drop(warrant);

        // When we allocate, the heuristic for whether we need to GC might change
        self.notify_async_gc_thread();

        (reference, uninit_ptr)
    }

    fn track(&self, gc_data_ptr: GcAllocation) -> InternalGcRef {
        let item = self.tracked_data.insert(Arc::new(GcData {
            underlying_allocation: gc_data_ptr,
            lockout: Lockout::new(),
            deallocated: AtomicBool::new(false),
            // Start the reference count at 1 to avoid a race condition between insertion and handle creation
            ref_cnt: GcRefCount::new(1),
        }));
        let res = InternalGcRef::new(item.v, RefCountPolicy::InitialCreation);

        // When we allocate, the heuristic for whether we need to GC might change
        self.notify_async_gc_thread();

        res
    }

    pub fn drop_handle(&self, handle: &InternalGcRef) {
        let was_invalidated = handle.invalidated.swap(true, Ordering::Relaxed);
        if !was_invalidated {
            self.decrement_reference_count(&handle.data_ref);
        }

        // NOTE: This is worth experimenting with
        // self.notify_async_gc_thread();
    }

    #[allow(clippy::unused_self)]
    pub fn clone_handle(&self, handle: &InternalGcRef) -> InternalGcRef {
        InternalGcRef::new(handle.data_ref.clone(), RefCountPolicy::FromExistingHandle)
    }

    pub fn increment_handle_count(&self) {
        self.live_handle_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_reference_count(&self, data: &GcData) {
        data.ref_cnt.inc_count();
        self.live_handle_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_reference_count(&self, data: &GcData) {
        data.ref_cnt.dec_count();
        // NOTE: This will wrap around on overflow
        self.live_handle_count.fetch_sub(1, Ordering::Relaxed);
    }

    // TODO: Fix the abstraction layer between `InternalGcRef` and `Collector`
    #[allow(clippy::unused_self)]
    pub fn get_data_warrant(&self, handle: &InternalGcRef) -> GcGuardWarrant {
        let data_deallocated = handle.data_ref.deallocated.load(Ordering::SeqCst);

        if data_deallocated {
            panic!("Tried to access into a Gc, but the internal state was corrupted (perhaps you're manipulating Gc<?> in a destructor?)");
        }

        GcGuardWarrant {
            _warrant: Lockout::take_warrant(handle.data_ref.clone()),
        }
    }

    pub fn tracked_data_count(&self) -> usize {
        self.tracked_data.estimate_len()
    }

    pub fn handle_count(&self) -> usize {
        self.live_handle_count.load(Ordering::Relaxed)
    }

    pub fn set_gc_trigger_percent(&self, new_trigger_percent: f32) {
        self.trigger.set_trigger_percent(new_trigger_percent);
    }

    pub fn synchronize_destructors(&self) {
        // We send a channel to the drop thread and wait for it to respond
        // This has the effect of synchronizing this thread with the drop thread

        let (sender, receiver) = channel::bounded(1);
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

        let current_data_count = self.tracked_data.estimate_len();
        let current_handle_count = self.live_handle_count.load(Ordering::Relaxed);
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
    use crate::marker::GcSafe;
    use crate::Scanner;

    pub(crate) struct MockAllocation;
    unsafe impl Scan for MockAllocation {
        fn scan(&self, _: &mut Scanner<'_>) {}
    }
    unsafe impl GcSafe for MockAllocation {}

    let mock_scannable: Box<dyn Scan> = Box::new(MockAllocation);

    let data_arc = Arc::new(GcData {
        underlying_allocation: unsafe { GcAllocation::raw(&*mock_scannable) },
        lockout: Lockout::new(),
        deallocated: AtomicBool::new(false),
        ref_cnt: GcRefCount::new(1),
    });

    InternalGcRef::new(data_arc, RefCountPolicy::InitialCreation)
}
