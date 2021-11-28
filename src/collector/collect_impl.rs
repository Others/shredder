use crossbeam::queue::SegQueue;
use dynqueue::IntoDynQueue;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::sync::atomic::Ordering;

use crate::collector::dropper::DropMessage;
use crate::collector::Collector;
use crate::concurrency::lockout::Lockout;

use parking_lot::MutexGuard;

impl Collector {
    pub(super) fn do_collect(&self, gc_guard: MutexGuard<'_, ()>) {
        // TODO: Improve this comment
        // Be careful modifying this method. The tracked data, reference counts, and to some extent
        // the graph, can change underneath us.
        //
        // Currently the state is this, as far as I can tell:
        // - New data is always seen as rooted as long is it is allocated after the graph freezing step
        // - After graph freezing (where we take all the Lockouts we can) there is no way to
        //   smuggle items in or out of the graph
        // - The reference count preperation is conservative (if concurrently modified, the graph will simply look more connected)

        trace!("Beginning collection");
        let _atomic_spinlock_guard = self.atomic_spinlock.lock_exclusive();

        // Here we synchronize destructors: this ensures that handles in objects in the background thread are dropped
        // Otherwise we'd see those handles as rooted and keep them around. (This would not lead to incorrectness, but
        // this improves consistency and determinism.)
        //
        // This makes a lot of sense in the background thread (since it's totally async),
        // but may slow direct calls to `collect`.
        self.synchronize_destructors();

        // eprintln!("tracked data {:?}", tracked_data);
        // eprintln!("tracked handles {:?}", tracked_handles);

        // First, go through the data, resetting all the reference count trackers,
        // and taking exclusive warrants where possible
        self.tracked_data.par_iter(|data| {
            unsafe {
                // Safe as we are the collector
                Lockout::try_take_exclusive_access_unsafe(&data);
            }
            // This can be done concurrently with the `Lockout` managment, since the ref-count snapshot is conservative
            // TODO: Double check this logic
            data.ref_cnt.prepare_for_collection();
        });

        // Then adjust reference counts to figure out what is rooted
        self.tracked_data.par_iter(|data| {
            if Lockout::unsafe_exclusive_access_taken(&data) {
                data.underlying_allocation.scan(|h| {
                    h.data_ref.ref_cnt.found_once_internally();
                });
            } else {
                // Someone else had this data during the collection, so it is clearly rooted
                data.ref_cnt.override_mark_as_rooted();
            }
        });

        // Now we need to translate our set of roots into a queue
        // TODO: This is the only allocation in the collector at this point, probably is removable or re-usable
        let roots = SegQueue::new();
        self.tracked_data.par_iter(|data| {
            if data.ref_cnt.is_rooted() {
                // We need to scan data that dynamically becomes rooted, so we use the `override_mark_as_rooted`
                // flag to track what we've enqued to scan already
                data.ref_cnt.override_mark_as_rooted();
                roots.push(data);
            }
        });

        let dfs_stack = roots.into_dyn_queue();
        dfs_stack.into_par_iter().for_each(|(queue, data)| {
            debug_assert!(!data.deallocated.load(Ordering::SeqCst));

            if Lockout::unsafe_exclusive_access_taken(&data) {
                data.underlying_allocation.scan(|h| {
                    let ref_cnt = &h.data_ref.ref_cnt;
                    // We need to scan data that dynamically becomes rooted, so we use the `override_mark_as_rooted`
                    // flag to track what we've enqued to scan already. (So we can't just use `is_rooted` here.)
                    if !ref_cnt.was_overriden_as_rooted() {
                        // This is technically racy, since we check the rooting status, THEN mark as rooted/enqueue
                        // But that doesn't matter since the worse that can happen is that we enqueue the data twice
                        ref_cnt.override_mark_as_rooted();
                        queue.enqueue(h.data_ref.clone());
                    }
                });
            } else {
                // Someone else had this data during the collection, so it is clearly rooted
                data.ref_cnt.override_mark_as_rooted();
            }
        });

        // We are done scanning, so release any warrants
        self.tracked_data.par_iter(|data| unsafe {
            Lockout::try_release_exclusive_access_unsafe(&data);
        });

        // Since new refcnts are created as rooted, and new data is created with new refcnts, we
        // can safely treat the refcnt data as definitive

        // Now cleanup by removing all the data that is done for
        let to_drop = self.dropper.get_buffer();

        self.tracked_data.par_retain(|data| {
            let is_marked = data.ref_cnt.is_rooted();
            if is_marked {
                // this is marked so retain it
                return true;
            }

            // Otherwise we didn't mark it and it should be deallocated
            // eprintln!("deallocating {:?}", data_ptr);
            // Send it to the drop thread to be dropped
            to_drop.push(data.clone());

            // Don't retain this data
            false
        });

        // Send off the data to be dropped in the background
        let drop_msg = DropMessage::DataToDrop(to_drop);
        if let Err(e) = self.dropper.send_msg(drop_msg) {
            error!("Error sending to drop thread {}", e);
        }

        // update the trigger based on the new baseline
        self.trigger
            .set_data_count_after_collection(self.tracked_data_count());

        drop(gc_guard);

        trace!("Collection finished");
    }
}
