use std::panic::catch_unwind;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::spawn;

use crossbeam::{SendError, Sender};

use crate::collector::GcData;

pub(crate) struct BackgroundDropper {
    sender: Sender<DropMessage>,
}

pub(crate) enum DropMessage {
    /// Signals the `BackgroundDropper` to deallocate the following data (possibly running some destructor)
    DataToDrop(Arc<GcData>),
    /// Indicates to the `BackgroundDropper` that it should sync up with the calling code
    SyncUp(Sender<()>),
}

impl BackgroundDropper {
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam::unbounded();

        // The drop thread deals with doing all the Drops this collector needs to do
        spawn(move || {
            // An Err value means the stream will never recover
            while let Ok(drop_msg) = receiver.recv() {
                match drop_msg {
                    DropMessage::DataToDrop(data) => {
                        // Mark this data as in the process of being deallocated and unsafe to access
                        // NOTE: All drops must be linearly in one thread, otherwise there could be a race around this flag being set
                        data.deallocated.store(true, Ordering::SeqCst);

                        // Deallocate / Run Drop
                        let underlying_allocation = data.underlying_allocation;
                        let res = catch_unwind(move || unsafe {
                            underlying_allocation.deallocate();
                        });
                        if let Err(e) = res {
                            eprintln!("Gc background drop failed: {:?}", e);
                        }
                    }
                    DropMessage::SyncUp(responder) => {
                        if let Err(e) = responder.send(()) {
                            eprintln!("Gc background syncup failed: {:?}", e);
                        }
                    }
                }
            }
        });

        Self { sender }
    }

    pub fn send_msg(&self, msg: DropMessage) -> Result<(), SendError<DropMessage>> {
        self.sender.send(msg)
    }
}
