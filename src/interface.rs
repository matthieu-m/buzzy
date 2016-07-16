//! Generic interface of the queues, consumers and producers

//
//  Public Structs
//
/// The result of a call to `try_pop`.
pub enum PopResult<T> {
    /// The queue is currently empty, nothing to pop.
    Empty,
    /// Another consumer popped in our stead, consider retrying.
    Concurrent,
    /// The popped item.
    Item(T),
}

/// The result of a call to `try_push`.
pub enum PushResult<T> {
    /// The item was pushed.
    Ok,
    /// Another producer pusher in our stead, consider retrying.
    Concurrent(T),
    /// The queue is currently full, cannot push, so here is the pushed item.
    Full(T),
}

//
//  Public Traits
//
/// Minimal interface of all consumers.
pub trait Consumer<T>
    where T: Send + 'static
{
    /// Attempts to pop an item from the queue, only yielding the item if it
    /// succeeds.
    fn try_pop(&mut self) -> PopResult<T>;
}

/// Minimal interface of all producers.
pub trait Producer<T>
    where T: Send + 'static
{
    /// Attempts to push an item into the queue, yielding the item back if it
    /// fails.
    fn try_push(&mut self, item: T) -> PushResult<T>;
}

/// Minimal interface of all queues.
pub trait Queue<T>: Sync
    where T: Send + 'static
{
    /// Consumer to be used with this queue.
    type Consumer: Consumer<T>;
    /// Producer to be used with this queue.
    type Producer: Producer<T>;

    /// Returns the capacity of the queue.
    fn capacity(&self) -> usize;

    /// Attempts to spawn a new consumer for this queue, this may fail if the
    /// maximum number of consumers for the queue has been reached.
    fn spawn_consumer(&self) -> Option<Self::Consumer>;

    /// Attempts to spawn a new producer for this queue, this may fail if the
    /// maximum number of producers for the queue has been reached.
    fn spawn_producer(&self) -> Option<Self::Producer>;
}
