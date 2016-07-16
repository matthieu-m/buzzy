//! Common building blocks for the concurrent queues, consumers and producers.

use std::{cmp, isize, marker, mem, ptr, sync, usize};
use std::sync::atomic::{AtomicUsize, Ordering};

use interface::{PopResult, PushResult};

//
//  Private Constants
//

//
//  In order to avoid false-sharing between buffer, read_index and write_index,
//  we make sure to spread them over two different cache lines.
//
//  # Portability
//
//  We take the assumption that a cache line is 64 bytes:
//  - 8 pointers in 64 bits
//  - 16 pointers in 32 bits
//
//  This leads us to two different representations, depending on the width.
//
#[cfg(target_pointer_width = "64")]
const READ_QUEUE_PADDING_SIZE: usize = 5;

#[cfg(target_pointer_width = "64")]
const WRITE_QUEUE_PADDING_SIZE: usize = 7;

#[cfg(target_pointer_width = "32")]
const READ_QUEUE_PADDING_SIZE: usize = 13;

#[cfg(target_pointer_width = "32")]
const WRITE_QUEUE_PADDING_SIZE: usize = 15;

//
//  In order to avoid false-sharing between two consecutive items in StridedRaw,
//  we define a stride so that they end up on two different cache lines.
//
//  # Portability
//
//  We take the assumption that a cache line is 64 bytes:
//
//  - 8 pointers in 64 bits
//  - 16 pointers in 32 bits
//
//  We also take the assumption that the item will be at least 2 pointers in
//  size, one pointer being the atomic flag and the other the element (padded if
//  necessary).
//
//  This leads us to two different strides, depending on the width:
//
//  - a stride of 4 in 64 bits
//  - a stride of 8 in 32 bits
//
#[cfg(target_pointer_width = "64")]
const LOG_2_STRIDE: usize = 2;

#[cfg(target_pointer_width = "32")]
const LOG_2_STRIDE: usize = 3;

//
//  Public Structs
//

/// Building block for concurrent queues.
///
/// It contains all necessary data members for a queue:
/// - internal fixed-size buffer for the items,
/// - the counters of consumers and producers,
/// - the position in the buffer to read from and write to,
/// - the necessary padding to avoid false-sharing between these positions.
///
/// It handles the initialization and clean-up of the buffer, as well as provide
/// the underlying manipulation methods used by the various consumers and
/// producers.
///
/// # Concurrency
///
/// Efficient concurrency is supported by the following key points:
///
/// - the buffer pointer, read index and write index are guaranteed to be on
///     separate cache lines; the buffer pointer will be shared by all cores,
///     while the read index will only bounce between consumer cores and the
///     write index will only bounce between producer cores.
/// - the underlying buffer is accessed in a strided fashion, so that
///     consecutive indexes point to items on different cache lines.
/// - the read (resp. write) index is generally only accessed once for each item
///     that is popped (resp. pushed), when there multiple consumers (resp.
///     producers).
/// - the read and write indices provide optimistic repartition of consumers and
///     producers to minimize contention.
///
/// It is to be noted that the optimistic nature of the repartition of consumers
/// and producers is insufficient for guaranteeing safety and requires another
/// stricter mechanism (one atomic flag per item). Its only role is to decrease
/// the contention between parallel consumers and producers.
#[repr(C)]
#[allow(drop_with_repr_extern)]
pub struct QueueRaw<T: Send + 'static> {
    /// Strided buffer handling the items.
    buffer: StridedRaw<T>,
    /// Counter of consumers:
    /// - 0/1 in unique mode, if an instance of UniqueConsumer is registered,
    /// - n+2 if n instances of MultiConsumer are registered.
    consumers: AtomicUsize,
    /// Counter of producers:
    /// - 0/1 in unique mode, if an instance of UniqueProducer is registered,
    /// - n+2 if n instances of MultiProducer are registered.
    producers: AtomicUsize,
    /// Padding to ensure that "buffer" and "read_index" are on different cache
    /// lines.
    _read_padding: [usize; READ_QUEUE_PADDING_SIZE],
    /// Next position to read from.
    read_index: AtomicUsize,
    /// Padding to ensure that "read_index" and "write_index" are on different
    /// cache lines.
    _write_padding: [usize; WRITE_QUEUE_PADDING_SIZE],
    /// Next position to write to.
    write_index: AtomicUsize,
}

/// Building block for a unique consumer of concurrent queue.
pub struct UniqueConsumer<T: Send + 'static> {
    queue: sync::Arc<QueueRaw<T>>,
}

/// Building block for a consumer among many of concurrent queue.
pub struct MultiConsumer<T: Send + 'static> {
    queue: sync::Arc<QueueRaw<T>>,
    read_next: OptionalIndex,
}

/// Building block for a unique producer of a concurrent queue.
pub struct UniqueProducer<T: Send + 'static> {
    queue: sync::Arc<QueueRaw<T>>,
}

/// Building block for a producer among many of concurrent queue.
pub struct MultiProducer<T: Send + 'static> {
    queue: sync::Arc<QueueRaw<T>>,
    write_next: OptionalIndex,
}

//
//  Private Struct
//

//
//  In order to avoid false-sharing between the items stored at successive
//  indexes, we use a strided representation.
//
//  Instead of accessing items as:
//
//      [ 0,  1,  2,  3,  4,  5,  6,  7]
//
//      [ 0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15]
//
//  A stride of x with a capacity of n will access items (x/n):
//
//      2/8: [ 0,  4,  1,  5,  2,  6,  3,  7]
//
//      2/16: [ 0,  8,  1,  9,  2, 10,  3, 11,  4, 12,  5, 13,  6, 14,  7, 15]
//
//      4/16: [ 0,  4,  8, 12,  1,  5,  9, 13,  2,  6, 10, 14,  3,  7, 11, 15]
//
//  That is, the access function is remapped to:
//
//      fn i -> (i % stride) * capacity / stride + i / stride
//
struct StridedRaw<T: Send + 'static> {
    items: *mut Item<T>,
    raw_capacity: usize,
    log2_capacity: usize,
    _marker: marker::PhantomData<T>,
}

struct Item<T: Send + 'static> {
    data: T,
    flag: AtomicGeneration,
}

//
//  The generation encodes two pieces of information:
//
//  - the actual generation: index % capacity
//  - a bit indicating whether the generation was set by a consumer or producer
//
//  The actual representation is: (index % capacity) << 1 + flag, where flag is
//  0 if set by a reader (consumed) or 1 if set by a producer (produced).
//
#[derive(Clone, Copy, Eq, PartialEq)]
struct Generation(usize);

struct AtomicGeneration(AtomicUsize);

#[derive(Clone, Copy)]
struct Index(usize);

#[derive(Clone, Copy)]
struct OptionalIndex(usize);

#[derive(Clone, Copy)]
enum GenerationState {
    /// Production cancelled
    /// Occurs when one of multiple producers is dropped whilst having reserved
    /// an index to write to.
    Dead,
    /// Production in progress
    InProgress,
    /// Item consumed by generation "usize"
    Consumed(usize),
    /// Item produced by generation "usize"
    Produced(usize),
}

//
//  Traits Implementations
//
unsafe impl<T: Send + 'static> Send for QueueRaw<T> {}

unsafe impl<T: Send + 'static> Sync for QueueRaw<T> {}

impl<T: Send + 'static> Drop for UniqueConsumer<T> {
    fn drop(&mut self) {self.queue.dec_consumers(); }
}

impl<T: Send + 'static> Drop for MultiConsumer<T> {
    fn drop(&mut self) { self.queue.dec_consumers(); }
}

impl<T: Send + 'static> Drop for UniqueProducer<T> {
    fn drop(&mut self) { self.queue.dec_producers(); }
}

impl<T: Send + 'static> Drop for MultiProducer<T> {
    fn drop(&mut self) {
        match self.write_next.get() {
            Some(index) => self.queue.cancel_push(index),
            None => (),
        };
        self.queue.dec_producers();
    }
}

impl<T: Send + 'static> Drop for StridedRaw<T> {
    fn drop(&mut self) {
        self.clear();
        unsafe { Vec::from_raw_parts(self.items, 0, self.raw_capacity) };
    }
}

impl Default for Generation {
    fn default() -> Generation { Generation::new_consumed(0) }
}

impl Default for OptionalIndex {
    fn default() -> OptionalIndex { OptionalIndex(usize::MAX) }
}

//
//  Public methods
//
impl<T: Send + 'static> QueueRaw<T> {
    //  buffer: StridedRaw<T>,
    //  consumers: AtomicUsize,
    //  producers: AtomicUsize,
    //  read_index: AtomicUsize,
    //  write_index: AtomicUsize,

    /// Creates a new `QueueRaw` with the specified capacity.
    ///
    /// For safety reasons, it is necessary to specify whether a unique consumer
    /// (resp. producer) or multiple ones will be used with this queue.
    pub fn new(
        log2_capacity: u8,
        unique_consumer: bool,
        unique_producer: bool
    ) -> QueueRaw<T>
    {
        assert!(
            mem::size_of::<QueueRaw<T>>() == 3 * mem::size_of::<usize>(),
            "Broken size assumption!"
        );

        let buffer = StridedRaw::new(log2_capacity);
        let capacity = buffer.capacity();

        QueueRaw{
            buffer: buffer,
            consumers: AtomicUsize::new(if unique_consumer { 0 } else { 2 }),
            producers: AtomicUsize::new(if unique_producer { 0 } else { 2 }),
            _read_padding: [0; READ_QUEUE_PADDING_SIZE],
            read_index: AtomicUsize::new(capacity),
            _write_padding: [0; WRITE_QUEUE_PADDING_SIZE],
            write_index: AtomicUsize::new(0),
        }
    }

    /// Returns the capacity of the queue.
    pub fn capacity(&self) -> usize { self.buffer.capacity() }
}

impl<T: Send + 'static> UniqueConsumer<T> {
    //  queue: sync::Arc<QueueRaw<T>>,

    /// Attempts to create a new consumer for the given queue:
    /// - if the counter is 0, succeeds, incrementing the counter of consumers
    /// and cloning the `Arc`.
    /// - otherwise fails, leaving the counter of consumers unmodified and not
    /// cloning the the `Arc`.
    pub fn new(queue: &sync::Arc<QueueRaw<T>>) -> Option<UniqueConsumer<T>> {
        if queue.inc_unique_consumer() {
            Some(UniqueConsumer { queue: queue.clone() })
        } else {
            None
        }
    }

    /// Attempts to pop an item from the queue:
    /// - returns `Item(T)` if it succeeds
    /// - or `Empty` if the queue is empty (at the moment)
    ///
    /// *Note: though `PopResult<T>` is used, its `Concurrent` alternative is
    /// never returned here.*
    pub fn try_pop(&mut self) -> PopResult<T> {
        let index = self.queue.read_index.load(Ordering::Relaxed);

        //  OK because we guarantee that there is a single consumer.
        match unsafe { self.queue.try_pop_unique(Index::new(index)) } {
            PopResult::Item(result) => {
                self.queue.read_index.store(index + 1, Ordering::Relaxed);
                PopResult::Item(result)
            },
            //  Happens in the case one of multiple producers is dropped whilst
            //  still having reserved this index; in which case it marks it as
            //  consumed with a greater generation to indicate to the consumer
            PopResult::Concurrent => {
                self.queue.read_index.store(index + 1, Ordering::Relaxed);
                PopResult::Empty
            },
            PopResult::Empty => {
                PopResult::Empty
            },
        }
    }
}

impl<T: Send + 'static> MultiConsumer<T> {
    //  queue: sync::Arc<QueueRaw<T>>,
    //  read_next: OptionalIndex,

    /// Creates a new consumer for the given queue, incrementing the counter of
    /// consumers as it goes and cloning the `Arc`.
    pub fn new(queue: &sync::Arc<QueueRaw<T>>) -> MultiConsumer<T> {
        queue.inc_consumers();
        MultiConsumer {
            queue: queue.clone(),
            read_next: OptionalIndex::default()
        }
    }

    /// Attempts to pop an item from the queue:
    /// - returns Item(T) if it succeeds
    /// - or Empty if the queue is empty (at the moment)
    /// - or Concurrent if another consumer popped concurrently
    pub fn try_pop(&mut self) -> PopResult<T> {
        let index = self.get_read_index();

        match self.queue.try_pop(index) {
            PopResult::Item(result) => {
                self.reset_read_index();
                PopResult::Item(result)
            },
            PopResult::Concurrent => {
                self.reset_read_index();
                PopResult::Concurrent
            },
            result => result,
        }
    }
}

impl<T: Send + 'static> UniqueProducer<T> {
    //  queue: sync::Arc<QueueRaw<T>>,

    /// Attempts to create a new producer for the given queue:
    /// - if the counter is 0, succeeds, incrementing the counter of producers
    /// and cloning the `Arc`.
    /// - otherwise fails, leaving the counter of producers unmodified and not
    /// cloning the the `Arc`.
    pub fn new(queue: &sync::Arc<QueueRaw<T>>) -> Option<UniqueProducer<T>> {
        if queue.inc_unique_producer() {
            Some(UniqueProducer { queue: queue.clone() })
        } else {
            None
        }
    }

    /// Attempts to push the item into the queue:
    /// - returns Ok if it succeeds
    /// - or Full(item) if it the queue is too full
    ///
    /// *Note: though `PushResult<T>` is used, its `Concurrent(T)` alternative
    /// is never returned here.*
    pub fn try_push(&mut self, item: T) -> PushResult<T> {
        let index = self.queue.write_index.load(Ordering::Relaxed);

        if let PushResult::Full(result) =
            //  OK: there is a unique producer
            unsafe { self.queue.try_push_unique(item, Index::new(index)) }
        {
            PushResult::Full(result)
        } else {
            self.queue.write_index.store(index + 1, Ordering::Relaxed);
            PushResult::Ok
        }
    }
}

impl<T: Send + 'static> MultiProducer<T> {
    //  queue: sync::Arc<QueueRaw<T>>,
    //  write_next: OptionalIndex,

    /// Creates a new Producer for the given queue, incrementing the counter of
    /// producers as it goes and cloning the `Arc`.
    pub fn new(queue: &sync::Arc<QueueRaw<T>>) -> MultiProducer<T> {
        queue.inc_producers();
        MultiProducer {
            queue: queue.clone(),
            write_next: OptionalIndex::default()
        }
    }

    /// Attempts to push the item into the queue:
    /// - returns Ok if it succeeds
    /// - or Full(item) if the queue is full (at the moment)
    /// - or Concurrent(item) if another producer pushed concurrently
    pub fn try_push(&mut self, item: T) -> PushResult<T> {
        let index = self.get_write_index();

        match self.queue.try_push(item, index) {
            PushResult::Ok => {
                self.reset_write_index();
                PushResult::Ok
            },
            PushResult::Concurrent(item) => {
                self.reset_write_index();
                PushResult::Concurrent(item)
            },
            result => result,
        }
    }
}

//
//  Private Methods
//
impl<T: Send + 'static> QueueRaw<T> {
    //  buffer: StridedRaw<T>,
    //  consumers: AtomicUsize,
    //  producers: AtomicUsize,
    //  read_index: AtomicUsize,
    //  write_index: AtomicUsize,

    fn inc_unique_consumer(&self) -> bool {
        self.consumers.compare_and_swap(0, 1, Ordering::Relaxed) == 0
    }

    fn inc_consumers(&self) { self.consumers.fetch_add(1, Ordering::Relaxed); }

    fn dec_consumers(&self) { self.consumers.fetch_sub(1, Ordering::Relaxed); }

    fn inc_unique_producer(&self) -> bool {
        self.producers.compare_and_swap(0, 1, Ordering::Relaxed) == 0
    }

    fn inc_producers(&self) { self.producers.fetch_add(1, Ordering::Relaxed); }

    fn dec_producers(&self) { self.producers.fetch_sub(1, Ordering::Relaxed); }

    unsafe fn try_pop_unique(&self, index: Index) -> PopResult<T> {
        self.buffer.try_pop_unique(index)
    }

    fn try_pop(&self, index: Index) -> PopResult<T> {
        self.buffer.try_pop(index)
    }

    unsafe fn try_push_unique(&self, value: T, index: Index) -> PushResult<T> {
        self.buffer.try_push_unique(value, index)
    }

    fn try_push(&self, value: T, index: Index) -> PushResult<T> {
        self.buffer.try_push(value, index)
    }

    fn cancel_push(&self, index: Index) {
        self.buffer.cancel_push(index);
    }

}

impl<T: Send + 'static> MultiConsumer<T> {
    //  queue: sync::Arc<QueueRaw<T>>,
    //  read_next: OptionalIndex,

    /// Get the current value of the read index.
    ///
    /// This may require getting said value from the queue.
    fn get_read_index(&mut self) -> Index {
        if let Some(index) = self.read_next.get() {
            return index;
        }

        let index =
            Index::new(self.queue.read_index.fetch_add(1, Ordering::Relaxed));
        self.read_next = OptionalIndex::new(index);
        index
    }

    /// Resets the read index, it has been used.
    fn reset_read_index(&mut self) { self.read_next = OptionalIndex::empty(); }
}

impl<T: Send + 'static> MultiProducer<T> {
    //  queue: sync::Arc<QueueRaw<T>>,
    //  write_next: OptionalIndex,

    /// Get the current value of the write index.
    ///
    /// This may require getting said value from the queue.
    fn get_write_index(&mut self) -> Index {
        if let Some(index) = self.write_next.get() {
            return index;
        }

        let index =
            Index::new(self.queue.write_index.fetch_add(1, Ordering::Relaxed));
        self.write_next = OptionalIndex::new(index);
        index
    }

    /// Resets the write index, it has been used.
    fn reset_write_index(&mut self) { self.write_next = OptionalIndex::empty(); }
}

impl<T: Send + 'static> StridedRaw<T> {
    //  items: *mut Item<T>,
    //  raw_capacity: usize,
    //  log2_capacity: usize,

    /// Creates a StridedRaw buffer with the specific stride and capacity.
    /// The buffer is zeroed, which sets all generation flags to 0.
    fn new(log2_capacity: u8) -> StridedRaw<T> {
        //  If the capacity is too small compared to the stride, then two
        //  consecutive indices may accidentally end up on the same line. It is
        //  necessary that the capacity be at least 4 times the stride.
        let log2_capacity = cmp::max(LOG_2_STRIDE + 2, log2_capacity as usize);

        assert!(
            log2_capacity < 64,
            "Too large capacity 2^{}.", log2_capacity
        );

        let capacity = 1u64 << (log2_capacity as u64);

        assert!(
            capacity < (isize::MAX as u64) + 1,
            "Too large capacity {}", capacity
        );

        let mut v = Vec::with_capacity(capacity as usize);

        let raw_capacity = v.capacity();

        let p = v.as_mut_ptr();
        mem::forget(v);

        //  OK: elements are not read unless the accompanying flags have a
        //      lower bit of 1, so we are marking all elements as uninitialized.
        unsafe { ptr::write_bytes(p, 0, capacity as usize) };

        StridedRaw {
            items: p,
            raw_capacity: raw_capacity,
            log2_capacity: log2_capacity,
            _marker: marker::PhantomData,
        }
    }

    /// Returns the capacity of the buffer.
    fn capacity(&self) -> usize { 1_usize << self.log2_capacity }

    /// Special case pop when there is a guaranteed unique consumer.
    ///
    /// # Safety
    ///
    /// Should only be called if there is a unique consumer.
    unsafe fn try_pop_unique(&self, index: Index) -> PopResult<T> {
        Item::<T>::try_pop_unique(
            self.items.offset(self.resolve_index(index) as isize),
            Generation::new_consumed(self.resolve_generation(index)),
        )
    }

    /// Generic purpose pop when the number of consumers is unknown.
    fn try_pop(&self, index: Index) -> PopResult<T> {
        Item::<T>::try_pop(
            //  OK: "resolve_index" ensures we do not run out of bounds,
            //      and "try_pop" will check the initialization flag.
            unsafe { self.items.offset(self.resolve_index(index) as isize) },
            Generation::new_consumed(self.resolve_generation(index)),
        )
    }

    /// Special case push when there is a guaranteed unique producer.
    ///
    /// # Safety
    ///
    /// Should only be called if there is a unique producer.
    unsafe fn try_push_unique(&self, value: T, index: Index) -> PushResult<T> {
        Item::<T>::try_push_unique(
            value,
            self.items.offset(self.resolve_index(index) as isize),
            Generation::new_produced(self.resolve_generation(index)),
        )
    }

    /// Generic purpose push when the number of consumers is unknown.
    fn try_push(&self, value: T, index: Index) -> PushResult<T> {
        Item::<T>::try_push(
            value,
            //  OK: "resolve_index" ensures we do not run out of bounds,
            //      and "try_pop" will check the initialization flag.
            unsafe { self.items.offset(self.resolve_index(index) as isize) },
            Generation::new_produced(self.resolve_generation(index)),
        )
    }

    /// Even though this slot was reserved, it will not be used. If necessary,
    /// mark it as already read.
    fn cancel_push(&self, index: Index) {
        Item::<T>::cancel_push(
            //  OK: "resolve_index" ensures we do not run out of bounds,
            //      and "try_pop" will check the initialization flag.
            unsafe { self.items.offset(self.resolve_index(index) as isize) },
        );
    }

    /// Clears all existing items in the underlying buffer of queue.
    fn clear(&mut self) {
        for i in 0..self.capacity() {
            //  OK because "&mut self" guarantees there is a unique consumer.
            unsafe {
                Item::<T>::try_pop_unique(
                    self.items.offset(i as isize),
                    Generation::default(),
                );
            }
        }
    }

    /// Transforms the logical index into its strided view.
    fn resolve_index(&self, index: Index) -> usize {
        const STRIDE_MASK: usize = (1_usize << LOG_2_STRIDE) - 1;
        let multiplier = self.log2_capacity - LOG_2_STRIDE;

        //  (index % stride) * capacity / stride + index / stride
        ((index.0 & STRIDE_MASK) << multiplier) + (index.0 >> LOG_2_STRIDE)
    }

    /// Transforms the logical index into its generation.
    fn resolve_generation(&self, index: Index) -> usize {
        index.0 >> self.log2_capacity
    }
}

impl<T: Send + 'static> Item<T> {
    //  data: T,
    //  flag: AtomicGeneration,

    /// Special case pop when there is a guaranteed unique consumer.
    ///
    /// # Safety
    ///
    /// Should only be called if there is a unique consumer.
    unsafe fn try_pop_unique(p: *const Item<T>, gen: Generation)
        -> PopResult<T>
    {
        let flag = &(*p).flag;
        let current = flag.load(Ordering::Acquire);

        match current.get_state() {
            GenerationState::InProgress => PopResult::Empty,
            GenerationState::Consumed(_) => PopResult::Empty,
            GenerationState::Dead => {
                flag.store(gen, Ordering::Relaxed);
                PopResult::Concurrent
            },
            GenerationState::Produced(_) => {
                let result = ptr::read(p).data;
                flag.store(gen, Ordering::Release);
                PopResult::Item(result)
            }
        }
    }

    /// Generic purpose pop when the number of consumers is unknown.
    fn try_pop(p: *const Item<T>, gen: Generation) -> PopResult<T> {
        let flag = unsafe { &(*p).flag };
        let current = flag.load(Ordering::Acquire);

        let new_generation = gen.get_generation();

        match current.get_state() {
            GenerationState::InProgress => PopResult::Empty,
            GenerationState::Dead => {
                flag.store(gen, Ordering::Relaxed);
                PopResult::Concurrent
            },
            GenerationState::Consumed(g) => {
                if g < new_generation { PopResult::Empty }
                else { PopResult::Concurrent }
            },
            GenerationState::Produced(_) => {
                let mut result = unsafe { mem::uninitialized() };

                unsafe {
                    ptr::copy_nonoverlapping(&(*p).data, &mut result, 1)
                };

                if flag.compare_and_swap(current, gen, Ordering::AcqRel) !=
                    current
                {
                    mem::forget(result);
                    PopResult::Concurrent
                } else {
                    PopResult::Item(result)
                }
            }
        }
    }

    /// Special case push when there is a guaranteed unique producer.
    ///
    /// # Safety
    ///
    /// Should only be called if there is a unique producer.
    unsafe fn try_push_unique(value: T, p: *mut Item<T>, gen: Generation)
        -> PushResult<T>
    {
        let flag = &(*p).flag;
        let current = flag.load(Ordering::Acquire);

        match current.get_state() {
            GenerationState::Dead | GenerationState::InProgress => {
                panic!("Another producer acting concurrently?");
            },
            GenerationState::Produced(g) => {
                assert!(g < gen.get_generation(), "Backward time travel?");
                PushResult::Full(value)
            },
            GenerationState::Consumed(_) => {
                ptr::write(&mut (*p).data, value);
                flag.store(gen, Ordering::Release);
                PushResult::Ok
            },
        }
    }

    /// Generic purpose push when the number of producers is unknown.
    fn try_push(value: T, p: *mut Item<T>, gen: Generation) -> PushResult<T> {
        let flag = unsafe { &(*p).flag };
        let current = flag.load(Ordering::Acquire);

        let new_generation = gen.get_generation();

        match current.get_state() {
            GenerationState::InProgress => {
                PushResult::Concurrent(value)
            },
            GenerationState::Produced(g) => {
                if g < new_generation {
                    PushResult::Full(value)
                } else {
                    PushResult::Concurrent(value)
                }
            },
            GenerationState::Dead | GenerationState::Consumed(_) => {
                if flag.compare_and_swap(
                    current,
                    Generation::new_in_progress(),
                    Ordering::AcqRel
                ) != current {
                    return PushResult::Concurrent(value);
                }

                unsafe { ptr::write(&mut (*p).data, value) };
                flag.store(gen, Ordering::Release);

                PushResult::Ok
            }
        }
    }

    /// Even though this slot was reserved, it will not be used. If necessary,
    /// mark it as already read.
    fn cancel_push(p: *mut Item<T>) {
        let flag = unsafe { &(*p).flag };
        let current = flag.load(Ordering::Acquire);

        match current.get_state() {
            GenerationState::Consumed(_) => {
                let gen = Generation::new_dead();
                flag.compare_and_swap(current, gen, Ordering::Relaxed);
            },
            _ => (),
        };
    }
}

impl Generation {
    //  (usize)

    fn new_in_progress() -> Generation { Generation(usize::MAX) }

    fn new_dead() -> Generation { Generation(usize::MAX - 1) }

    fn new_consumed(gen: usize) -> Generation {
        let result = Generation(gen << 1);
        assert!(
            result != Generation::new_dead(),
            "Dead marker conflict detected"
        );

        result
    }

    fn new_produced(gen: usize) -> Generation {
        let result = Generation(gen << 1 + 1);
        assert!(
            result != Generation::new_in_progress(),
            "InProgress marker conflict detected"
        );

        result
    }

    fn get_state(&self) -> GenerationState {
        if self.0 % 2 == 0 {
            if *self == Generation::new_dead() {
                GenerationState::Dead
            } else {
                GenerationState::Consumed(self.0 >> 1)
            }
        } else {
            if *self == Generation::new_in_progress() {
                GenerationState::InProgress
            } else {
                GenerationState::Produced(self.0 >> 1)
            }
        }
    }

    fn get_generation(&self) -> usize {
        match self.get_state() {
            GenerationState::InProgress | GenerationState::Dead => {
                panic!("Should never be called");
            },
            GenerationState::Consumed(g) => g,
            GenerationState::Produced(g) => g,
        }
    }
}

impl AtomicGeneration {
    //  (AtomicUsize)

    fn load(&self, o: Ordering) -> Generation { Generation(self.0.load(o)) }

    fn store(&self, g: Generation, o: Ordering) { self.0.store(g.0, o) }

    fn compare_and_swap(
        &self,
        current: Generation,
        next: Generation,
        o: Ordering
    ) -> Generation {
        Generation(self.0.compare_and_swap(current.0, next.0, o))
    }
}

impl Index {
    //  (usize)

    fn new(value: usize) -> Index {
        assert!(value != usize::MAX, "Cannot instantiate with empty marker");
        Index(value)
    }
}

impl OptionalIndex {
    //  (usize)

    fn empty() -> OptionalIndex { OptionalIndex(usize::MAX) }

    fn new(value: Index) -> OptionalIndex { OptionalIndex(value.0) }

    fn get(&self) -> Option<Index> {
        match self.0 {
            usize::MAX => None,
            _ => Some(Index(self.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem;

    use super::{QueueRaw};
    use super::{Index, Item, StridedRaw};

    const CACHE_LINE_SIZE: isize = 64;  //  Verified on x86 and x86_64

    #[test]
    fn queue_raw_cache_line_spacing() {
        //  Check that the buffer, read_index and write_index data members of
        //  QueueRaw are each on their own cache lines.

        macro_rules! size_of(
            ($T:ty, $field:ident) => {
                unsafe {
                    let exemplar: $T = mem::uninitialized();
                    let result = mem::size_of_val(&exemplar.$field);
                    mem::forget(exemplar);
                    result
                }
            }
        );

        macro_rules! offset_of(
            ($T:ty, $field:ident) => {
                unsafe {
                    let exemplar: $T = mem::uninitialized();
                    let base: *const u8 = mem::transmute(&exemplar);
                    let attr: *const u8 = mem::transmute(&exemplar.$field);
                    mem::forget(exemplar);
                    (attr as isize) - (base as isize)
                }
            }
        );

        //  Cannot test in general, so we pick a given representation.
        type Q = QueueRaw<usize>;
        type S = StridedRaw<usize>;

        let start_buffer = offset_of!(Q, buffer);
        let size_of_buffer = size_of!(Q, buffer) as isize;
        let end_buffer = start_buffer + size_of_buffer;

        let start_read = offset_of!(Q, read_index);
        let size_of_read = size_of!(Q, read_index) as isize;
        let end_read = start_read + size_of_read;

        let start_write = offset_of!(Q, write_index);

        let desired_spacing = CACHE_LINE_SIZE - size_of_read;

        assert_eq!(desired_spacing, start_read - end_buffer);
        assert_eq!(desired_spacing, start_write - end_read);
    }

    #[test]
    fn strided_raw_items_spacing() {
        //  Check that two consecutive indices yield elements on different cache
        //  lines.

        type S = StridedRaw<u8>; // even with the smallest item!

        let buffer = S::new(0);

        for i in 0..buffer.capacity() {
            let current = buffer.resolve_index(Index(i)) as isize;
            let next = buffer.resolve_index(Index(i+1)) as isize;

            let diff = (current - next).abs();
            let size = mem::size_of::<Item<u8>>() as isize;

            assert!(
                diff * size >= CACHE_LINE_SIZE,
                "Index {} (resolved as {}) and {} (resolved as {}) are only {} bytes (and {} positions) apart, with capacity {} and stride {}",
                i, current,
                i + 1, next,
                diff * size, diff,
                buffer.capacity(), (1_usize << super::LOG_2_STRIDE),
            );
        }
    }
}
