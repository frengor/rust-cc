use crate::counter_marker::*;

fn assert_not_marked(counter: &CounterMarker) {
    assert!(counter.is_not_marked());
    assert!(!counter.is_in_possible_cycles());
    assert!(!counter.is_in_list());
    assert!(!counter._is_in_queue());
    assert!(!counter.is_in_list_or_queue());
    assert!(!counter.is_in_list_or_queue());
}

fn assert_default_settings(_counter: &CounterMarker) {
    #[cfg(feature = "finalization")]
    assert!(_counter.needs_finalization());

    #[cfg(feature = "weak-ptrs")]
    {
        assert!(!_counter.has_allocated_for_metadata());
        assert!(!_counter.is_dropped());
    }
}

#[test]
fn test_new() {
    fn test(counter: CounterMarker) {
        assert_not_marked(&counter);
        assert_default_settings(&counter);

        assert_eq!(counter.counter(), 1);
        assert_eq!(counter.tracing_counter(), 1);
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(false));
}

#[cfg(feature = "finalization")]
#[test]
fn test_is_to_finalize() {
    fn assert_not_marked_fin(counter: &CounterMarker) {
        assert_not_marked(counter);
        #[cfg(feature = "weak-ptrs")]
        {
            assert!(!counter.has_allocated_for_metadata());
            assert!(!counter.is_dropped());
        }
    }

    fn test(already_fin: bool) {
        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_fin(&counter);
        assert_eq!(!already_fin, counter.needs_finalization());

        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_fin(&counter);
        counter.set_finalized(true);
        assert!(!counter.needs_finalization());

        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_fin(&counter);
        counter.set_finalized(false);
        assert!(counter.needs_finalization());
    }

    test(true);
    test(false);
}

#[cfg(feature = "weak-ptrs")]
#[test]
fn test_weak_ptrs_exists() {
    fn assert_not_marked_weak_ptrs(counter: &CounterMarker, _already_fin: bool) {
        assert_not_marked(counter);

        assert!(!counter.is_dropped());

        #[cfg(feature = "finalization")]
        assert_eq!(!_already_fin, counter.needs_finalization());
    }

    fn test(already_fin: bool) {
        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_weak_ptrs(&counter, already_fin);
        assert!(!counter.has_allocated_for_metadata());

        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_weak_ptrs(&counter, already_fin);
        counter.set_allocated_for_metadata(true);
        assert!(counter.has_allocated_for_metadata());

        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_weak_ptrs(&counter, already_fin);
        counter.set_allocated_for_metadata(false);
        assert!(!counter.has_allocated_for_metadata());
    }

    test(true);
    test(false);
}

#[cfg(feature = "weak-ptrs")]
#[test]
fn test_dropped() {
    fn assert_not_marked_dropped(counter: &CounterMarker, _already_fin: bool) {
        assert_not_marked(counter);

        assert!(!counter.has_allocated_for_metadata());

        #[cfg(feature = "finalization")]
        assert_eq!(!_already_fin, counter.needs_finalization());
    }

    fn test(already_fin: bool) {
        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_dropped(&counter, already_fin);
        assert!(!counter.is_dropped());

        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_dropped(&counter, already_fin);
        counter.set_dropped(true);
        assert!(counter.is_dropped());

        let counter = CounterMarker::new_with_counter_to_one(already_fin);
        assert_not_marked_dropped(&counter, already_fin);
        counter.set_dropped(false);
        assert!(!counter.is_dropped());
    }

    test(true);
    test(false);
}

#[test]
fn test_increment_decrement() {
    fn test(counter: CounterMarker) {
        assert_not_marked(&counter);
        assert_default_settings(&counter);

        assert_eq!(counter.counter(), 1);

        assert_not_marked(&counter);
        assert_default_settings(&counter);

        assert_eq!(counter.tracing_counter(), 1);

        assert_not_marked(&counter);
        assert_default_settings(&counter);

        assert!(counter.increment_counter().is_ok());

        assert_not_marked(&counter);
        assert_default_settings(&counter);

        assert_eq!(counter.counter(), 2);
        assert_eq!(counter.tracing_counter(), 1);

        assert!(counter.increment_tracing_counter().is_ok());

        assert_not_marked(&counter);
        assert_default_settings(&counter);

        assert_eq!(counter.counter(), 2);
        assert_eq!(counter.tracing_counter(), 2);

        assert!(counter.decrement_counter().is_ok());

        assert_not_marked(&counter);
        assert_default_settings(&counter);

        assert_eq!(counter.counter(), 1);
        assert!(counter._decrement_tracing_counter().is_ok());

        assert_not_marked(&counter);
        assert_default_settings(&counter);

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
        assert_default_settings(&counter);
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(false));
}

#[test]
fn test_marks() {
    fn test(counter: CounterMarker) {
        assert_not_marked(&counter);
        assert_default_settings(&counter);

        counter.mark(Mark::NonMarked);

        assert_not_marked(&counter);
        assert_default_settings(&counter);

        counter.mark(Mark::PossibleCycles);

        assert!(counter.is_not_marked());
        assert!(counter.is_in_possible_cycles());
        assert!(!counter.is_in_list());
        assert!(!counter._is_in_queue());
        assert!(!counter.is_in_list_or_queue());
        assert_default_settings(&counter);

        counter.mark(Mark::InList);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(counter.is_in_list());
        assert!(!counter._is_in_queue());
        assert!(counter.is_in_list_or_queue());
        assert_default_settings(&counter);

        counter.mark(Mark::InQueue);

        assert!(!counter.is_not_marked());
        assert!(!counter.is_in_possible_cycles());
        assert!(!counter.is_in_list());
        assert!(counter._is_in_queue());
        assert!(counter.is_in_list_or_queue());
        assert_default_settings(&counter);

        counter.mark(Mark::NonMarked);

        assert_not_marked(&counter);
        assert_default_settings(&counter);
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(false));
}

#[test]
fn test_reset_tracing_counter() {
    fn test(counter: CounterMarker) {
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();
        let _ = counter.increment_tracing_counter();

        assert_ne!(counter.tracing_counter(), 0);
        assert_default_settings(&counter);

        counter.reset_tracing_counter();

        assert_eq!(counter.tracing_counter(), 0);
        assert_default_settings(&counter);
    }

    test(CounterMarker::new_with_counter_to_one(false));
    test(CounterMarker::new_with_counter_to_one(false));
}
