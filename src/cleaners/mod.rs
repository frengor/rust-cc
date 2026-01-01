//! Execute cleaning actions on object destruction.
//!
//! [`Cleaner`]s can be used to [`register`][`Cleaner::register`] cleaning actions,
//! which are executed when the [`Cleaner`] in which they're registered is dropped.
//!
//! Adding a [`Cleaner`] field to a struct makes it possible to execute cleaning actions on object destruction.
//!
//! # Cleaning actions
//!
//! A cleaning action is provided as a closure to the [`register`][`Cleaner::register`] method, which returns
//! a [`Cleanable`] that can be used to manually run the action.
//!
//! Every cleaning action is executed at maximum once. Thus, any manually-run action will not be executed
//! when their [`Cleaner`] is dropped. The same also applies to cleaning actions run manually after the [`Cleaner`]
//! in which they were registered is dropped, as they have already been executed.
//!
//! # Avoiding memory leaks
//!
//! Usually, [`Cleaner`]s are stored inside a cycle-collected object. Make sure to **never capture** a reference to the container object
//! inside the cleaning action closure, otherwise the object will be leaked and the cleaning action will never be executed.
//!
//! # Cleaners vs finalization
//!
//! [`Cleaner`]s provide a faster alternative to [`finalization`][`crate::Finalize`].
//! As such, *when possible* it's suggested to prefer cleaners and disable finalization.

use alloc::boxed::Box;
use core::cell::{RefCell, UnsafeCell};
use core::fmt::{self, Debug, Formatter};

use slotmap::{SlotMap, new_key_type};

use crate::weak::Weak;
use crate::{Cc, Context, Finalize, Trace};

new_key_type! {
    struct CleanerKey;
}

struct CleanerMap {
    map: RefCell<SlotMap<CleanerKey, CleaningAction>>,
}

unsafe impl Trace for CleanerMap {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {}
}

impl Finalize for CleanerMap {}

struct CleaningAction(Option<Box<dyn FnOnce() + 'static>>);

impl Drop for CleaningAction {
    #[inline]
    fn drop(&mut self) {
        if let Some(fun) = self.0.take() {
            fun();
        }
    }
}

/// A type capable of [`register`][`Cleaner::register`]ing cleaning actions.
///
/// All the cleaning actions registered in a `Cleaner` are run when it is dropped, unless they have been manually executed before.
pub struct Cleaner {
    cleaner_map: UnsafeCell<Option<Cc<CleanerMap>>>, // The Option is used to avoid allocating until a cleaning action is registered
}

impl Cleaner {
    /// Creates a new [`Cleaner`].
    #[inline]
    pub fn new() -> Cleaner {
        Cleaner {
            cleaner_map: UnsafeCell::new(None),
        }
    }

    /// Registers a new cleaning action inside a [`Cleaner`].
    ///
    /// This method returns a [`Cleanable`], which can be used to manually run the cleaning action.
    ///
    /// # Avoiding memory leaks
    /// Usually, [`Cleaner`]s are stored inside a cycle-collected object. Make sure to **never capture**
    /// a reference to the container object inside the `action` closure, otherwise the object will
    /// be leaked and the cleaning action will never be executed.
    #[inline]
    pub fn register(&self, action: impl FnOnce() + 'static) -> Cleanable {
        let cc = {
            // SAFETY: no reference to the Option already exists
            let map = unsafe { &mut *self.cleaner_map.get() };

            map.get_or_insert_with(|| {
                Cc::new(CleanerMap {
                    map: RefCell::new(SlotMap::with_capacity_and_key(3)),
                })
            })
        };

        let map_key = cc.map.borrow_mut().insert(CleaningAction(Some(Box::new(action))));

        Cleanable {
            cleaner_map: cc.downgrade(),
            key: map_key,
        }
    }

    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    pub(crate) fn has_allocated(&self) -> bool {
        // SAFETY: no reference to the Option already exists
        unsafe { (*self.cleaner_map.get()).is_some() }
    }
}

unsafe impl Trace for Cleaner {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
        // DO NOT TRACE self.cleaner_map, it would be unsound!
        // If self.cleaner_map would be traced here, it would be possible to have cleaning actions called
        // with a reference to the cleaned object accessible from inside the cleaning action itself.
        // This would be unsound, since cleaning actions are called from the Drop implementation of Ccs (see the Trace trait safety section)
    }
}

impl Finalize for Cleaner {}

impl Default for Cleaner {
    #[inline]
    fn default() -> Self {
        Cleaner::new()
    }
}

impl Debug for Cleaner {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cleaner").finish_non_exhaustive()
    }
}

/// A `Cleanable` represents a cleaning action registered in a [`Cleaner`].
pub struct Cleanable {
    cleaner_map: Weak<CleanerMap>,
    key: CleanerKey,
}

impl Cleanable {
    /// Executes the cleaning action manually.
    ///
    /// As cleaning actions are never run twice, if it has already been executed then this method will not run it again.
    #[inline]
    pub fn clean(&self) {
        // Try upgrading to see if the CleanerMap hasn't been deallocated
        let Some(cc) = self.cleaner_map.upgrade() else {
            return;
        };

        // Just return in case try_borrow_mut fails
        let Ok(mut map) = cc.map.try_borrow_mut() else {
            crate::utils::cold(); // Should never happen
            return;
        };
        let _ = map.remove(self.key);
    }
}

unsafe impl Trace for Cleanable {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {}
}

impl Finalize for Cleanable {}

impl Debug for Cleanable {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cleanable").finish_non_exhaustive()
    }
}
