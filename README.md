# threadlanes

Real-time executors with deterministic task routing and guaranteed ordering.

## Example

User-defined Executor:
```
struct MyExecutor {
    id: usize,
}
impl LaneExecutor<usize> for MyExecutor {
    fn execute(&self, task: usize) {
       println!("{} received {}", self.id, task);
    }
}
```

Using ThreadLanes:
```
let lanes = ThreadLanes::new(vec![
    MyExecutor{id: 0},
    MyExecutor{id: 1},
    MyExecutor{id: 2},
]);

lanes.send(0, 11); // send task=11 to thread lane 0
lanes.send(1, 12); // send task=12 to thread lane 1
lanes.send(1, 13); // send task=13 to thread lane 1
lanes.send(2, 14); // send task=14 to thread lane 2
lanes.send(2, 15); // send task=15 to thread lane 2
lanes.send(2, 16); // send task=16 to thread lane 2

// flush tasks
lanes.flush();
```

In the output, you'll notice that task ordering is preserved for each Executor, but is not preserved between Executors:
```
1 received 12
0 received 11
1 received 13
2 received 14
2 received 15
2 received 16
```

## Why threadlanes

ThreadLanes are useful when you need deterministic task ordering through stateful Executors.
This is in contrast to a thread pool, where the thread that executes a task is not deterministic.
