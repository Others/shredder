use std::prelude::v1::*;

/// A marker trait that marks that this data can be stored in a `DerefGc`
///
/// `T` can be `GcDeref` only if it is deeply immutable through a `&T`. This is because it's
/// problematic to move `Gc`s around from inside other `Gc`s without a `GcGuard`.
///
/// This would seem to make `DerefGc`/`GcDeref` very hard to use. Luckily there is one exception! A
/// `GcDeref` can contain a `Gc`, even if that `Gc` contains data that is not deeply immutable
/// through`&T`!
///
/// You can get the `Scan` derive to automatically generate your `GcDeref` implementation for you.
/// To do this, you should supply the `#[shredder(can_deref)]` annotation. For example:
/// ```
/// use shredder::Scan;
///
/// #[derive(Scan)]
/// #[shredder(can_deref)]
/// struct ToPutInADerefGc {
///     random_data: u32
/// }
/// // `ToPutInADerefGc` will automagically get a `GcDeref` implementation
/// ```
pub unsafe trait GcDeref: Sync {}
