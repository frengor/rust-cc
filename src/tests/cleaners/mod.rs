use std::cell::Cell;
#[cfg(not(miri))] // Used by tests run only when not on miri
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use super::reset_state;
use crate::cleaners::{Cleanable, Cleaner};
use crate::{Cc, Context, Finalize, Trace, collect_cycles};

#[test]
fn clean_after_drop() {
    reset_state();

    struct ToClean {
        cleaner: Cleaner,
    }

    unsafe impl Trace for ToClean {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cleaner.trace(ctx);
        }
    }

    impl Finalize for ToClean {}

    let to_clean = Cc::new(ToClean {
        cleaner: Cleaner::new(),
    });

    let already_cleaned = Rc::new(Cell::new(false));

    let cleaner = register_cleaner(&already_cleaned, &to_clean.cleaner);

    assert!(!already_cleaned.get());

    drop(to_clean); // Should call the clean function

    assert!(already_cleaned.get());

    cleaner.clean(); // This should be a no-op

    assert!(already_cleaned.get());
}

#[test]
fn clean_before_drop() {
    reset_state();

    struct ToClean {
        cleaner: Cleaner,
    }

    unsafe impl Trace for ToClean {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cleaner.trace(ctx);
        }
    }

    impl Finalize for ToClean {}

    let to_clean = Cc::new(ToClean {
        cleaner: Cleaner::new(),
    });

    let already_cleaned = Rc::new(Cell::new(false));

    let cleaner = register_cleaner(&already_cleaned, &to_clean.cleaner);

    assert!(!already_cleaned.get());

    cleaner.clean(); // Clean immediately after

    assert!(already_cleaned.get());

    drop(to_clean); // Should call the clean function

    assert!(already_cleaned.get());
}

fn register_cleaner(already_cleaned: &Rc<Cell<bool>>, cleaner: &Cleaner) -> Cleanable {
    assert!(!cleaner.has_allocated());

    let already_cleaned = already_cleaned.clone();
    let cleanable = cleaner.register(move || {
        assert!(!already_cleaned.get(), "Already cleaned!");
        already_cleaned.set(true);
    });

    assert!(cleaner.has_allocated());

    cleanable
}

#[test]
fn clean_with_cyclic_cc() {
    reset_state();

    struct ToClean {
        cleaner: Cleaner,
    }

    unsafe impl Trace for ToClean {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cleaner.trace(ctx);
        }
    }

    impl Finalize for ToClean {}

    let to_clean = Cc::new(ToClean {
        cleaner: Cleaner::new(),
    });

    let already_cleaned = Rc::new(Cell::new(false));

    assert!(!to_clean.cleaner.has_allocated());

    let cleaner = {
        let cloned = to_clean.clone();
        let cloned_ac = already_cleaned.clone();
        to_clean.cleaner.register(move || {
            let _cc = cloned; // Move to_clone inside the closure
            assert!(!cloned_ac.get(), "Already cleaned!");
            cloned_ac.set(true);
        })
    };

    assert!(to_clean.cleaner.has_allocated());

    assert!(!already_cleaned.get());

    drop(to_clean);

    assert!(!already_cleaned.get());

    collect_cycles();

    assert!(!already_cleaned.get());

    cleaner.clean();

    assert!(already_cleaned.get());
}

#[cfg(not(miri))] // Don't run on Miri due to leaks
#[test]
fn simple_panic_on_clean() {
    reset_state();

    let cleaner = Cleaner::new();

    let already_cleaned = Rc::new(Cell::new(false));

    let already_cleaned_clone = already_cleaned.clone();
    let cleanable = cleaner.register(move || {
        already_cleaned_clone.set(true);
        panic!("Panic inside registered cleaner!");
    });

    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            cleanable.clean();
        }))
        .is_err()
    );

    assert!(already_cleaned.get());
}

#[cfg(not(miri))] // Don't run on Miri due to leaks
#[test]
fn simple_panic_on_cleaner_drop() {
    reset_state();

    let cleaner = Cleaner::new();

    let already_cleaned = Rc::new(Cell::new(false));

    let already_cleaned_clone = already_cleaned.clone();
    cleaner.register(move || {
        already_cleaned_clone.set(true);
        panic!("Panic inside registered cleaner!");
    });

    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            drop(cleaner);
        }))
        .is_err()
    );

    assert!(already_cleaned.get());
}

#[cfg(not(miri))] // Don't run on Miri due to leaks
#[test]
fn panic_on_clean() {
    reset_state();

    struct ToClean {
        cleaner: Cleaner,
    }

    unsafe impl Trace for ToClean {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cleaner.trace(ctx);
        }
    }

    impl Finalize for ToClean {}

    let to_clean = Cc::new(ToClean {
        cleaner: Cleaner::new(),
    });

    let already_cleaned = Rc::new(Cell::new(false));

    let already_cleaned_clone = already_cleaned.clone();
    to_clean.cleaner.register(move || {
        already_cleaned_clone.set(true);
        panic!("Panic inside registered cleaner!");
    });

    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            drop(to_clean);
        }))
        .is_err()
    );

    assert!(already_cleaned.get());
}

#[test]
fn clean_multiple_times() {
    reset_state();

    let rc = Rc::new(Cell::new(false));

    let cleaner = Cleaner::new();

    let cleanable = cleaner.register({
        let rc = rc.clone();
        move || {
            assert!(!rc.replace(true));
        }
    });

    cleanable.clean();

    assert!(rc.get());

    cleanable.clean();

    assert!(rc.get());
}
