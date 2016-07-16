//! Multiple Producer Multiple Consumer (MPMC) queue.

use std::sync;

use common::{MultiConsumer, MultiProducer, QueueRaw};
use interface::{Consumer, Producer, Queue};
use interface::{PopResult, PushResult};

//
//  Public Structs
//
/// MPMC Queue
#[derive(Clone)]
pub struct MpmcQueue<T: Send + 'static>(sync::Arc<QueueRaw<T>>);

/// Consumer of MPMC Queue
pub struct MpmcConsumer<T: Send + 'static>(MultiConsumer<T>);

/// Producer of MPMC Queue
pub struct MpmcProducer<T: Send + 'static>(MultiProducer<T>);

//
//  Public Methods
//
impl<T: Send + 'static> MpmcQueue<T> {
    //  (sync::Arc<QueueRaw<T>>)

    /// Creates a new SPMSC Queue with the specified capacity.
    pub fn new(log2_capacity: u8) -> MpmcQueue<T> {
        MpmcQueue(sync::Arc::new(QueueRaw::new(log2_capacity, false, false)))
    }

    /// Returns the capacity of the queue
    pub fn capacity(&self) -> usize { self.0.capacity() }

    /// Spawn a new consumer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of consumers (a `usize`) overflows.
    pub fn spawn_consumer(&self) -> MpmcConsumer<T> {
        MpmcConsumer(MultiConsumer::new(&self.0))
    }

    /// Spawn a new producer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of producers (a `usize`) overflows.
    pub fn spawn_producer(&self) -> MpmcProducer<T> {
        MpmcProducer(MultiProducer::new(&self.0))
    }
}

//
//  Traits Implementations
//
impl<T: Send + 'static> Queue<T> for MpmcQueue<T> {
    type Consumer = MpmcConsumer<T>;
    type Producer = MpmcProducer<T>;

    /// Returns the capacity of the queue.
    fn capacity(&self) -> usize { self.0.capacity() }

    /// Spawn a new consumer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of consumers (a `usize`) overflows.
    fn spawn_consumer(&self) -> Option<MpmcConsumer<T>> {
        Some(MpmcQueue::<T>::spawn_consumer(&self))
    }

    /// Spawn a new producer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of producers (a `usize`) overflows.
    fn spawn_producer(&self) -> Option<MpmcProducer<T>> {
        Some(MpmcQueue::<T>::spawn_producer(&self))
    }
}

impl<T: Send + 'static> Consumer<T> for MpmcConsumer<T> {
    //  (Consumer<MpmcQueueRaw<T>>)

    /// Attempts to pop an item from the queue:
    /// - returns Item(T) if it succeeds
    /// - or Empty if the queue is empty (at the moment)
    /// - or Concurrent if another consumer popped concurrently
    fn try_pop(&mut self) -> PopResult<T> { self.0.try_pop() }
}

impl<T: Send + 'static> Producer<T> for MpmcProducer<T> {
    //  (Producer<MpmcQueueRaw<T>>)

    /// Attempts to push the item into the queue:
    /// - returns Ok if it succeeds
    /// - or Full(item) if the queue is full (at the moment)
    /// - or Concurrent(item) if another producer pushed concurrently
    fn try_push(&mut self, item: T) -> PushResult<T> {
        self.0.try_push(item)
    }
}
