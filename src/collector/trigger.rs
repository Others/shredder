use parking_lot::Mutex;

// TODO(issue): https://github.com/Others/shredder/issues/8
const DEFAULT_TRIGGER_PERCENT: f32 = 0.75;
const MIN_ALLOCATIONS_FOR_COLLECTION: f32 = 512.0 * 1.3;

pub struct Trigger {
    data: Mutex<InternalTriggerData>,
}

struct InternalTriggerData {
    // Percent more allocations needed to trigger garbage collection
    gc_trigger_percent: f32,
    data_count_at_last_collection: usize,
}

impl Trigger {
    pub fn set_trigger_percent(&self, p: f32) {
        self.data.lock().gc_trigger_percent = p;
    }

    pub fn should_collect(&self, current_data_count: usize) -> bool {
        let internal_data = self.data.lock();

        // If we haven't reached the min allocation threshold, then hold off
        if (current_data_count as f32) < MIN_ALLOCATIONS_FOR_COLLECTION {
            return false;
        }

        let amount_of_new_data = current_data_count - internal_data.data_count_at_last_collection;
        let percent_more_data =
            amount_of_new_data as f32 / internal_data.data_count_at_last_collection as f32;

        // If we get NaN or Infinity, go ahead and optimistically say we should collect
        if percent_more_data.is_nan() || percent_more_data.is_infinite() {
            return true;
        }

        // Otherwise base our decision off the configured gc_trigger_percent
        percent_more_data >= internal_data.gc_trigger_percent
    }

    pub fn set_data_count_after_collection(&self, data_count: usize) {
        let mut internal_data = self.data.lock();
        internal_data.data_count_at_last_collection = data_count;
    }
}

impl Default for Trigger {
    fn default() -> Self {
        Trigger {
            data: Mutex::new(InternalTriggerData {
                gc_trigger_percent: DEFAULT_TRIGGER_PERCENT,
                data_count_at_last_collection: 0,
            }),
        }
    }
}
