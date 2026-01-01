use core::cell::Cell;

use crate::utils;

const NON_MARKED: u16 = 0u16;
const IN_POSSIBLE_CYCLES: u16 = 1u16 << (u16::BITS - 2);
const IN_LIST: u16 = 2u16 << (u16::BITS - 2);
const IN_QUEUE: u16 = 3u16 << (u16::BITS - 2);

const COUNTER_MASK: u16 = 0b11111111111111u16; // First 14 bits set to 1
const FIRST_BIT_MASK: u16 = 1u16 << (u16::BITS - 1);
const FINALIZED_MASK: u16 = 1u16 << (u16::BITS - 2);
const BITS_MASK: u16 = !COUNTER_MASK;

const INITIAL_VALUE: u16 = 1u16;
const INITIAL_VALUE_TRACING_COUNTER: u16 = INITIAL_VALUE | NON_MARKED;
const INITIAL_VALUE_FINALIZED: u16 = INITIAL_VALUE | FINALIZED_MASK;

// pub(crate) to make it available in tests
pub(crate) const MAX: u16 = COUNTER_MASK - 1;

/// Internal representation:
/// ```text
/// +-----------+------------+ +----------+----------+------------+
/// | A: 2 bits | B: 14 bits | | C: 1 bit | D: 1 bit | E: 14 bits |  Total: 32 bits (16 + 16)
/// +-----------+------------+ +----------+----------+------------+
/// ```
///
/// * `A` has 4 possible states:
///   * `NON_MARKED`
///   * `IN_POSSIBLE_CYCLES`: in `possible_cycles` list (implies `NON_MARKED`)
///   * `IN_LIST`: in `root_list` or `non_root_list`
///   * `IN_QUEUE`: in queue to be traced
/// * `B` is the tracing counter. The max value (the one with every bit set to 1) is reserved
///   and indicates that the allocated value has already been dropped (but not yet deallocated)
/// * `C` is `1` when metadata has been allocated, `0` otherwise
/// * `D` is `1` when the element inside `CcBox` has already been finalized, `0` otherwise
/// * `E` is the reference counter. The max value (the one with every bit set to 1) is reserved and should not be used
#[derive(Clone, Debug)]
pub(crate) struct CounterMarker {
    tracing_counter: Cell<u16>,
    counter: Cell<u16>,
}

pub(crate) struct OverflowError;

impl CounterMarker {
    #[inline]
    #[must_use]
    pub(crate) fn new_with_counter_to_one(already_finalized: bool) -> CounterMarker {
        CounterMarker {
            tracing_counter: Cell::new(INITIAL_VALUE_TRACING_COUNTER),
            counter: Cell::new(if !already_finalized {
                INITIAL_VALUE
            } else {
                INITIAL_VALUE_FINALIZED
            }),
        }
    }

    #[inline]
    pub(crate) fn increment_counter(&self) -> Result<(), OverflowError> {
        debug_assert!(self.counter() != COUNTER_MASK); // Check for reserved value

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
        debug_assert!(self.counter() != COUNTER_MASK); // Check for reserved value

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
        debug_assert!(self.tracing_counter() != COUNTER_MASK); // Check for reserved value

        if self.tracing_counter() == MAX {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            // Increment trace_counter and not counter
            self.tracing_counter.set(self.tracing_counter.get() + 1);
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn _decrement_tracing_counter(&self) -> Result<(), OverflowError> {
        debug_assert!(self.tracing_counter() != COUNTER_MASK); // Check for reserved value

        if self.tracing_counter() == 0 {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            // Decrement trace_counter and not counter
            self.tracing_counter.set(self.tracing_counter.get() - 1);
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn counter(&self) -> u16 {
        let rc = self.counter.get() & COUNTER_MASK;
        debug_assert!(rc != COUNTER_MASK); // Check for reserved value
        rc
    }

    #[inline]
    pub(crate) fn tracing_counter(&self) -> u16 {
        let tc = self.tracing_counter.get() & COUNTER_MASK;
        debug_assert!(tc != COUNTER_MASK); // Check for reserved value
        tc
    }

    #[inline]
    pub(crate) fn reset_tracing_counter(&self) {
        debug_assert!(self.tracing_counter() != COUNTER_MASK); // Check for reserved value
        self.tracing_counter.set(self.tracing_counter.get() & !COUNTER_MASK);
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub(crate) fn needs_finalization(&self) -> bool {
        (self.counter.get() & FINALIZED_MASK) == 0u16
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub(crate) fn set_finalized(&self, finalized: bool) {
        Self::set_bits(&self.counter, finalized, FINALIZED_MASK);
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn has_allocated_for_metadata(&self) -> bool {
        (self.counter.get() & FIRST_BIT_MASK) == FIRST_BIT_MASK
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn set_allocated_for_metadata(&self, allocated_for_metadata: bool) {
        Self::set_bits(&self.counter, allocated_for_metadata, FIRST_BIT_MASK);
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn is_dropped(&self) -> bool {
        (self.tracing_counter.get() & COUNTER_MASK) == COUNTER_MASK
    }

    #[cfg(feature = "weak-ptrs")]
    #[inline]
    pub(crate) fn set_dropped(&self, dropped: bool) {
        Self::set_bits(&self.tracing_counter, dropped, COUNTER_MASK);
    }

    #[inline]
    pub(crate) fn is_not_marked(&self) -> bool {
        // true if (self.counter & BITS_MASK) is equal to 01 or 00,
        // so if the first bit is 0
        (self.tracing_counter.get() & FIRST_BIT_MASK) == 0u16
    }

    #[inline]
    pub(crate) fn is_in_possible_cycles(&self) -> bool {
        (self.tracing_counter.get() & BITS_MASK) == IN_POSSIBLE_CYCLES
    }

    #[inline]
    pub(crate) fn is_in_list(&self) -> bool {
        (self.tracing_counter.get() & BITS_MASK) == IN_LIST
    }

    #[inline]
    pub(crate) fn _is_in_queue(&self) -> bool {
        (self.tracing_counter.get() & BITS_MASK) == IN_QUEUE
    }

    #[inline]
    pub(crate) fn is_in_list_or_queue(&self) -> bool {
        // true if (self.counter & BITS_MASK) is equal to 10 or 11,
        // so if the first bit is 1
        (self.tracing_counter.get() & FIRST_BIT_MASK) == FIRST_BIT_MASK
    }

    #[inline]
    pub(crate) fn mark(&self, new_mark: Mark) {
        self.tracing_counter.set((self.tracing_counter.get() & !BITS_MASK) | (new_mark as u16));
    }

    #[cfg(any(feature = "weak-ptrs", feature = "finalization"))]
    #[inline(always)]
    fn set_bits(cell: &Cell<u16>, value: bool, mask: u16) {
        if value {
            cell.set(cell.get() | mask);
        } else {
            cell.set(cell.get() & !mask);
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(u16)]
pub(crate) enum Mark {
    NonMarked = NON_MARKED,
    PossibleCycles = IN_POSSIBLE_CYCLES,
    InList = IN_LIST,
    InQueue = IN_QUEUE,
}
