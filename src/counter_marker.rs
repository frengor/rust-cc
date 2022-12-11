use crate::utils;

const NON_MARKED: u32 = 0u32;
const IN_POSSIBLE_CYCLES: u32 = 1u32 << (u32::BITS - 3);
const TRACING_COUNTING_MARKED: u32 = 2u32 << (u32::BITS - 3);
const TRACING_ROOTS_MARKED: u32 = 3u32 << (u32::BITS - 3);
const TRACING_DROPPING_MARKED: u32 = 4u32 << (u32::BITS - 3);
const TRACING_RESURRECTING_MARKED: u32 = 5u32 << (u32::BITS - 3);
const INVALID: u32 = 0b111u32 << (u32::BITS - 3);

const COUNTER_MASK: u32 = 0b11111111111111u32; // First 14 bits set to 1
const TRACING_COUNTER_MASK: u32 = COUNTER_MASK << 14; // 14 bits set to 1 followed by 14 bits set to 0
const FINALIZED_MASK: u32 = 1u32 << (u32::BITS - 4);
const BITS_MASK: u32 = !(COUNTER_MASK | TRACING_COUNTER_MASK | FINALIZED_MASK);
const FIRST_TWO_BITS_MASK: u32 = 3u32 << (u32::BITS - 2);

const INITIAL_VALUE: u32 = COUNTER_MASK + 2; // +2 means that tracing counter and counter are both set to 1

const MAX: u32 = COUNTER_MASK;

/// Internal representation:
/// ```text
/// +-----------+----------+------------+------------+
/// | A: 3 bits | B: 1 bit | C: 14 bits | D: 14 bits |  Total: 32 bits
/// +-----------+----------+------------+------------+
/// ```
///
/// * `A` has 6 possible states:
///   * `NON_MARKED`
///   * `IN_POSSIBLE_CYCLES` (this implies `NON_MARKED`)
///   * `TRACING_COUNTING_MARKED`
///   * `TRACING_ROOT_MARKED`
///   * `TRACING_DROPPING_MARKED`
///   * `INVALID` (`CcOnHeap` is invalid)
/// * `B` is `1` when the element inside `CcOnHeap` has already been finalized, `0` otherwise
/// * `C` is the tracing counter
/// * `D` is the counter (last one for sum/subtraction efficiency)
#[derive(Copy, Clone, Debug)]
#[repr(transparent)]
pub(crate) struct CounterMarker {
    counter: u32,
}

pub(crate) struct OverflowError;

impl CounterMarker {
    #[inline]
    #[must_use]
    pub(crate) fn new_with_counter_to_one() -> CounterMarker {
        CounterMarker {
            counter: INITIAL_VALUE,
        }
    }

    #[inline]
    pub(crate) fn increment_counter(&mut self) -> Result<(), OverflowError> {
        if self.counter() == MAX {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            self.counter += 1;
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn decrement_counter(&mut self) -> Result<(), OverflowError> {
        if self.counter() == 0 {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            self.counter -= 1;
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn increment_tracing_counter(&mut self) -> Result<(), OverflowError> {
        if self.tracing_counter() == MAX {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            // Increment trace_counter and not counter
            self.counter += 1u32 << 14;
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn decrement_tracing_counter(&mut self) -> Result<(), OverflowError> {
        if self.tracing_counter() == 0 {
            utils::cold(); // This branch of the if is rarely taken
            Err(OverflowError)
        } else {
            // Decrement trace_counter and not counter
            self.counter -= 1u32 << 14;
            Ok(())
        }
    }

    #[inline]
    pub(crate) fn counter(&self) -> u32 {
        self.counter & COUNTER_MASK
    }

    #[inline]
    pub(crate) fn tracing_counter(&self) -> u32 {
        (self.counter & TRACING_COUNTER_MASK) >> 14
    }

    #[inline]
    pub(crate) fn reset_tracing_counter(&mut self) {
        self.counter &= !TRACING_COUNTER_MASK;
    }

    #[inline]
    pub(crate) fn is_in_possible_cycles(&self) -> bool {
        (self.counter & BITS_MASK) == IN_POSSIBLE_CYCLES
    }

    #[inline]
    pub(crate) fn needs_finalization(&self) -> bool {
        (self.counter & FINALIZED_MASK) == 0u32
    }

    #[inline]
    pub(crate) fn set_finalized(&mut self, finalized: bool) {
        if finalized {
            self.counter |= FINALIZED_MASK;
        } else {
            self.counter &= !FINALIZED_MASK;
        }
    }

    #[inline]
    pub(crate) fn is_not_marked(&self) -> bool {
        // true if (self.counter & BITS_MASK) is equal to 001 or 000,
        // so if the first two bits are both 0
        (self.counter & FIRST_TWO_BITS_MASK) == 0u32
    }

    /// Returns whether this CounterMarker is traced or not valid (exclusive or).
    #[inline]
    pub(crate) fn is_traced_or_invalid(&self) -> bool {
        // true if (self.counter & BITS_MASK) is equal to 010, 011, 100, 101 or 111,
        // so if the first two bits aren't both 0
        (self.counter & FIRST_TWO_BITS_MASK) != 0u32
    }

    #[inline]
    pub(crate) fn is_marked_trace_counting(&self) -> bool {
        (self.counter & BITS_MASK) == TRACING_COUNTING_MARKED
    }

    #[inline]
    pub(crate) fn is_marked_trace_roots(&self) -> bool {
        (self.counter & BITS_MASK) == TRACING_ROOTS_MARKED
    }

    #[inline]
    pub(crate) fn is_marked_trace_dropping(&self) -> bool {
        (self.counter & BITS_MASK) == TRACING_DROPPING_MARKED
    }

    #[inline]
    pub(crate) fn is_marked_trace_resurrecting(&self) -> bool {
        (self.counter & BITS_MASK) == TRACING_RESURRECTING_MARKED
    }

    #[inline]
    pub(crate) fn is_valid(&self) -> bool {
        (self.counter & BITS_MASK) != INVALID
    }

    #[inline]
    pub(crate) fn mark(&mut self, new_mark: Mark) {
        self.counter = (self.counter & !BITS_MASK) | (new_mark as u32);
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(u32)]
pub(crate) enum Mark {
    NonMarked = NON_MARKED,
    PossibleCycles = IN_POSSIBLE_CYCLES,
    TraceCounting = TRACING_COUNTING_MARKED,
    TraceRoots = TRACING_ROOTS_MARKED,
    TraceDropping = TRACING_DROPPING_MARKED,
    TraceResurrecting = TRACING_RESURRECTING_MARKED,
    Invalid = INVALID,
}
