#[cfg(test)]
mod tests {

    use crate::{LaneExecutor, ThreadLanes};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct AggregatingExecutor {
        value: Arc<AtomicUsize>,
    }
    impl AggregatingExecutor {
        fn new(value: Arc<AtomicUsize>) -> Self {
            Self { value }
        }
    }
    impl LaneExecutor<usize> for AggregatingExecutor {
        fn execute(&mut self, value: usize) {
            self.value.fetch_add(value, Ordering::AcqRel);
        }
    }

    #[test]
    fn multiple_lanes() {
        // create sharable atomic usizes to aggregate
        let v0 = Arc::new(AtomicUsize::new(0));
        let v1 = Arc::new(AtomicUsize::new(0));
        let v2 = Arc::new(AtomicUsize::new(0));

        // create executors that will aggregate shared atomic usizes
        let executors = vec![
            AggregatingExecutor::new(Arc::clone(&v0)),
            AggregatingExecutor::new(Arc::clone(&v1)),
            AggregatingExecutor::new(Arc::clone(&v2)),
        ];

        // create thread lanes and enqueue work for each lane
        let lanes = ThreadLanes::new(executors);
        lanes.send(0, 1);
        lanes.send(1, 2);
        lanes.send(1, 3);
        lanes.send(2, 4);
        lanes.send(2, 5);
        lanes.send(2, 6);

        // flush tasks
        lanes.flush();

        // assert values were aggregated from all events
        assert_eq!(v0.load(Ordering::Acquire), 1);
        assert_eq!(v1.load(Ordering::Acquire), 5);
        assert_eq!(v2.load(Ordering::Acquire), 15);
    }
}
