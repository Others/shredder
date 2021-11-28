use std::panic::catch_unwind;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::spawn;

use crossbeam::channel::{self, Receiver, SendError, Sender};

use crate::collector::GcData;
use crate::concurrency::cross_thread_buffer::CrossThreadBuffer;

type DropBuffer = CrossThreadBuffer<Arc<GcData>>;

pub(crate) struct BackgroundDropper {
    // TODO: This would probably be marginally more efficient with non-channel based synchronization
    drop_message_sender: Sender<DropMessage>,
    buffer_recycler: Receiver<DropBuffer>,
}

pub(crate) enum DropMessage {
    /// Signals the `BackgroundDropper` to deallocate the following data (possibly running some destructor)
    DataToDrop(DropBuffer),
    /// Indicates to the `BackgroundDropper` that it should sync up with the calling code
    SyncUp(Sender<()>),
}

impl BackgroundDropper {
    const RECYCLING_CHANNEL_SIZE: usize = 1;

    pub fn new() -> Self {
        let (drop_message_sender, drop_message_retriever) = channel::unbounded();
        let (recycling_sender, recycling_receiver) = channel::bounded(Self::RECYCLING_CHANNEL_SIZE);

        // The drop thread deals with doing all the Drops this collector needs to do
        spawn(move || {
            // An Err value means the stream will never recover
            while let Ok(drop_msg) = drop_message_retriever.recv() {
                match drop_msg {
                    DropMessage::DataToDrop(mut to_drop) => {
                        // NOTE: It's important that all data is correctly marked as deallocated before we start
                        to_drop.par_for_each(|data| {
                            // Mark this data as in the process of being deallocated and unsafe to access
                            data.deallocated.store(true, Ordering::SeqCst);
                        });

                        // Then run the drops if needed
                        to_drop.par_for_each(|data| {
                            let underlying_allocation = data.underlying_allocation;
                            let res = catch_unwind(move || unsafe {
                                underlying_allocation.deallocate();
                            });
                            if let Err(e) = res {
                                eprintln!("Gc background drop failed: {:?}", e);
                            }
                        });

                        // Then clear and recycle the buffer
                        to_drop.clear();
                        // ignore recycling failures
                        let recycling_error = recycling_sender.try_send(to_drop);
                        if let Err(e) = recycling_error {
                            error!("Error recycling drop buffer {:?}", e);
                        }
                    }
                    DropMessage::SyncUp(responder) => {
                        if let Err(e) = responder.send(()) {
                            error!("Gc background syncup failed: {:?}", e);
                        }
                    }
                }
            }
        });

        Self {
            drop_message_sender,
            buffer_recycler: recycling_receiver,
        }
    }

    pub fn send_msg(&self, msg: DropMessage) -> Result<(), SendError<DropMessage>> {
        self.drop_message_sender.send(msg)
    }

    pub fn get_buffer(&self) -> DropBuffer {
        self.buffer_recycler.try_recv().unwrap_or_default()
    }
}

impl Default for BackgroundDropper {
    fn default() -> Self {
        Self::new()
    }
}
