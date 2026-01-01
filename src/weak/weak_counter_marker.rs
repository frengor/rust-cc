use core::cell::Cell;

use crate::utils;

const ACCESSIBLE_MASK: u16 = 1u16 << (u16::BITS - 1);
const COUNTER_MASK: u16 = !ACCESSIBLE_MASK;
const INITIAL_VALUE: u16 = 0;
const INITIAL_VALUE_ACCESSIBLE: u16 = INITIAL_VALUE | ACCESSIBLE_MASK;

// pub(crate) to make it available in tests
pub(crate) const MAX: u16 = !ACCESSIBLE_MASK; // First 15 bits to 1

/// Internal representation:
/// ```text
/// +-----------+-----------+
/// | A: 1 bits | B: 15 bit |  Total: 16 bits
/// +-----------+-----------+
/// ```
///
/// * `A` is `1` when the `CcBox` is accessible (i.e., not deallocated), `0` otherwise
/// * `B` is the weak counter
#[derive(Clone, Debug)]
pub(crate) struct WeakCounterMarker {
    weak_counter: Cell<u16>,
}

pub(crate) struct OverflowError;

impl WeakCounterMarker {
    #[inline]
    #[must_use]
    pub(crate) fn new(accessible: bool) -> WeakCounterMarker {
        WeakCounterMarker {
            weak_counter: Cell::new(if accessible {
                INITIAL_VALUE_ACCESSIBLE
            } else {
                INITIAL_VALUE
            }),
        }
    }

    #[inline]
    pub(crate) fn increment_counter(&self) -> Result<(), OverflowError> {
        if self.counter() == MAX {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            self.weak_counter.set(self.weak_counter.get() + 1);
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn decrement_counter(&self) -> Result<(), OverflowError> {
        if self.counter() == 0 {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            self.weak_counter.set(self.weak_counter.get() - 1);
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn counter(&self) -> u16 {
        self.weak_counter.get() & COUNTER_MASK
    }

    #[inline]
    pub(crate) fn is_accessible(&self) -> bool {
        (self.weak_counter.get() & ACCESSIBLE_MASK) != 0
    }

    #[inline]
    pub(crate) fn set_accessible(&self, accessible: bool) {
        if accessible {
            self.weak_counter.set(self.weak_counter.get() | ACCESSIBLE_MASK);
        } else {
            self.weak_counter.set(self.weak_counter.get() & !ACCESSIBLE_MASK);
        }
    }
}
