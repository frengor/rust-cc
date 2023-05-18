use std::alloc::{dealloc, Layout};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

use crate::{CcOnHeap, List, Mark};
use crate::counter_marker::CounterMarker;

fn assert_contains(list: &List, mut elements: Vec<i32>) {
    list.iter().for_each(|ptr| {
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

fn check_list(list: &List) {
    let mut iter = list.iter();
    let Some(first) = iter.next() else {
        return;
    };
    unsafe {
        assert_eq!(*first.as_ref().get_prev(), None);
        let mut prev = first;
        for elem in iter {
            assert_eq!(*elem.as_ref().get_prev(), Some(prev));
            prev = elem;
        }
    }
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

    check_list(&list);
    assert_contains(&list, vec);

    drop(list);
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

        check_list(&list);
        assert_contains(&list, elements);
        drop(list);
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
            it.as_ref().counter_marker().mark(Mark::PossibleCycles); // Just a random mark
        }
    }

    let res = catch_unwind(AssertUnwindSafe(|| list.into_iter().for_each(|ptr| {
        // Manually set mark for the first CcOnHeap, the others should be handled by List::drop
        unsafe { ptr.as_ref().counter_marker().mark(Mark::NonMarked) };

        panic!("into_iter().for_each panic");
    })));

    assert!(res.is_err(), "Hasn't panicked.");

    for it in vec.iter() {
        fn counter_marker(it: &NonNull<CcOnHeap<i32>>) -> &CounterMarker {
            unsafe { it.as_ref().counter_marker() }
        }

        let counter_marker = counter_marker(it);

        assert!(counter_marker.is_valid());
        assert!(counter_marker.is_not_marked());
        assert!(!counter_marker.is_in_possible_cycles());
    }

    deallocate(vec);
}

#[test]
fn test_list_moving() {
    let mut list = List::new();
    let cc = CcOnHeap::new_for_tests(5i32);
    list.add(cc.cast());

    let list_moved = list;

    list_moved.into_iter().for_each(|elem| unsafe {
        assert_eq!(*elem.cast::<CcOnHeap<i32>>().as_ref().get_elem(), 5i32);
    });

    unsafe {
        dealloc(cc.cast().as_ptr(), Layout::new::<CcOnHeap<i32>>());
    }
}

#[test]
fn test_mark_self_and_append() {
    let mut list = List::new();
    let mut to_append = List::new();
    let elements: Vec<i32> = vec![0, 1, 2];
    let elements_to_append: Vec<i32> = vec![3, 4, 5];
    let elements_final: Vec<i32> = vec![0, 1, 2, 3, 4, 5];

    let vec = new_list(&elements, &mut list);
    let vec_to_append = new_list(&elements_to_append, &mut to_append);

    list.iter().for_each(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::Traced);
    });
    to_append.iter().for_each(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::PossibleCycles);
    });

    list.mark_self_and_append(Mark::PossibleCycles, to_append);

    check_list(&list);
    assert_contains(&list, elements_final);
    list.iter().for_each(|elem| unsafe {
        assert!(elem.as_ref().counter_marker().is_in_possible_cycles());
    });

    drop(list);
    deallocate(vec);
    deallocate(vec_to_append);
}

#[test]
fn test_mark_self_and_append_empty_list() {
    let mut list = List::new();
    let to_append = List::new();
    let elements: Vec<i32> = vec![0, 1, 2];

    let vec = new_list(&elements, &mut list);

    list.iter().for_each(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::Traced);
    });

    list.mark_self_and_append(Mark::PossibleCycles, to_append);

    check_list(&list);
    assert_contains(&list, elements);
    list.iter().for_each(|elem| unsafe {
        assert!(elem.as_ref().counter_marker().is_in_possible_cycles());
    });

    drop(list);
    deallocate(vec);
}

#[test]
fn test_mark_empty_self_and_append() {
    let mut list = List::new();
    let mut to_append = List::new();
    let elements: Vec<i32> = vec![0, 1, 2];

    let vec = new_list(&elements, &mut to_append);

    to_append.iter().for_each(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::PossibleCycles);
    });

    list.mark_self_and_append(Mark::PossibleCycles, to_append);

    check_list(&list);
    assert_contains(&list, elements);
    list.iter().for_each(|elem| unsafe {
        assert!(elem.as_ref().counter_marker().is_in_possible_cycles());
    });

    drop(list);
    deallocate(vec);
}

#[test]
fn test_mark_empty_self_and_append_empty_list() {
    let mut list = List::new();
    let to_append = List::new();

    list.mark_self_and_append(Mark::PossibleCycles, to_append);

    assert!(list.is_empty());
}
