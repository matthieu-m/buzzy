//! Single Producer Multiple Consumer (SPMC) queue.

use std::sync;

use common::{MultiConsumer, QueueRaw, UniqueProducer};
use interface::{Consumer, Producer, Queue};
use interface::{PopResult, PushResult};

//
//  Public Structs
//
/// SPMC Queue
#[derive(Clone)]
pub struct SpmcQueue<T: Send + 'static>(sync::Arc<QueueRaw<T>>);

/// Consumer of SPMC Queue
pub struct SpmcConsumer<T: Send + 'static>(MultiConsumer<T>);

/// Producer of SPMC Queue
pub struct SpmcProducer<T: Send + 'static>(UniqueProducer<T>);

//
//  Public Methods
//
impl<T: Send + 'static> SpmcQueue<T> {
    //  (sync::Arc<QueueRaw<T>>)

    /// Creates a new SPMSC Queue with the specified capacity.
    pub fn new(log2_capacity: u8) -> SpmcQueue<T> {
        SpmcQueue(sync::Arc::new(QueueRaw::new(log2_capacity, false, true)))
    }

    /// Spawn a new consumer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of consumers (`usize`) overflows.
    pub fn spawn_consumer(&self) -> SpmcConsumer<T> {
        SpmcConsumer(MultiConsumer::new(&self.0))
    }
}

//
//  Traits Implementations
//
impl<T: Send + 'static> Queue<T> for SpmcQueue<T> {
    type Consumer = SpmcConsumer<T>;
    type Producer = SpmcProducer<T>;

    /// Returns the capacity of the queue.
    fn capacity(&self) -> usize { self.0.capacity() }

    /// Spawn a new consumer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of consumers (`usize`) overflows.
    fn spawn_consumer(&self) -> Option<SpmcConsumer<T>> {
        Some(SpmcQueue::<T>::spawn_consumer(&self))
    }

    /// Attempts to spawn a new producer for this queue, this will fail if a
    /// producer already exists.
    fn spawn_producer(&self) -> Option<SpmcProducer<T>> {
        UniqueProducer::new(&self.0).map(|p| SpmcProducer(p))
    }
}

impl<T: Send + 'static> Consumer<T> for SpmcConsumer<T> {
    //  (MultiConsumer<T>)

    /// Attempts to pop an item from the queue:
    /// - returns Item(T) if it succeeds
    /// - or Empty if the queue is empty (at the moment)
    /// - or Concurrent if another consumer popped concurrently
    fn try_pop(&mut self) -> PopResult<T> { self.0.try_pop() }
}

impl<T: Send + 'static> Producer<T> for SpmcProducer<T> {
    //  (UniqueProducer<T>)

    /// Attempts to push the item into the queue:
    /// - returns Ok if it succeeds
    /// - or Full(item) if the queue is full (at the moment)
    ///
    /// *Note: though `PushResult<T>` is used, its `Concurrent(T)` alternative
    /// is never returned here.*
    fn try_push(&mut self, item: T) -> PushResult<T> {
        self.0.try_push(item)
    }
}
