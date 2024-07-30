use std::alloc::Layout;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

use test_case::{test_case, test_matrix};

use crate::{CcBox, Mark};
use crate::counter_marker::CounterMarker;
use crate::lists::*;
use crate::state::state;
use crate::utils::cc_dealloc;

fn assert_contains(list: &impl ListMethods, mut elements: Vec<i32>) {
    list.iter().for_each(|ptr| {
        // Test contains
        assert!(list.contains(ptr));

        let elem = unsafe { *ptr.cast::<CcBox<i32>>().as_ref().get_elem() };
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

fn new_list(elements: &[i32], list: &mut impl ListMethods) -> Vec<NonNull<CcBox<i32>>> {
    elements
        .iter()
        .map(|&i| CcBox::new_for_tests(i))
        .inspect(|&ptr| list.add(ptr.cast()))
        .collect()
}

fn deallocate(elements: Vec<NonNull<CcBox<i32>>>) {
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
        state(|state| cc_dealloc(ptr, Layout::new::<CcBox<i32>>(), state));
    });
}

fn check_list(list: &impl ListMethods) {
    let mut iter = list.iter();
    let Some(first) = iter.next() else {
        assert!(list.is_empty());
        list.assert_size(0);
        return;
    };
    let mut real_size = 1; // Already got 1 element from the iterator
    unsafe {
        assert_eq!(*first.as_ref().get_prev(), None);
        let mut prev = first;
        for elem in iter {
            real_size += 1;
            assert_eq!(*elem.as_ref().get_prev(), Some(prev));
            prev = elem;
        }
    }
    list.assert_size(real_size);
}

#[test_case(LinkedList::new())]
#[test_case(PossibleCycles::new())]
fn test_new(mut list: impl ListMethods) {
    assert!(list.is_empty());
    list.assert_size(0);
    assert!(list.first().is_none());
    assert!(list.remove_first().is_none());
}

#[test_case(LinkedList::new())]
#[test_case(PossibleCycles::new())]
fn test_add(mut list: impl ListMethods) {
    let vec: Vec<i32> = vec![0, 1, 2];

    assert!(list.is_empty());
    let elements = new_list(&vec, &mut list);
    assert!(list.first().is_some());

    list.assert_size(vec.len());
    check_list(&list);
    assert_contains(&list, vec);

    drop(list);
    deallocate(elements);
}

#[test_matrix(
    [LinkedList::new(), PossibleCycles::new()],
    [0, 1, 2, 3]
)]
fn test_remove(mut list: impl ListMethods, index: usize) {
    let mut elements = vec![0, 1, 2, 3];
    let vec = new_list(&elements, &mut list);

    list.assert_size(4);

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

    list.assert_size(3);
    check_list(&list);
    assert_contains(&list, elements);
    drop(list);
    deallocate(vec);
}

#[test_case(LinkedList::new())]
#[test_case(PossibleCycles::new())]
fn test_remove_first(mut list: impl ListMethods) {
    let mut elements = vec![0, 1, 2, 3];
    let vec = new_list(&elements, &mut list);

    // Mark to test the removal of the mark
    list.iter().for_each(|ptr| unsafe {
        ptr.as_ref().counter_marker().mark(Mark::Traced)
    });

    // Iterate over the list to get the elements in the correct order as in the list
    elements = list.iter().map(|ptr| unsafe {
        *ptr.cast::<CcBox<i32>>().as_ref().get_elem()
    }).collect();

    for element in elements {
        let removed = list.remove_first().expect("List has smaller size then expected");

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
                *removed.cast::<CcBox<i32>>().as_ref().get_elem(),
                element,
                "Removed wrong element"
            );
            let cm = removed.as_ref().counter_marker();
            assert!(
                cm.is_not_marked() && !cm.is_in_possible_cycles(),
                "Removed element is still marked"
            );
        }

        check_list(&list);
    }

    list.assert_size(0);
    assert!(list.is_empty());
    drop(list);
    deallocate(vec);
}

#[test]
fn test_for_each_clearing_panic() {
    let mut list = LinkedList::new();
    let mut vec = new_list(&[0, 1, 2, 3], &mut list);

    for it in &mut vec {
        unsafe {
            it.as_ref().counter_marker().mark(Mark::PossibleCycles); // Just a random mark
        }
    }

    let res = catch_unwind(AssertUnwindSafe(|| list.into_iter().for_each(|ptr| {
        // Manually set mark for the first CcBox, the others should be handled by List::drop
        unsafe { ptr.as_ref().counter_marker().mark(Mark::NonMarked) };

        panic!("into_iter().for_each panic");
    })));

    assert!(res.is_err(), "Hasn't panicked.");

    for it in vec.iter() {
        fn counter_marker(it: &NonNull<CcBox<i32>>) -> &CounterMarker {
            unsafe { it.as_ref().counter_marker() }
        }

        let counter_marker = counter_marker(it);

        assert!(counter_marker.is_not_marked());
        assert!(!counter_marker.is_in_possible_cycles());
    }

    deallocate(vec);
}

#[test_case(LinkedList::new())]
#[test_case(PossibleCycles::new())]
fn test_list_moving(mut list: impl ListMethods) {
    let cc = CcBox::new_for_tests(5i32);
    list.add(cc.cast());

    let list_moved = list;

    list_moved.iter().for_each(|elem| unsafe {
        assert_eq!(*elem.cast::<CcBox<i32>>().as_ref().get_elem(), 5i32);
    });

    drop(list_moved);

    deallocate(vec![cc]);
}

#[cfg(feature = "finalization")]
#[test]
fn test_mark_self_and_append() {
    let mut list = PossibleCycles::new();
    let mut to_append = LinkedList::new();
    let elements: Vec<i32> = vec![0, 1, 2];
    let elements_to_append: Vec<i32> = vec![3, 4, 5];
    let elements_final: Vec<i32> = vec![0, 1, 2, 3, 4, 5];

    let vec = new_list(&elements, &mut list);
    let vec_to_append = new_list(&elements_to_append, &mut to_append);

    let list_size = list.iter().inspect(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::Traced);
    }).count();
    let to_append_size = to_append.iter().inspect(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::PossibleCycles);
    }).count();

    unsafe {
        list.mark_self_and_append(Mark::PossibleCycles, to_append, to_append_size);
    }

    list.assert_size(list_size + to_append_size);

    check_list(&list);
    assert_contains(&list, elements_final);
    list.iter().for_each(|elem| unsafe {
        assert!(elem.as_ref().counter_marker().is_in_possible_cycles());
    });

    drop(list);
    deallocate(vec);
    deallocate(vec_to_append);
}

#[cfg(feature = "finalization")]
#[test]
fn test_mark_self_and_append_empty_list() {
    let mut list = PossibleCycles::new();
    let to_append = LinkedList::new();
    let elements: Vec<i32> = vec![0, 1, 2];

    let vec = new_list(&elements, &mut list);

    list.iter().for_each(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::Traced);
    });

    unsafe {
        list.mark_self_and_append(Mark::PossibleCycles, to_append, 0);
    }

    list.assert_size(vec.len());

    check_list(&list);
    assert_contains(&list, elements);
    list.iter().for_each(|elem| unsafe {
        assert!(elem.as_ref().counter_marker().is_in_possible_cycles());
    });

    drop(list);
    deallocate(vec);
}

#[cfg(feature = "finalization")]
#[test]
fn test_mark_empty_self_and_append() {
    let list = PossibleCycles::new();
    let mut to_append = LinkedList::new();
    let elements: Vec<i32> = vec![0, 1, 2];

    let vec = new_list(&elements, &mut to_append);

    to_append.iter().for_each(|elem| unsafe {
        elem.as_ref().counter_marker().mark(Mark::PossibleCycles);
    });

    unsafe {
        list.mark_self_and_append(Mark::PossibleCycles, to_append, vec.len());
    }

    list.assert_size(vec.len());

    check_list(&list);
    assert_contains(&list, elements);
    list.iter().for_each(|elem| unsafe {
        assert!(elem.as_ref().counter_marker().is_in_possible_cycles());
    });

    drop(list);
    deallocate(vec);
}

#[cfg(feature = "finalization")]
#[test]
fn test_mark_empty_self_and_append_empty_list() {
    let list = PossibleCycles::new();
    let to_append = LinkedList::new();

    unsafe {
        list.mark_self_and_append(Mark::PossibleCycles, to_append, 0);
    }

    list.assert_size(0);
    assert!(list.is_empty());
}

#[cfg(feature = "finalization")]
#[test]
fn test_swap_list() {
    let mut list = PossibleCycles::new();
    let mut to_swap = LinkedList::new();
    let elements: Vec<i32> = vec![0, 1, 2];
    let elements_to_swap: Vec<i32> = vec![3, 4, 5];

    let vec = new_list(&elements, &mut list);
    let vec_to_swap = new_list(&elements_to_swap, &mut to_swap);

    unsafe {
        list.swap_list(&mut to_swap, elements_to_swap.len());
    }

    list.assert_size(elements_to_swap.len());

    check_list(&list);
    check_list(&to_swap);
    assert_contains(&list, elements_to_swap);
    assert_contains(&to_swap, elements);

    drop(list);
    drop(to_swap);
    deallocate(vec);
    deallocate(vec_to_swap);
}

// Common methods to DRY in list's tests
// Also, mutating methods always take a &mut reference, even for PossibleCycles
trait ListMethods {
    fn first(&self) -> Option<NonNull<CcBox<()>>>;

    fn add(&mut self, ptr: NonNull<CcBox<()>>);

    fn remove(&mut self, ptr: NonNull<CcBox<()>>);

    fn remove_first(&mut self) -> Option<NonNull<CcBox<()>>>;

    fn is_empty(&self) -> bool;

    fn iter(&self) -> Iter;

    fn contains(&self, ptr: NonNull<CcBox<()>>) -> bool;

    fn assert_size(&self, expected_size: usize);
}

impl ListMethods for LinkedList {
    fn first(&self) -> Option<NonNull<CcBox<()>>> {
        self.first()
    }

    fn add(&mut self, ptr: NonNull<CcBox<()>>) {
        self.add(ptr)
    }

    fn remove(&mut self, ptr: NonNull<CcBox<()>>) {
        self.remove(ptr)
    }

    fn remove_first(&mut self) -> Option<NonNull<CcBox<()>>> {
        self.remove_first()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn iter(&self) -> Iter {
        self.iter()
    }

    fn contains(&self, ptr: NonNull<CcBox<()>>) -> bool {
        self.iter().any(|elem| elem == ptr)
    }

    fn assert_size(&self, expected_size: usize) {
        assert_eq!(expected_size, self.iter().count());
    }
}

impl ListMethods for PossibleCycles {
    fn first(&self) -> Option<NonNull<CcBox<()>>> {
        self.first()
    }

    fn add(&mut self, ptr: NonNull<CcBox<()>>) {
        Self::add(self, ptr)
    }

    fn remove(&mut self, ptr: NonNull<CcBox<()>>) {
        Self::remove(self, ptr)
    }

    fn remove_first(&mut self) -> Option<NonNull<CcBox<()>>> {
        Self::remove_first(self)
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn iter(&self) -> Iter {
        self.iter()
    }

    fn contains(&self, ptr: NonNull<CcBox<()>>) -> bool {
        self.contains(ptr)
    }

    fn assert_size(&self, expected_size: usize) {
        assert_eq!(expected_size, self.size());
    }
}
