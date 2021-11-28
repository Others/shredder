use std::sync::atomic::{AtomicI64, Ordering};

#[derive(Debug)]
pub struct GcRefCount {
    // `total_handles` = count_positive + count_negative
    // `count_positive` is always >= the actual count >= 0
    // At the beginning of every collection we recalculate `count_positive`
    count_positive: AtomicI64,
    // `count_negative` is always <= 0
    count_negative: AtomicI64,

    // Handles found in the current collection
    found_internally: AtomicI64,
}

impl GcRefCount {
    pub fn new(starting_count: i64) -> Self {
        let s = Self {
            count_positive: AtomicI64::new(starting_count),
            count_negative: AtomicI64::new(0),
            found_internally: AtomicI64::new(0),
        };
        s.override_mark_as_rooted();

        s
    }

    pub fn prepare_for_collection(&self) {
        // Ordering = relaxed, as this is protected by the collection mutex
        self.found_internally.store(0, Ordering::Relaxed);

        // `Ordering::Acquire` to sequence with the `Release` in `dec_count`
        let negative = self.count_negative.swap(0, Ordering::Acquire);
        // `Ordering::Acquire` to sequence with the `Release` in `inc_count`
        let fixed_positive = self.count_positive.fetch_add(negative, Ordering::Acquire);

        // If enabled, double check that we're adhering to the invariant
        debug_assert!(fixed_positive >= 0);
    }

    pub fn found_once_internally(&self) {
        // Ordering = relaxed, as this is protected by the collection mutex
        self.found_internally.fetch_add(1, Ordering::Relaxed);
    }

    pub fn is_rooted(&self) -> bool {
        // Due to the count invariant, this can never be less than the actual count
        // Ordering = Acquire for the same reason as `prepare_for_collection`
        let actual_count_plus_n = self.count_positive.load(Ordering::Acquire);
        // Ordering = Relaxed, as this is protected by the collection mutex
        let found_internally = self.found_internally.load(Ordering::Relaxed);

        if actual_count_plus_n > found_internally {
            // If n = 0, then `actual_count > found_internally` so we know we're rooted
            // If n > 0, then we may or may not be rooted, but it's safe to assume we are
            return true;
        }
        // In this case `actual_count + n <= found_internally`
        // This implies `actual_count <= found_internally`
        // So we found at least as many handles as actually exist, so this data must not be rooted
        false
    }

    const ROOT_OVERRIDE_VALUE: i64 = -(1 << 60);

    pub fn override_mark_as_rooted(&self) {
        // Ordering = relaxed, as this is protected by the collection mutex
        self.found_internally
            .store(Self::ROOT_OVERRIDE_VALUE, Ordering::Relaxed);
    }

    pub fn was_overriden_as_rooted(&self) -> bool {
        self.found_internally.load(Ordering::Relaxed) == Self::ROOT_OVERRIDE_VALUE
    }

    pub fn inc_count(&self) {
        // `Ordering::Release` to sequence with the `Acquire` in `prepare_for_collection`
        self.count_positive.fetch_add(1, Ordering::Release);
    }

    pub fn dec_count(&self) {
        // `Ordering::Release` to sequence with the `Acquire` in `prepare_for_collection`
        self.count_negative.fetch_sub(1, Ordering::Release);
    }

    pub fn snapshot_ref_count(&self) -> i64 {
        let positive = self.count_positive.load(Ordering::Acquire);
        let negative = self.count_negative.load(Ordering::Acquire);

        positive + negative
    }
}
