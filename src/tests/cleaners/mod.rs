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

    let already_cleaned = Rc::new(Cell::new(CleanableState::new()));

    let cleaner = register_cleaner(&already_cleaned, &to_clean.cleaner);

    already_cleaned.get().assert_not_cleaned();

    drop(to_clean); // Should call the clean function

    already_cleaned.get().assert_cleaned();

    cleaner.clean(); // This should be a no-op

    already_cleaned.get().assert_cleaned();
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

    let already_cleaned = Rc::new(Cell::new(CleanableState::new()));

    let cleaner = register_cleaner(&already_cleaned, &to_clean.cleaner);

    already_cleaned.get().assert_not_cleaned();

    cleaner.clean(); // Clean immediately after

    already_cleaned.get().assert_cleaned();

    drop(to_clean); // Should call the clean function

    already_cleaned.get().assert_cleaned();
}

#[derive(Copy, Clone, Default, Debug)]
struct CleanableState {
    cleaned: bool,
    cleaned_twice: bool,
}

impl CleanableState {
    fn new() -> Self {
        Self::default()
    }

    fn assert_cleaned(self) {
        assert!(self.cleaned);
        assert!(!self.cleaned_twice);
    }

    fn assert_not_cleaned(self) {
        assert!(!self.cleaned);
        assert!(!self.cleaned_twice);
    }
}

fn register_cleaner(already_cleaned: &Rc<Cell<CleanableState>>, cleaner: &Cleaner) -> Cleanable {
    assert!(!cleaner.has_allocated());

    let already_cleaned = already_cleaned.clone();
    let cleanable = cleaner.register(move || {
        let mut old = already_cleaned.get();
        if old.cleaned {
            // Already cleaned
            old.cleaned_twice = true;
        } else {
            old.cleaned = true;
        }
        already_cleaned.set(old);
    });

    assert!(cleaner.has_allocated());

    cleanable
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

#[test]
fn simple_panic_on_clean() {
    let cleaner = Cleaner::new();

    let already_cleaned = Rc::new(Cell::new(false));

    let already_cleaned_clone = already_cleaned.clone();
    cleaner.register(move || {
        already_cleaned_clone.set(true);
        panic!("Panic inside registered cleaner!");
    }).clean();

    assert!(already_cleaned.get());
}

#[test]
fn simple_panic_on_cleaner_drop() {
    let cleaner = Cleaner::new();

    let already_cleaned = Rc::new(Cell::new(false));

    let already_cleaned_clone = already_cleaned.clone();
    cleaner.register(move || {
        already_cleaned_clone.set(true);
        panic!("Panic inside registered cleaner!");
    });

    drop(cleaner);

    assert!(already_cleaned.get());
}

#[test]
fn panic_on_clean() {
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

    drop(to_clean);

    assert!(already_cleaned.get());
}
