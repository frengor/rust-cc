use std::collections::hash_map::Keys;
use std::ptr::NonNull;
use std::slice::Iter;

use rustc_hash::FxHashMap;

use crate::cc::CcOnHeap;

pub(crate) type Nodes<'graph> = Keys<'graph, NonNull<CcOnHeap<()>>, Vec<NonNull<CcOnHeap<()>>>>;
pub(crate) type Edges<'graph> = Iter<'graph, NonNull<CcOnHeap<()>>>;

#[derive(Debug)]
pub(crate) struct Graph {
    edges: FxHashMap<NonNull<CcOnHeap<()>>, Vec<NonNull<CcOnHeap<()>>>>,
}

impl Graph {
    #[inline]
    pub(crate) fn new() -> Graph {
        Graph {
            edges: FxHashMap::default(),
        }
    }

    #[inline]
    pub(crate) fn add_edge(
        &mut self,
        source: NonNull<CcOnHeap<()>>,
        target: NonNull<CcOnHeap<()>>,
    ) {
        self.edges.entry(source).or_insert_with(|| Vec::with_capacity(2)).push(target);
    }

    #[inline]
    pub(crate) fn nodes(&self) -> Nodes {
        self.edges.keys()
    }

    #[inline]
    pub(crate) fn edges(&self, node: NonNull<CcOnHeap<()>>) -> Option<Edges> {
        if let Some(vec) = self.edges.get(&node) {
            let iter = vec.iter();
            Some(iter)
        } else {
            None
        }
    }
}
