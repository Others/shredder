/// A trait implementing an alternative to `Drop`, useful for non-`GcDrop` data.
///
/// Usually when you have data in a `Gc` you just want its destructor to be called when the data is
/// collected. However, the collector can't naively run the `drop` method of non-`GcDrop`
/// /non-`'static` data, since it could access data with an elapsed lifetime. (It's even UB to
/// create a reference into a struct holding an invalid reference!) We address this in two parts.
/// The `R` and `RMut` structs provide a safe alternative to holding a direct reference with a
/// non-'static lifetime. Then the `Finalize` trait let's you opt-in to writing unsafe code at
/// deallocation time.
///
/// Note: Some data is `!GcDrop` even though it is `'static`, like `AtomicGc` or `DerefGc`. (Or
/// anything that contains a `AtomicGc` or `DerefGc`.) In those cases you will need to use
/// `Finalize` to write destructors, and promise not touch fields of those types.
///
/// You probably want to use `#[derive(Finalize)]` to implement this :)
///
/// # Safety
/// When implementing this trait you're promising a few things:
///
/// 1) Your data does not contain any non-`'static` references.
/// (You may use `R` and `RMut` instead!)
///
/// 2) Your `finalize` method does not access any data with a non-`'static` lifetime. In particular
/// you may not call any methods on `R` or `RMut` other than `finalize`. (No `Deref` either!)
///
/// 3) Your `finalize` method does not call any methods on a `AtomicGc` or `DerefGc`.
/// (No `Deref` either!)
///
/// 4) Your `finalize` method does not make an `AtomicGc`, `DerefGc`, `R` or `RMut` "live again."
/// Basically you must not send one of these pieces of data to another thread.
pub unsafe trait Finalize {
    /// Do cleanup on this data, potentially leaving it in an invalid state.
    /// (See trait documentation for the rules for implementing this method.)
    ///
    /// Please ensure your `finalize` implementations delegate properly and call your fields
    /// `finalize` methods after doing cleanup.
    ///
    /// # Safety
    /// After calling this method, no further operations may be performed with this object. You
    /// may not even drop this object! You must `mem::forget` it or otherwise force its destructor
    /// not to run.
    unsafe fn finalize(&mut self);
}

/// A trait that lets you finalize all fields of a piece of data
///
/// This is useful for implementing `Finalize` itself, since it gives you a simple way to
/// recursively finalize the fields.
///
/// You probably want to use `#[derive(FinalizeFields)]` to implement this :)
///
/// # Safety
/// Implementing this has the same rules as implementing `Finalize`
pub unsafe trait FinalizeFields {
    /// Do cleanup on this data's fields, potentially leaving it in an invalid state.
    ///
    /// # Safety
    /// After calling this method, you may not access anything contained in this data. You may not
    /// even drop this object! You must `mem::forget` it or otherwise force its destructor not to run.
    unsafe fn finalize_fields(&mut self);
}
