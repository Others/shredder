//! # Shredder
//!
//! `shredder` is a library providing a garbage collected smart pointer: `Gc`.
//! This is useful for times where you want shared access to some data, but the structure
//! of the data has unpredictable cycles in it. (So Arc would not be appropriate.)
//!
//! `shredder` has the following features:
//! - fairly ergonomic: no need to manually manage roots, just a regular smart pointer
//! - destructors: no need for finalization, your destructors are seamlessly run
//! - ready for fearless concurrency: works in multi-threaded contexts
//! - safe: detects error conditions on the fly, and protects you from undefined behavior
//! - limited stop-the world: regular processing will rarely be interrupted
//! - concurrent collection: collection and destruction happens in the background
//!
//!
//! `shredder` has the following limitations:
//! - non-sync ergonomics: `Send` objects need a guard object
//! - multiple collectors: multiple collectors do not co-operate
//! - can't handle `Rc`/`Arc`: requires all `Gc` objects have straightforward ownership semantics
//! - non static data: `Gc` cannot handle non 'static data (fix WIP)

// We love docs here
// #![deny(missing_docs)]
// Clippy configuration:
// I'd like the most pedantic warning level
#![warn(
    clippy::cargo,
    clippy::needless_borrow,
    clippy::pedantic,
    clippy::redundant_clone
)]
// But I don't care about these ones
#![allow(
    clippy::cast_precision_loss,     // There is no way to avoid this precision loss
    clippy::module_name_repetitions, // Sometimes clear naming calls for repetition
    clippy::multiple_crate_versions  // There is no way to easily fix this without modifying our dependencies
)]

#[macro_use]
extern crate log;

#[macro_use]
extern crate rental;

mod collector;
mod lockout;
mod scan;
mod smart_ptr;

use std::cell::RefCell;
use std::sync::Mutex;

use collector::COLLECTOR;

pub use scan::{GcSafe, GcSafeWrapper, Scan, Scanner};
pub use smart_ptr::{Gc, GcGuard, GcRef, GcRefMut};

// Re-export the Scan derive
pub use shredder_derive::Scan;

pub type GRef<T> = Gc<RefCell<T>>;
pub type GMutex<T> = Gc<Mutex<T>>;

/// Returns how many underlying allocations are currently allocated
///
/// # Example
/// ```
/// use shredder::{number_of_tracked_allocations, Gc};
///
/// let data = Gc::new(128);
/// assert!(number_of_tracked_allocations() > 0);
/// ```
#[must_use]
pub fn number_of_tracked_allocations() -> usize {
    COLLECTOR.tracked_data_count()
}

/// Returns how many `Gc`s are currently in use
///
/// # Example
/// ```
/// use shredder::{number_of_active_handles, Gc};
///
/// let data = Gc::new(128);
/// assert!(number_of_active_handles() > 0);
/// ```
#[must_use]
pub fn number_of_active_handles() -> usize {
    COLLECTOR.handle_count()
}

/// `shredders` collection automatically triggers when:
/// ```text
///     allocations > allocations_after_last_collection * (1 + gc_trigger_percent)
/// ```
/// The default value of `gc_trigger_percent` is 0.75, but `set_gc_trigger_percent` lets you configure
/// it yourself. Only values 0 or greater are allowed (NaNs and negative values will cause a panic)
///
/// # Example
/// ```
/// use shredder::set_gc_trigger_percent;
/// set_gc_trigger_percent(0.75); // GC will trigger after data exceeds 1.75x previous heap size
/// ```
pub fn set_gc_trigger_percent(percent: f32) {
    if percent < -0.0 || percent.is_nan() {
        panic!(
            "The trigger percentage cannot be less than zero or NaN! (percent = {})",
            percent
        )
    }
    COLLECTOR.set_gc_trigger_percent(percent)
}

/// `collect` allows you to manually run a collection, ignoring the heuristic that governs normal
/// garbage collector operations. This can be an extremely slow operation, since the algorithm is
/// designed to be run in the background, while this method runs it on the thread that calls the
/// method. Additionally, you may end up blocking waiting to collect, since `shredder` doesn't allow
/// two collections at once
///
/// # Example
/// ```
/// use shredder::collect;
/// collect(); // Manually run GC
/// ```
#[allow(clippy::must_use_candidate)]
pub fn collect() {
    COLLECTOR.collect();
}
