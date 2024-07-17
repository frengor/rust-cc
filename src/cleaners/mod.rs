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
//! Usually, [`Cleaner`]s are stored inside a cycle-collected object. Make sure to **never capture** a reference to the containing object
//! inside the cleaning action closure, otherwise the object will be leaked and the cleaning action will never be executed.
//! 
//! # Cleaners vs finalization
//!
//! [`Cleaner`]s provide a faster alternative to [`finalization`][`crate::Finalize`].
//! As such, *when possible* it's suggested to prefer cleaners and disable finalization.

use alloc::boxed::Box;
use core::cell::RefCell;

use slotmap::{new_key_type, SlotMap};

use crate::{Cc, Context, Finalize, Trace};
use crate::weak::Weak;

new_key_type! {
    struct CleanerKey;
}

struct CleanerMap {
    map: RefCell<Option<SlotMap<CleanerKey, CleaningAction>>>, // The Option is used to avoid allocating until a cleaning action is registered
}

unsafe impl Trace for CleanerMap {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
    }
}

impl Finalize for CleanerMap {}

struct CleaningAction(Option<Box<dyn FnOnce() + 'static>>);

impl Drop for CleaningAction {
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
    cleaner_map: Cc<CleanerMap>,
}

impl Cleaner {
    /// Creates a new [`Cleaner`].
    #[allow(clippy::new_without_default)]
    #[inline]
    pub fn new() -> Cleaner {
        Cleaner {
            cleaner_map: Cc::new(CleanerMap {
                map: RefCell::new(None),
            }),
        }
    }

    /// Registers a new cleaning action inside a [`Cleaner`].
    /// 
    /// This method returns a [`Cleanable`], which can be used to manually run the cleaning action.
    ///
    /// # Avoiding memory leaks
    /// Usually, [`Cleaner`]s are stored inside a cycle-collected object. Make sure to **never capture**
    /// a reference to the containing object inside the `action` closure, otherwise the object will
    /// be leaked and the cleaning action will never be executed.
    #[inline]
    pub fn register(&self, action: impl FnOnce() + 'static) -> Cleanable {
        let map_key = self.cleaner_map
            .map
            .borrow_mut()
            .get_or_insert_with(|| SlotMap::with_capacity_and_key(3))
            .insert(CleaningAction(Some(Box::new(action))));

        Cleanable {
            cleaner_map: self.cleaner_map.downgrade(),
            key: map_key,
        }
    }

    #[cfg(all(test, feature = "std"))] // Only used in unit tests
    pub(crate) fn has_allocated(&self) -> bool {
        self.cleaner_map.map.borrow().is_some()
    }
}

unsafe impl Trace for Cleaner {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
        // DO NOT TRACE self.cleaner_map, it would be unsound!
        // If self.cleaner_map would be traced here, it would be possible to have cleaning actions called
        // with a reference to the cleaned object accessible from inside the clean function.
        // This would be unsound, since cleaning actions are called from the Drop implementation of Ccs (see the Trace trait safety section)
    }
}

impl Finalize for Cleaner {}

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
        let Some(cc) = self.cleaner_map.upgrade() else { return };

        // Just return in case try_borrow_mut fails or the map is None
        // (the latter shouldn't happen, but better be sure)
        let Ok(mut ref_mut) = cc.map.try_borrow_mut() else { return };
        let Some(map) = &mut *ref_mut else { return };
        let _ = map.remove(self.key);
    }
}

unsafe impl Trace for Cleanable {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
    }
}

impl Finalize for Cleanable {}
