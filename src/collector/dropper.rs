use std::prelude::v1::*;

#[cfg(feature = "std")]
use std::panic::catch_unwind;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam::channel::{self, SendError, Sender};
use parking_lot::RwLock;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;

use crate::collector::GcData;

pub(crate) struct BackgroundDropper {
    sender: Sender<DropMessage>,
}

pub(crate) enum DropMessage {
    /// Signals the `BackgroundDropper` to deallocate the following data (possibly running some destructor)
    DataToDrop(RwLock<Vec<Arc<GcData>>>),
    /// Indicates to the `BackgroundDropper` that it should sync up with the calling code
    SyncUp(Sender<()>),
}

impl BackgroundDropper {
    pub fn new<F>(spawn: &'static F) -> Self
    where
        F: Fn(Box<dyn Fn() + Send>),
        F: Sized,
    {
        let (sender, receiver) = channel::unbounded();

        // The drop thread deals with doing all the Drops this collector needs to do
        spawn(Box::new(move || {
            // An Err value means the stream will never recover
            while let Ok(drop_msg) = receiver.recv() {
                match drop_msg {
                    DropMessage::DataToDrop(to_drop) => {
                        let to_drop = to_drop.read();

                        // NOTE: It's important that all data is correctly marked as deallocated before we start
                        to_drop.par_iter().for_each(|data| {
                            // Mark this data as in the process of being deallocated and unsafe to access
                            data.deallocated.store(true, Ordering::SeqCst);
                        });

                        // Then run the drops if needed
                        to_drop.par_iter().for_each(|data| {
                            let underlying_allocation = data.underlying_allocation;

                            // When the stdlib is available, we can use catch_unwind
                            // to protect ourselves against panics that unwind.
                            #[cfg(feature = "std")]
                            {
                                let res = catch_unwind(move || unsafe {
                                    underlying_allocation.deallocate();
                                });
                                if let Err(e) = res {
                                    eprintln!("Gc background drop failed: {:?}", e);
                                }
                            }

                            // When it is not available, however, panics probably
                            // won't unwind, and there's no safe means to catch
                            // a panic.
                            //
                            // TODO is there a better way to safely handle this?
                            #[cfg(not(feature = "std"))]
                            underlying_allocation.deallocate();
                        });
                    }
                    DropMessage::SyncUp(responder) => {
                        if let Err(e) = responder.send(()) {
                            eprintln!("Gc background syncup failed: {:?}", e);
                        }
                    }
                }
            }
        }));

        Self { sender }
    }

    pub fn send_msg(&self, msg: DropMessage) -> Result<(), SendError<DropMessage>> {
        self.sender.send(msg)
    }
}
