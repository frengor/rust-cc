use alloc::boxed::Box;
use core::cell::RefCell;

use slotmap::{new_key_type, SlotMap};

use crate::{Context, Finalize, Trace};
use crate::weak::{Weak, WeakableCc};

new_key_type! {
    struct CleanerKey;
}

struct CleanerMap {
    map: RefCell<Option<SlotMap<CleanerKey, CleanerFn>>>, // The Option is used to avoid allocating until a cleaner is registered
}

unsafe impl Trace for CleanerMap {
    #[inline]
    fn trace(&self, _: &mut Context<'_>) {
    }
}

impl Finalize for CleanerMap {}

struct CleanerFn(Option<Box<dyn FnOnce() + 'static>>);

impl Drop for CleanerFn {
    fn drop(&mut self) {
        if let Some(fun) = self.0.take() {
            // Catch unwind only on std since catch_unwind isn't present in core
            #[cfg(feature = "std")]
            {
                use std::panic::{AssertUnwindSafe, catch_unwind};

                let _ = catch_unwind(AssertUnwindSafe(|| {
                    fun();
                }));
            }

            #[cfg(not(feature = "std"))]
            {
                fun();
            }
        }
    }
}

pub struct Cleaner {
    cleaner_map: WeakableCc<CleanerMap>,
}

impl Cleaner {
    #[allow(clippy::new_without_default)]
    #[inline]
    pub fn new() -> Cleaner {
        Cleaner {
            cleaner_map: WeakableCc::new_weakable(CleanerMap {
                map: RefCell::new(None),
            }),
        }
    }

    #[inline]
    pub fn register(&self, cleaner: impl FnOnce() + 'static) -> Cleanable {
        let map_key = {
            let map = &mut *self.cleaner_map.map.borrow_mut();

            if map.is_none() {
                *map = Some(SlotMap::with_capacity_and_key(3));
            }

            // The unwrap should never fail and should be optimized out by the compiler
            map.as_mut().unwrap().insert(CleanerFn(Some(Box::new(cleaner))))
        };

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
        // DO NOT TRACE self.cleaner_map, It would be unsound!
        // If self.cleaner_map would be traced here, it would be possible to have cleaners called
        // with a reference to the cleaned object accessible inside the clean function.
        // This would be unsound, since cleaners are called from the Drop implementation of Ccs (see the Trace trait safety section)
        // For example, thw following would work:
        // struct RegisteredCleaner {
        //     cyclic: Cc<CcObjectToClean>,
        // }
        // impl RegisteredCleaner for Clean {
        //     fn clean(&mut self) {
        //         self.cyclic is accessible here!
        //     }
        // }
        // Without tracing self.cleaner_map, the Cc<CcObjectToClean> will simply be leaked.
    }
}

impl Finalize for Cleaner {}

pub struct Cleanable {
    cleaner_map: Weak<CleanerMap>,
    key: CleanerKey,
}

impl Cleanable {
    #[inline]
    pub fn clean(self) {
        // Try upgrading to see if the CleanerMap hasn't been deallocated
        let Some(cc) = self.cleaner_map.upgrade() else { return };

        // Just return in case try_borrow_mut fails or the map is None
        // (the latter shouldn't happen, but better be sure)
        let Ok(mut ref_mut) = cc.map.try_borrow_mut() else { return };
        let Some(map) = &mut *ref_mut else { return };
        map.remove(self.key);
    }
}

unsafe impl Trace for Cleanable {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
    }
}

impl Finalize for Cleanable {}
