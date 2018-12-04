//! The default garbage collector.
//!
//! For each thread, a participant is lazily initialized on its first use, when the current thread
//! is registered in the default collector.  If initialized, the thread's participant will get
//! destructed on thread exit, which in turn unregisters the thread.

use collector::{Collector, LocalHandle};
use guard::Guard;
use alloc::thread;
use alloc::cell::Cell;
use alloc::sync::atomic::Ordering;
use unprotected;

const COLLECT_BLOCKS: usize = 16;

lazy_static! {
    /// The global data for the default garbage collector.
    static ref COLLECTOR: Collector = {
        let mut collector = Collector::new();
        let mut c = collector.clone();
        let receiver = collector.receiver.take().unwrap();
        
        let _ = thread::spawn(move || {

            let handle = collector.register();
            
            // array for accumulate (garbage block).
            let mut array = Vec::new();
            //     
            let mut epoch = 1;
            let block_id = receiver.recv().unwrap();
            // println!("block_id: {:?}", block_id);
            let mut max_block_id = block_id;
            collector.global.epoch.store_epoch(epoch, Ordering::Release);
            array.push( (Cell::new(block_id), epoch) );
            
            // 
            loop {
                epoch = epoch + 1;
                let block_id = receiver.recv().unwrap();
                collector.global.epoch.store_epoch(epoch, Ordering::Release);
                let guard = pin_for_dedicate(Some(&handle));
                
                // if block_id > max_block_id: new slice could be reclaimed at future.
                if block_id > max_block_id {
                    // println!("block_id: {:?}", block_id);
                    array.push( (Cell::new(block_id - max_block_id), epoch) );
                    max_block_id = block_id;
                }
                
                // for reclaim
                if array.len() > 0 && (epoch - array[0].1) > COLLECT_BLOCKS {
                    // try to wait all threads reach epoch. 
                    // this must be fast, because the epoch(array[0].1) has been a long time.....
                    collector.global.try_until_epoch(array[0].1, &guard);
                    let nums = array[0].0.get();
                    for _ in 0..nums {
                        collector.global.drop_bags_per_block(&guard);
                    }
                    let _ = array.remove(0);
                }
            }
        });
        
        c
    };
}

thread_local! {
    /// The per-thread participant for the default garbage collector.
    static HANDLE: LocalHandle = COLLECTOR.register();
}

/// Pins for dedicated thread.
pub fn pin_for_dedicate(option: Option<&LocalHandle>) -> Guard {
    with_handle(|handle| handle.pin(), option)
}

/// Pins the current thread.
#[inline]
pub fn pin() -> Guard {
    with_handle(|handle| handle.pin(), None)
}

/// Returns `true` if the current thread is pinned.
#[inline]
pub fn is_pinned() -> bool {
    with_handle(|handle| handle.is_pinned(), None)
}

/// Returns the default global collector.
pub fn default_collector() -> &'static Collector {
    &COLLECTOR
}

#[inline]
fn with_handle<F, R>(mut f: F, option: Option<&LocalHandle>) -> R
where
    F: FnMut(&LocalHandle) -> R,
{
    match option {
        None => {
            HANDLE
                .try_with(|h| f(h))
                .unwrap_or_else(|_| f(&COLLECTOR.register()))
        }
        Some(local) => {
            f(local)
        }
    }    
}

#[cfg(test)]
mod tests {
    use crossbeam_utils::thread;

    #[test]
    fn pin_while_exiting() {
        struct Foo;

        impl Drop for Foo {
            fn drop(&mut self) {
                // Pin after `HANDLE` has been dropped. This must not panic.
                super::pin();
            }
        }

        thread_local! {
            static FOO: Foo = Foo;
        }

        thread::scope(|scope| {
            scope.spawn(|_| {
                // Initialize `FOO` and then `HANDLE`.
                FOO.with(|_| ());
                super::pin();
                // At thread exit, `HANDLE` gets dropped first and `FOO` second.
            });
        }).unwrap();
    }
}
