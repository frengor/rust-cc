use core::cell::Cell;

use crate::utils;

const NON_MARKED: u32 = 0u32;
const IN_POSSIBLE_CYCLES: u32 = 1u32 << (u32::BITS - 2);
const TRACED: u32 = 2u32 << (u32::BITS - 2);
#[allow(dead_code)] // Used only in weak ptrs, silence warnings
const DROPPED: u32 = 3u32 << (u32::BITS - 2);

const COUNTER_MASK: u32 = 0b11111111111111u32; // First 14 bits set to 1
const TRACING_COUNTER_MASK: u32 = COUNTER_MASK << 14; // 14 bits set to 1 followed by 14 bits set to 0
const FINALIZED_MASK: u32 = 1u32 << (u32::BITS - 4);
const METADATA_MASK: u32 = 1u32 << (u32::BITS - 3);
const BITS_MASK: u32 = !(COUNTER_MASK | TRACING_COUNTER_MASK | FINALIZED_MASK | METADATA_MASK);
const FIRST_BIT_MASK: u32 = 1u32 << (u32::BITS - 1);

const INITIAL_VALUE: u32 = COUNTER_MASK + 2; // +2 means that tracing counter and counter are both set to 1
const INITIAL_VALUE_FINALIZED: u32 = INITIAL_VALUE | FINALIZED_MASK;

// pub(crate) to make it available in tests
pub(crate) const MAX: u32 = COUNTER_MASK;

/// Internal representation:
/// ```text
/// +-----------+----------+----------+------------+------------+
/// | A: 2 bits | B: 1 bit | C: 1 bit | D: 14 bits | E: 14 bits |  Total: 32 bits
/// +-----------+----------+----------+------------+------------+
/// ```
///
/// * `A` has 4 possible states:
///   * `NON_MARKED`
///   * `IN_POSSIBLE_CYCLES`: in `possible_cycles` list (implies `NON_MARKED`)
///   * `TRACED`: in `root_list` or `non_root_list`
///   * `DROPPED`: allocated value has already been dropped (but not yet deallocated)
/// * `B` is `1` when metadata has been allocated, `0` otherwise
/// * `C` is `1` when the element inside `CcBox` has already been finalized, `0` otherwise
/// * `D` is the tracing counter
/// * `E` is the counter (last one for sum/subtraction efficiency)
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
        self.set_bit(finalized, FINALIZED_MASK);
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn has_allocated_for_metadata(&self) -> bool {
        (self.counter.get() & METADATA_MASK) == METADATA_MASK
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn set_allocated_for_metadata(&self, allocated_for_metadata: bool) {
        self.set_bit(allocated_for_metadata, METADATA_MASK);
    }

    #[cfg(any(feature = "weak-ptrs", feature = "finalization"))]
    #[inline(always)]
    fn set_bit(&self, value: bool, mask: u32) {
        if value {
            self.counter.set(self.counter.get() | mask);
        } else {
            self.counter.set(self.counter.get() & !mask);
        }
    }

    #[inline]
    pub(crate) fn is_not_marked(&self) -> bool {
        // true if (self.counter & BITS_MASK) is equal to 01 or 00,
        // so if the first bit is 0
        (self.counter.get() & FIRST_BIT_MASK) == 0u32
    }

    #[inline]
    pub(crate) fn is_traced(&self) -> bool {
        (self.counter.get() & BITS_MASK) == TRACED
    }

    #[inline]
    pub(crate) fn is_traced_or_dropped(&self) -> bool {
        // true if (self.counter & BITS_MASK) is equal to 10 or 11,
        // so if the first bit is 1
        (self.counter.get() & FIRST_BIT_MASK) == FIRST_BIT_MASK
    }

    #[cfg(any(feature = "weak-ptrs", all(test, feature = "std")))]
    #[inline]
    pub(crate) fn is_dropped(&self) -> bool {
        (self.counter.get() & BITS_MASK) == DROPPED
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
    #[allow(dead_code)] // Used only in weak ptrs, silence warnings
    Dropped = DROPPED,
}
