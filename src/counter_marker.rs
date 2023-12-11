use core::cell::Cell;

use crate::utils;

const NON_MARKED: u32 = 0u32;
const IN_POSSIBLE_CYCLES: u32 = 1u32 << (u32::BITS - 3);
const TRACED: u32 = 2u32 << (u32::BITS - 3);

const COUNTER_MASK: u32 = 0b11111111111111u32; // First 14 bits set to 1
const TRACING_COUNTER_MASK: u32 = COUNTER_MASK << 14; // 14 bits set to 1 followed by 14 bits set to 0
const FINALIZED_MASK: u32 = 1u32 << (u32::BITS - 4);
const BITS_MASK: u32 = !(COUNTER_MASK | TRACING_COUNTER_MASK | FINALIZED_MASK);
const FIRST_TWO_BITS_MASK: u32 = 3u32 << (u32::BITS - 2);

const INITIAL_VALUE: u32 = COUNTER_MASK + 2; // +2 means that tracing counter and counter are both set to 1
const INITIAL_VALUE_FINALIZED: u32 = INITIAL_VALUE | FINALIZED_MASK;

// pub(crate) to make it available in tests
pub(crate) const MAX: u32 = COUNTER_MASK;

/// Internal representation:
/// ```text
/// +-----------+----------+------------+------------+
/// | A: 3 bits | B: 1 bit | C: 14 bits | D: 14 bits |  Total: 32 bits
/// +-----------+----------+------------+------------+
/// ```
///
/// * `A` has 3 possible states:
///   * `NON_MARKED`
///   * `IN_POSSIBLE_CYCLES` (this implies `NON_MARKED`)
///   * `TRACED`
/// * `B` is `1` when the element inside `CcBox` has already been finalized, `0` otherwise
/// * `C` is the tracing counter
/// * `D` is the counter (last one for sum/subtraction efficiency)
#[derive(Clone, Debug)]
#[repr(transparent)]
pub(crate) struct CounterMarker {
    counter: Cell<u32>,
}

pub(crate) struct OverflowError;

impl CounterMarker {
    #[inline]
    #[must_use]
    pub(crate) fn new_with_counter_to_one(already_finalized: bool) -> CounterMarker {
        CounterMarker {
            counter: Cell::new(if !already_finalized {
                INITIAL_VALUE
            } else {
                INITIAL_VALUE_FINALIZED
            }),
        }
    }

    #[inline]
    pub(crate) fn increment_counter(&self) -> Result<(), OverflowError> {
        if self.counter() == MAX {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            self.counter.set(self.counter.get() + 1);
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn decrement_counter(&self) -> Result<(), OverflowError> {
        if self.counter() == 0 {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            self.counter.set(self.counter.get() - 1);
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn increment_tracing_counter(&self) -> Result<(), OverflowError> {
        if self.tracing_counter() == MAX {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            // Increment trace_counter and not counter
            self.counter.set(self.counter.get() + (1u32 << 14));
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn _decrement_tracing_counter(&self) -> Result<(), OverflowError> {
        if self.tracing_counter() == 0 {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            // Decrement trace_counter and not counter
            self.counter.set(self.counter.get() - (1u32 << 14));
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn counter(&self) -> u32 {
        self.counter.get() & COUNTER_MASK
    }

    #[inline]
    pub(crate) fn tracing_counter(&self) -> u32 {
        (self.counter.get() & TRACING_COUNTER_MASK) >> 14
    }

    #[inline]
    pub(crate) fn reset_tracing_counter(&self) {
        self.counter.set(self.counter.get() & !TRACING_COUNTER_MASK);
    }

    #[inline]
    pub(crate) fn is_in_possible_cycles(&self) -> bool {
        (self.counter.get() & BITS_MASK) == IN_POSSIBLE_CYCLES
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub(crate) fn needs_finalization(&self) -> bool {
        (self.counter.get() & FINALIZED_MASK) == 0u32
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub(crate) fn set_finalized(&self, finalized: bool) {
        if finalized {
            self.counter.set(self.counter.get() | FINALIZED_MASK);
        } else {
            self.counter.set(self.counter.get() & !FINALIZED_MASK);
        }
    }

    #[inline]
    pub(crate) fn is_not_marked(&self) -> bool {
        // true if (self.counter & BITS_MASK) is equal to 001 or 000,
        // so if the first two bits are both 0
        (self.counter.get() & FIRST_TWO_BITS_MASK) == 0u32
    }

    #[inline]
    pub(crate) fn is_traced(&self) -> bool {
        (self.counter.get() & BITS_MASK) == TRACED
    }

    #[inline]
    pub(crate) fn mark(&self, new_mark: Mark) {
        self.counter.set((self.counter.get() & !BITS_MASK) | (new_mark as u32));
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(u32)]
pub(crate) enum Mark {
    NonMarked = NON_MARKED,
    PossibleCycles = IN_POSSIBLE_CYCLES,
    Traced = TRACED,
}
