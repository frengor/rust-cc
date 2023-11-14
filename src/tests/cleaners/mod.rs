use std::cell::Cell;
use std::rc::Rc;
use crate::{Cc, collect_cycles, Context, Finalize, Trace};
use crate::cleaners::{Cleanable, Cleaner};

#[test]
fn clean_after_drop() {
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
    let already_cleaned = already_cleaned.clone();
    cleaner.register(move || {
        assert!(!already_cleaned.get(), "Already cleaned!");
        already_cleaned.set(true);
    })
}

#[test]
fn clean_with_cyclic_cc() {
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

    let cleaner = {
        let cloned = to_clean.clone();
        let cloned_ac = already_cleaned.clone();
        to_clean.cleaner.register(move || {
            let _cc = cloned; // Move to_clone inside the closure
            assert!(!cloned_ac.get(), "Already cleaned!");
            cloned_ac.set(true);
        })
    };

    assert!(!already_cleaned.get());

    drop(to_clean);

    assert!(!already_cleaned.get());

    collect_cycles();

    assert!(!already_cleaned.get());

    cleaner.clean();

    assert!(already_cleaned.get());
}
