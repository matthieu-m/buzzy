//! Single Producer Single Consumer (SPSC) queue.

use std::sync;

use common::{QueueRaw, UniqueConsumer, UniqueProducer};
use interface::{Consumer, Producer, Queue};
use interface::{PopResult, PushResult};

//
//  Public Structs
//
/// SPSC Queue
#[derive(Clone)]
pub struct SpscQueue<T: Send + 'static>(sync::Arc<QueueRaw<T>>);

/// Consumer of SPSC Queue
pub struct SpscConsumer<T: Send + 'static>(UniqueConsumer<T>);

/// Producer of SPSC Queue
pub struct SpscProducer<T: Send + 'static>(UniqueProducer<T>);

//
//  Public Methods
//
impl<T: Send + 'static> SpscQueue<T> {
    //  (sync::Arc<QueueRaw<T>>)

    /// Creates a new SPSC Queue with the specified capacity.
    pub fn new(log2_capacity: u8) -> SpscQueue<T> {
        SpscQueue(sync::Arc::new(QueueRaw::new(log2_capacity, true, true)))
    }

}

//
//  Trait Implementations
//
impl<T: Send + 'static> Queue<T> for SpscQueue<T> {
    type Consumer = SpscConsumer<T>;
    type Producer = SpscProducer<T>;

    /// Returns the capacity of the queue
    fn capacity(&self) -> usize { self.0.capacity() }

    /// Attempts to spawn a new consumer for this queue, this will fail if a
    /// consumer already exists.
    fn spawn_consumer(&self) -> Option<SpscConsumer<T>> {
        UniqueConsumer::new(&self.0).map(|c| SpscConsumer(c))
    }

    /// Attempts to spawn a new producer for this queue, this will fail if a
    /// producer already exists.
    fn spawn_producer(&self) -> Option<SpscProducer<T>> {
        UniqueProducer::new(&self.0).map(|p| SpscProducer(p))
    }
}

impl<T: Send + 'static> Consumer<T> for SpscConsumer<T> {
    //  (UniqueConsumer<T>)

    /// Attempts to pop an item from the queue:
    /// - returns `Item(T)` if it succeeds
    /// - or `Empty` if the queue is empty (at the moment)
    ///
    /// *Note: though `PopResult<T>` is used, its `Concurrent` alternative is
    /// never returned here.*
    fn try_pop(&mut self) -> PopResult<T> { self.0.try_pop() }
}

impl<T: Send + 'static> Producer<T> for SpscProducer<T> {
    //  (UniqueProducer<T>)

    /// Attempts to push the item into the queue:
    /// - returns Ok if it succeeds
    /// - or Full(item) if it the queue is too full
    ///
    /// *Note: though `PushResult<T>` is used, its `Concurrent(T)` alternative
    /// is never returned here.*
    fn try_push(&mut self, item: T) -> PushResult<T> {
        self.0.try_push(item)
    }
}
