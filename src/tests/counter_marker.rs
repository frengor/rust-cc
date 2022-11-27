use crate::counter_marker::*;

// This const is used only in a part of test_increment_decrement() which is
// disabled under MIRI (since it is very slow and doesn't really use unsafe).
// Thus, the cfg attribute removes the constant when running under MIRI to
// disable the "unused const" warning
#[cfg(not(miri))]
const MAX: u32 = 0b11111111111111; // 14 ones

#[test]
fn test_new() {
    fn test(counter: CounterMarker) {
        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        assert_eq!(counter.counter(), 1);
        assert_eq!(counter.tracing_counter(), 1);
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(true));
}

#[test]
fn test_is_to_finalize() {
    let counter = CounterMarker::new_with_counter_to_one(true);
    assert!(counter.is_finalizable());

    let counter = CounterMarker::new_with_counter_to_one(false);
    assert!(!counter.is_finalizable());
}

#[test]
fn test_increment_decrement() {
    fn test(mut counter: CounterMarker) {
        fn assert_not_marked(counter: &mut CounterMarker) {
            assert!(counter.is_not_marked());
            assert!(!counter.is_in_possible_cycles());
            assert!(!counter.is_marked_trace_counting());
            assert!(!counter.is_marked_trace_roots());
            assert!(!counter.is_marked_trace_dropping());
            assert!(!counter.is_marked_trace_resurrecting());
            assert!(counter.is_valid());
        }

        assert_not_marked(&mut counter);

        assert_eq!(counter.counter(), 1);

        assert_not_marked(&mut counter);

        assert_eq!(counter.tracing_counter(), 1);

        assert_not_marked(&mut counter);

        assert!(counter.increment_counter().is_ok());

        assert_not_marked(&mut counter);

        assert_eq!(counter.counter(), 2);
        assert_eq!(counter.tracing_counter(), 1);

        assert!(counter.increment_tracing_counter().is_ok());

        assert_not_marked(&mut counter);

        assert_eq!(counter.counter(), 2);
        assert_eq!(counter.tracing_counter(), 2);

        assert!(counter.decrement_counter().is_ok());

        assert_not_marked(&mut counter);

        assert_eq!(counter.counter(), 1);
        assert!(counter.decrement_tracing_counter().is_ok());

        assert_not_marked(&mut counter);

        assert_eq!(counter.counter(), 1);
        assert_eq!(counter.tracing_counter(), 1);

        // Don't run this under MIRI since it slows down tests by a lot. Moreover, there's no
        // unsafe code used in the functions down below, so MIRI isn't really necessary here
        #[cfg(not(miri))]
        {
            while counter.counter() < MAX {
                assert!(counter.increment_counter().is_ok());
            }
            assert!(counter.increment_counter().is_err());

            while counter.tracing_counter() < MAX {
                assert!(counter.increment_tracing_counter().is_ok());
            }
            assert!(counter.increment_tracing_counter().is_err());

            while counter.counter() > 0 {
                assert!(counter.decrement_counter().is_ok());
            }
            assert!(counter.decrement_counter().is_err());

            while counter.tracing_counter() > 0 {
                assert!(counter.decrement_tracing_counter().is_ok());
            }
            assert!(counter.decrement_tracing_counter().is_err());
        }

        assert_not_marked(&mut counter);
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(true));
}

#[test]
fn test_marks() {
    fn test(mut counter: CounterMarker) {
        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::NonMarked);

        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::PossibleCycles);

        assert!(counter.is_not_marked());
        assert!(counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::TraceCounting);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::TraceRoots);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::TraceDropping);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::TraceResurrecting);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::NonMarked);

        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(counter.is_valid());

        counter.mark(Mark::Invalid);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_marked_trace_counting());
        assert!(!counter.is_marked_trace_roots());
        assert!(!counter.is_marked_trace_dropping());
        assert!(!counter.is_marked_trace_resurrecting());
        assert!(!counter.is_valid());
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(true));
}

#[test]
fn test_reset_tracing_counter() {
    fn test(mut counter: CounterMarker) {
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();

        assert_ne!(counter.tracing_counter(), 0);

        counter.reset_tracing_counter();

        assert_eq!(counter.tracing_counter(), 0);
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(true));
}
