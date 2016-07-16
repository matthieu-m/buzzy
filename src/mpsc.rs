//! Multiple Producer Single Consumer (MPSC) queue.

use std::sync;

use common::{MultiProducer, QueueRaw, UniqueConsumer};
use interface::{Consumer, Producer, Queue};
use interface::{PopResult, PushResult};

//
//  Public Structs
//
/// MPSC Queue
#[derive(Clone)]
pub struct MpscQueue<T: Send + 'static>(sync::Arc<QueueRaw<T>>);

/// Consumer of MPSC Queue
pub struct MpscConsumer<T: Send + 'static>(UniqueConsumer<T>);

/// Producer of MPSC Queue
pub struct MpscProducer<T: Send + 'static>(MultiProducer<T>);

//
//  Public Methods
//
impl<T: Send + 'static> MpscQueue<T> {
    //  (sync::Arc<QueueRaw<T>>)

    /// Creates a new SPMSC Queue with the specified capacity.
    pub fn new(log2_capacity: u8) -> MpscQueue<T> {
        MpscQueue(sync::Arc::new(QueueRaw::new(log2_capacity, true, false)))
    }

    /// Spawn a new producer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of producers (`usize`) overflows.
    pub fn spawn_producer(&self) -> MpscProducer<T> {
        MpscProducer(MultiProducer::new(&self.0))
    }
}

//
//  Traits Implementations
//
impl<T: Send + 'static> Queue<T> for MpscQueue<T> {
    type Consumer = MpscConsumer<T>;
    type Producer = MpscProducer<T>;

    /// Returns the capacity of the queue.
    fn capacity(&self) -> usize { self.0.capacity() }

    /// Attempts to spawn a new consumer for this queue, this will fail if a
    /// consumer already exists.
    fn spawn_consumer(&self) -> Option<MpscConsumer<T>> {
        UniqueConsumer::new(&self.0).map(|c| MpscConsumer(c))
    }

    /// Spawn a new producer for this queue.
    ///
    /// # Panics
    ///
    /// May panic if the counter of producers (`usize`) overflows.
    fn spawn_producer(&self) -> Option<MpscProducer<T>> {
        Some(MpscQueue::<T>::spawn_producer(&self))
    }
}

impl<T: Send + 'static> Consumer<T> for MpscConsumer<T> {
    //  (UniqueConsumer<T>)

    /// Attempts to pop an item from the queue:
    /// - returns Item(T) if it succeeds
    /// - or Empty if the queue is empty (at the moment)
    ///
    /// *Note: though `PopResult<T>` is used, its `Concurrent` alternative is
    /// never returned here.*
    fn try_pop(&mut self) -> PopResult<T> { self.0.try_pop() }
}

impl<T: Send + 'static> Producer<T> for MpscProducer<T> {
    //  (MultiProducer<T>)

    /// Attempts to push the item into the queue:
    /// - returns Ok if it succeeds
    /// - or Full(item) if the queue is full (at the moment)
    /// - or Concurrent(item) if another producer pushed concurrently
    fn try_push(&mut self, item: T) -> PushResult<T> {
        self.0.try_push(item)
    }
}
