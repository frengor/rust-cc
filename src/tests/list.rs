use std::alloc::{dealloc, Layout};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

use crate::{CcOnHeap, List, Mark};
use crate::counter_marker::CounterMarker;

fn assert_contains(list: &List, mut elements: Vec<i32>) {
    list.for_each(|ptr| {
        // Test contains
        assert!(list.contains(ptr));

        let elem = unsafe { *ptr.cast::<CcOnHeap<i32>>().as_ref().get_elem() };
        let index = elements.iter().position(|&i| i == elem);
        assert!(index.is_some(), "Couldn't find element {} in list.", elem);
        elements.swap_remove(index.unwrap());
    });

    assert!(
        elements.is_empty(),
        "List does not contains: {:?}",
        elements
    );
}

fn new_list(elements: &[i32], list: &mut List) -> Vec<NonNull<CcOnHeap<i32>>> {
    elements
        .iter()
        .map(|&i| CcOnHeap::new_for_tests(i))
        .inspect(|&ptr| list.add(ptr.cast()))
        .collect()
}

fn deallocate(elements: Vec<NonNull<CcOnHeap<i32>>>) {
    elements.into_iter().for_each(|ptr| unsafe {
        assert!(
            (*ptr.as_ref().get_next()).is_none(),
            "{} has a next",
            *ptr.as_ref().get_elem()
        );
        assert!(
            (*ptr.as_ref().get_prev()).is_none(),
            "{} has a prev",
            *ptr.as_ref().get_elem()
        );
        dealloc(ptr.cast().as_ptr(), Layout::new::<CcOnHeap<i32>>());
    });
}

#[test]
fn test_new() {
    let list = List::new();
    assert!(list.first().is_none());
}

#[test]
fn test_add() {
    let mut list = List::new();

    let vec: Vec<i32> = vec![0, 1, 2];

    assert!(list.first().is_none());
    let elements = new_list(&vec, &mut list);
    assert!(list.first().is_some());

    assert_contains(&list, vec);

    list.for_each_clearing(|_| {}); // Clear the list
    deallocate(elements);
}

#[test]
fn test_remove() {
    fn remove(index: usize) {
        let mut list = List::new();
        let mut elements = vec![0, 1, 2, 3];
        let vec = new_list(&elements, &mut list);

        let removed = vec[index];
        list.remove(removed.cast());
        let removed_i = elements.swap_remove(index);

        unsafe {
            assert!(
                (*removed.as_ref().get_next()).is_none(),
                "Removed element has still a next."
            );
            assert!(
                (*removed.as_ref().get_prev()).is_none(),
                "Removed element has still a prev."
            );
            assert_eq!(
                *removed.as_ref().get_elem(),
                removed_i,
                "Removed wrong element"
            );
        }

        assert_contains(&list, elements);
        list.for_each_clearing(|_| {}); // Clear the list
        deallocate(vec);
    }

    remove(0);
    remove(1);
    remove(2);
    remove(3);
}

#[test]
fn test_for_each_clearing_panic() {
    let mut list = List::new();
    let mut vec = new_list(&[0, 1, 2, 3], &mut list);

    for it in &mut vec {
        unsafe {
            (*it.as_ref().counter_marker()).mark(Mark::TraceCounting); // Just a random mark
        }
    }

    let res = catch_unwind(AssertUnwindSafe(|| list.for_each_clearing(|ptr| {
        // Manually set mark for the first CcOnHeap, the others should be handled by for_each_clearing
        unsafe { (*ptr.as_ref().counter_marker()).mark(Mark::NonMarked) };

        panic!("for_each_clearing panic");
    })));

    assert!(res.is_err(), "Hasn't panicked.");

    for it in vec.iter() {
        fn counter_marker(it: &NonNull<CcOnHeap<i32>>) -> &CounterMarker {
            unsafe { &*it.as_ref().counter_marker() }
        }

        let counter_marker = counter_marker(it);

        assert!(counter_marker.is_valid());
        assert!(counter_marker.is_not_marked());
        assert!(!counter_marker.is_in_possible_cycles());
    }

    deallocate(vec);
}
