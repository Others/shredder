use std::sync::atomic::Ordering;

use crossbeam::queue::SegQueue;
use dynqueue::DynQueue;
use parking_lot::MutexGuard;
use rayon::iter::{IntoParallelIterator, ParallelIterator};

use crate::collector::dropper::DropMessage;
use crate::collector::{Collector, GcExclusiveWarrant};
use crate::concurrency::lockout::Lockout;

impl Collector {
    pub(super) fn do_collect(&self, gc_guard: MutexGuard<'_, ()>) {
        // Be careful modifying this method. The tracked data and tracked handles can change underneath us
        // Currently the state is this, as far as I can tell:
        // - New handles are conservatively seen as roots if seen at all while we are touching handles
        // (there is nowhere a new "secret root" can be created and then the old root stashed and seen as non-rooted)
        // - New data is treated as a special case, and only deallocated if it existed at the start of collection
        // - Deleted handles cannot make the graph "more connected" if the deletion was not observed

        trace!("Beginning collection");
        let _atomic_spinlock_guard = self.atomic_spinlock.lock_exclusive();

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
        let warrants: SegQueue<GcExclusiveWarrant> = SegQueue::new();

        // eprintln!("tracked data {:?}", tracked_data);
        // eprintln!("tracked handles {:?}", tracked_handles);

        // In this step we calculate what's not rooted by marking all data definitively in a Gc
        self.tracked_data.data.par_iter(|data| {
            // If data.last_marked == 0, then it is new data. Update that we've seen this data
            // (this step helps synchronize what data is valid to be deallocated)
            if data.last_marked.load(Ordering::SeqCst) == 0 {
                data.last_marked
                    .store(current_collection - 1, Ordering::SeqCst);
            }

            if let Some(warrant) = Lockout::get_exclusive_warrant(data.clone()) {
                // Save that warrant so things can't shift around under us
                warrants.push(warrant);

                // Now figure out what handles are not rooted
                data.underlying_allocation.scan(|h| {
                    h.handle_ref
                        .v
                        .last_non_rooted
                        .store(current_collection, Ordering::SeqCst);
                });
            } else {
                // eprintln!("failed to get warrant!");
                // If we can't get the warrant, then this data must be in use, so we can mark it
                data.last_marked.store(current_collection, Ordering::SeqCst);
            }
        });

        // The handles that were not just marked need to be treated as roots
        let roots = SegQueue::new();
        self.tracked_data.handles.par_iter(|handle| {
            // If the `last_non_rooted` number was not now, then it is a root
            if handle.last_non_rooted.load(Ordering::SeqCst) != current_collection {
                roots.push(handle);
            }
        });

        // eprintln!("roots {:?}", roots);

        // This step is dfs through the object graph (starting with the roots)
        // We mark each object we find
        let dfs_stack = DynQueue::new(roots);
        dfs_stack
            .into_par_iter()
            .for_each(|(queue, handle)| unsafe {
                handle.underlying_data.with_data(|data| {
                    // If this data is new, we don't want to `Scan` it, since we may not have its Lockout
                    // Any handles inside this could not of been seen in step 1, so they'll be rooted anyway
                    if data.last_marked.load(Ordering::SeqCst) != 0 {
                        // Essential note! All non-new non-warranted data is automatically marked
                        // Thus we will never accidentally scan non-warranted data here
                        let previous_mark =
                            data.last_marked.swap(current_collection, Ordering::SeqCst);

                        // Since we've done an atomic swap, we know we've already scanned this iff it was marked
                        // (excluding data marked because we couldn't get its warrant, who's handles would be seen as roots)
                        // This stops us for scanning data more than once and, crucially, concurrently scanning the same data
                        if previous_mark != current_collection {
                            data.last_marked.store(current_collection, Ordering::SeqCst);

                            data.underlying_allocation.scan(|h| {
                                let mut should_enque = false;
                                h.handle_ref.v.underlying_data.with_data(|scanned_data| {
                                    if scanned_data.last_marked.load(Ordering::SeqCst)
                                        != current_collection
                                    {
                                        should_enque = true;
                                    }
                                });
                                if should_enque {
                                    queue.enqueue(h.handle_ref.v);
                                }
                            });
                        }
                    }
                })
            });
        // We're done scanning things, and have established what is marked. Release the warrants
        drop(warrants);

        // Now cleanup by removing all the data that is done for
        self.tracked_data.data.par_retain(|data| {
            // Mark the new data as in use for now
            // This stops us deallocating data that was allocated during collection
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

        // update the trigger based on the new baseline
        self.trigger
            .set_data_count_after_collection(self.tracked_data_count());

        // update collection number
        self.tracked_data
            .current_collection_number
            .fetch_add(1, Ordering::SeqCst);

        drop(gc_guard);

        trace!("Collection finished");
    }
}
