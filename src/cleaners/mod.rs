use std::cell::RefCell;

use slotmap::{new_key_type, SlotMap};

use crate::{Context, Finalize, Trace};
use crate::weak::{Weak, WeakableCc};

new_key_type! {
    struct CleanerKey;
}

struct CleanerMap {
    map: RefCell<SlotMap<CleanerKey, CleanerFn>>,
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
            fun();
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
                map: RefCell::new(SlotMap::with_capacity_and_key(3)),
            }),
        }
    }

    #[inline]
    pub fn register(&self, cleaner: impl FnOnce() + 'static) -> Cleanable {
        let key = self.cleaner_map.map.borrow_mut().insert(CleanerFn(Some(Box::new(cleaner))));
        Cleanable {
            cleaner_map: self.cleaner_map.downgrade(),
            key,
        }
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
        if let Some(cc) = self.cleaner_map.upgrade() {
            if let Ok(mut map) = cc.map.try_borrow_mut() {
                let _ = map.remove(self.key);
            }
        }
    }
}

unsafe impl Trace for Cleanable {
    #[inline(always)]
    fn trace(&self, _: &mut Context<'_>) {
    }
}

impl Finalize for Cleanable {}
