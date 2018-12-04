/// Epoch-based garbage collector.
///
/// # Examples
///
/// ```
/// use crossbeam_epoch::Collector;
///
/// let collector = Collector::new();
///
/// let handle = collector.register();
/// drop(collector); // `handle` still works after dropping `collector`
///
/// handle.pin().flush();
/// ```
use alloc::sync::Arc;
use alloc::thread;
use alloc::sync::mpsc::channel;
use alloc::sync::mpsc::Sender;
use alloc::sync::mpsc::Receiver;
use alloc::option::Option;
use core::fmt;

use guard::Guard;
use internal::{Global, Local};

/// An epoch-based garbage collector.
pub struct Collector {
    pub(crate) global: Arc<Global>,
    pub(crate) sender: Sender<usize>,
    pub(crate) receiver: Option<Receiver<usize>>,
}

unsafe impl Send for Collector {}
unsafe impl Sync for Collector {}

impl Collector {
    /// Creates a new collector.
    pub fn new() -> Self {
        let global = Arc::new(Global::new());
        let (sender, receiver) = channel();
        
        Collector {
            global: global,
            sender: sender,
            receiver: Some(receiver),
        }
    }

    /// Registers a new handle for the collector.
    pub fn register(&self) -> LocalHandle {
        Local::register(self)
    }
}

impl Clone for Collector {
    /// Creates another reference to the same garbage collector.
    fn clone(&self) -> Self {
        Collector {
            global: self.global.clone(),
            sender: self.sender.clone(),
            receiver: None,
        }
    }
}

impl fmt::Debug for Collector {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Collector").finish()
    }
}

impl PartialEq for Collector {
    /// Checks if both handles point to the same collector.
    fn eq(&self, rhs: &Collector) -> bool {
        Arc::ptr_eq(&self.global, &rhs.global)
    }
}
impl Eq for Collector {}

/// A handle to a garbage collector.
pub struct LocalHandle {
    pub(crate) local: *const Local,
}

impl LocalHandle {
    /// Pins the handle.
    #[inline]
    pub fn pin(&self) -> Guard {
        unsafe { (*self.local).pin() }
    }

    /// Returns `true` if the handle is pinned.
    #[inline]
    pub fn is_pinned(&self) -> bool {
        unsafe { (*self.local).is_pinned() }
    }

    /// Returns the `Collector` associated with this handle.
    #[inline]
    pub fn collector(&self) -> &Collector {
        unsafe { (*self.local).collector() }
    }
}

impl Drop for LocalHandle {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            Local::release_handle(&*self.local);
        }
    }
}

impl fmt::Debug for LocalHandle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("LocalHandle").finish()
    }
}

#[cfg(test)]
mod tests {

}
