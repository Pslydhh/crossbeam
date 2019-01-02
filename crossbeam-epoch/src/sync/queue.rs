use core::mem::{self, ManuallyDrop};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crossbeam_utils::CachePadded;
use {unprotected, Atomic, Guard, Owned, Shared};
use core::marker::PhantomData;
use alloc::sync::mpsc::Sender;

pub struct Queue<T> {
    /// The head of the channel.
    head: CachePadded<Position<T>>,

    /// The tail of the channel.
    tail: CachePadded<Position<T>>,

    /// Indicates that dropping a `Channel<T>` may drop values of type `T`.
    _marker: PhantomData<T>,
}

// Any particular `T` should never be accessed concurrently, so no need for `Sync`.
unsafe impl<T: Send> Sync for Queue<T> {}
unsafe impl<T: Send> Send for Queue<T> {}

const BLOCK_CAP: usize = 32;
/// A slot in a block.
struct Slot<T> {
    /// The message.
    msg: UnsafeCell<ManuallyDrop<T>>,
}

struct Block<T> {
    start_index: usize,

    /// The next block in the linked list.
    next: Atomic<Block<T>>,

    /// Slots for messages.
    slots: [UnsafeCell<Slot<T>>; BLOCK_CAP],
}

struct Position<T> {
    index: AtomicUsize,
    block: Atomic<Block<T>>,
}

impl<T> Block<T> {
    /// Creates an empty block that starts at `start_index`.
    fn new(start_index: usize) -> Block<T> {
        Block {
            start_index,
            slots: unsafe { mem::zeroed() },
            next: Atomic::null(),
        }
    }
}

impl<T> Queue<T> {
    /// Create a new, empty queue.
    pub fn new() -> Queue<T> {
        let queue = Queue {
            head: CachePadded::new(Position {
                index: AtomicUsize::new(0),
                block: Atomic::null(),
            }),
            tail: CachePadded::new(Position {
                index: AtomicUsize::new(0),
                block: Atomic::null(),
            }),
            _marker: PhantomData,
        };

        // Allocate an empty block for the first batch of messages.
        let block = unsafe { Owned::new(Block::new(0)).into_shared(unprotected()) };
        queue.head.block.store(block, Ordering::Relaxed);
        queue.tail.block.store(block, Ordering::Relaxed);

        queue
    }
    
    pub fn drop_bags_per_block(&self, guard: &Guard) {
        let head_ptr = self.head.block.load(Ordering::Relaxed, &guard);
        let head = unsafe { head_ptr.deref() };

        for offset in 0..BLOCK_CAP {
            let slot = unsafe { &*head.slots.get_unchecked(offset).get() };
            
            unsafe {
                let slot = &*head.slots.get_unchecked(offset).get();
                let data = ManuallyDrop::into_inner(slot.msg.get().read());
                drop(data);
            }
        }
        
        let next = head.next.load(Ordering::Relaxed, &guard);
        self.head.block.store(next, Ordering::Relaxed);
        
        unsafe{ 
            drop(head_ptr.into_owned());
        }
        
    }
    
    pub fn drop_all_blocks(&self, guard: &Guard) {
        loop {
            let head_ptr = self.head.block.load(Ordering::Relaxed, &guard);
            let head = unsafe { head_ptr.deref() };
    
            for offset in 0..BLOCK_CAP {
                let slot = unsafe { &*head.slots.get_unchecked(offset).get() };
                
                unsafe {
                    let slot = &*head.slots.get_unchecked(offset).get();
                    let data = ManuallyDrop::into_inner(slot.msg.get().read());
                    drop(data);
                }
            }
            
            let next = head.next.load(Ordering::Relaxed, &guard);
            if next == Shared::null() {
                break;
            }
            self.head.block.store(next, Ordering::Relaxed);
            unsafe{ 
                drop(head_ptr.into_owned());
            }    
        }
    }

    pub fn push(&self, t: T, sender: &Sender<usize>, guard: &Guard) {
        loop {
            let tail_ptr = self.tail.block.load(Ordering::Acquire, &guard);
            let tail = unsafe { tail_ptr.deref() };
            let tail_index = self.tail.index.load(Ordering::Relaxed);

            // Calculate the index of the corresponding slot in the block.
            let offset = tail_index.wrapping_sub(tail.start_index);

            // Advance the current index one slot forward.
            let new_index = tail_index.wrapping_add(1);

            // A closure that installs a block following `tail` in case it hasn't been yet.
            let install_next_block = || {
                let current = tail
                    .next
                    .compare_and_set(
                        Shared::null(),
                        Owned::new(Block::new(tail.start_index.wrapping_add(BLOCK_CAP))),
                        Ordering::AcqRel,
                        &guard,
                    ).unwrap_or_else(|err| err.current);

                let _ =
                    self.tail
                        .block
                        .compare_and_set(tail_ptr, current, Ordering::Release, &guard);
            };

            // If `tail_index` is pointing into `tail`...
            if offset < BLOCK_CAP {
                // Try moving the tail index forward.
                if self
                    .tail
                    .index
                    .compare_exchange_weak(
                        tail_index,
                        new_index,
                        Ordering::SeqCst,
                        Ordering::Relaxed,
                    ).is_ok()
                {
                    
                    unsafe {
                        let slot = tail.slots.get_unchecked(offset).get();
                        (*slot).msg.get().write(ManuallyDrop::new(t));
                    }
                    
                    if offset + 1 == BLOCK_CAP {
                        install_next_block();
                        sender.send(new_index.checked_div(BLOCK_CAP).unwrap());
                        // println!("send block_id: {:?}", new_index.checked_div(BLOCK_CAP).unwrap());
                    }
                    
                    break;
                }
            } else if offset == BLOCK_CAP {
                install_next_block();
            }
        }
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        unsafe {
            let guard = &unprotected();

            self.drop_all_blocks(guard);

            // Destroy the remaining sentinel node.
            let sentinel = self.head.block.load(Ordering::Relaxed, guard);
            drop(sentinel.into_owned());
        }
    }
}

#[cfg(test)]
mod test {

}
