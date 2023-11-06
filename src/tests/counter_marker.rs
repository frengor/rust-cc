use crate::counter_marker::*;

#[test]
fn test_new() {
    fn test(counter: CounterMarker) {
        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_traced());

        assert_eq!(counter.counter(), 1);
        assert_eq!(counter.tracing_counter(), 1);
    }

    test(CounterMarker::new_with_counter_to_one());
    test(CounterMarker::new_with_counter_to_one());
}

#[test]
fn test_is_to_finalize() {
    let counter = CounterMarker::new_with_counter_to_one();
    assert!(counter.needs_finalization());

    let counter = CounterMarker::new_with_counter_to_one();
    counter.set_finalized(true);
    assert!(!counter.needs_finalization());

    let counter = CounterMarker::new_with_counter_to_one();
    counter.set_finalized(false);
    assert!(counter.needs_finalization());
}

#[test]
fn test_increment_decrement() {
    fn test(counter: CounterMarker) {
        fn assert_not_marked(counter: &CounterMarker) {
            assert!(counter.is_not_marked());
            assert!(!counter.is_in_possible_cycles());
            assert!(!counter.is_traced());
        }

        assert_not_marked(&counter);

        assert_eq!(counter.counter(), 1);

        assert_not_marked(&counter);

        assert_eq!(counter.tracing_counter(), 1);

        assert_not_marked(&counter);

        assert!(counter.increment_counter().is_ok());

        assert_not_marked(&counter);

        assert_eq!(counter.counter(), 2);
        assert_eq!(counter.tracing_counter(), 1);

        assert!(counter.increment_tracing_counter().is_ok());

        assert_not_marked(&counter);

        assert_eq!(counter.counter(), 2);
        assert_eq!(counter.tracing_counter(), 2);

        assert!(counter.decrement_counter().is_ok());

        assert_not_marked(&counter);

        assert_eq!(counter.counter(), 1);
        assert!(counter._decrement_tracing_counter().is_ok());

        assert_not_marked(&counter);

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
                assert!(counter._decrement_tracing_counter().is_ok());
            }
            assert!(counter._decrement_tracing_counter().is_err());
        }

        assert_not_marked(&counter);
    }

    test(CounterMarker::new_with_counter_to_one());
    test(CounterMarker::new_with_counter_to_one());
}

#[test]
fn test_marks() {
    fn test(counter: CounterMarker) {
        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_traced());

        counter.mark(Mark::NonMarked);

        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_traced());

        counter.mark(Mark::PossibleCycles);

        assert!(counter.is_not_marked());
        assert!(counter.is_in_possible_cycles());
        assert!(!counter.is_traced());

        counter.mark(Mark::Traced);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(counter.is_traced());

        counter.mark(Mark::NonMarked);

        assert!(counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_traced());
    }

    test(CounterMarker::new_with_counter_to_one());
    test(CounterMarker::new_with_counter_to_one());
}

#[test]
fn test_reset_tracing_counter() {
    fn test(counter: CounterMarker) {
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();

        assert_ne!(counter.tracing_counter(), 0);

        counter.reset_tracing_counter();

        assert_eq!(counter.tracing_counter(), 0);
    }

    test(CounterMarker::new_with_counter_to_one());
    test(CounterMarker::new_with_counter_to_one());
}
