use std::cell::RefCell;
use std::mem::drop;
use std::ops::Deref;
use std::sync::{self, Arc, Mutex};

use once_cell::sync::Lazy;

use shredder::*;

static TEST_MUTEX: Lazy<parking_lot::Mutex<()>> = Lazy::new(|| parking_lot::Mutex::new(()));

#[derive(Debug, Scan)]
struct DirectedGraphNode {
    label: String,
    edges: Vec<Gc<RefCell<DirectedGraphNode>>>,
}

#[test]
fn alloc_u32_gc() {
    let _guard = TEST_MUTEX.lock();
    run_with_gc_cleanup(|| {
        assert_eq!(number_of_tracked_allocations(), 0);

        let val = 7;

        let gc_ptr = Gc::new(val);
        assert_eq!(*gc_ptr.get(), val);
        drop(gc_ptr);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
    });
}

#[test]
fn alloc_directed_graph_node_gc() {
    let _guard = TEST_MUTEX.lock();
    run_with_gc_cleanup(|| {
        assert_eq!(number_of_tracked_allocations(), 0);

        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr = Gc::new(node);
        assert_eq!(gc_ptr.get().label, "A");
        drop(gc_ptr);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
    });
}

#[test]
fn clone_directed_graph_node_gc() {
    let _guard = TEST_MUTEX.lock();
    run_with_gc_cleanup(|| {
        assert_eq!(number_of_tracked_allocations(), 0);
        assert_eq!(number_of_active_handles(), 0);

        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr_one = Gc::new(node);
        let gc_ptr_two = gc_ptr_one.clone();
        assert_eq!(number_of_tracked_allocations(), 1);
        assert_eq!(number_of_active_handles(), 2);

        assert_eq!(gc_ptr_one.get().label, "A");
        assert_eq!(gc_ptr_two.get().label, "A");
        drop(gc_ptr_one);
        drop(gc_ptr_two);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
    });
}

#[test]
fn clone_directed_graph_chain_gc() {
    let _guard = TEST_MUTEX.lock();
    run_with_gc_cleanup(|| {
        assert_eq!(number_of_tracked_allocations(), 0);
        assert_eq!(number_of_active_handles(), 0);

        let node = DirectedGraphNode {
            label: "A".to_string(),
            edges: Vec::new(),
        };

        let gc_ptr_one = Gc::new(RefCell::new(node));
        let gc_ptr_two = gc_ptr_one.clone();
        assert_eq!(number_of_tracked_allocations(), 1);
        assert_eq!(number_of_active_handles(), 2);

        assert_eq!(gc_ptr_one.get().borrow().label, "A");
        assert_eq!(gc_ptr_two.get().borrow().label, "A");

        gc_ptr_two
            .get()
            .deref()
            .borrow_mut()
            .edges
            .push(gc_ptr_one.clone());

        drop(gc_ptr_one);
        drop(gc_ptr_two);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
    });
}

#[derive(Debug, Default, Scan)]
struct Connection {
    connect: Option<Gc<sync::Mutex<Connection>>>,
}

#[test]
fn scan_skip_problem() {
    let _guard = TEST_MUTEX.lock();
    run_with_gc_cleanup(|| {
        assert_eq!(number_of_tracked_allocations(), 0);
        let root_con = Gc::new(sync::Mutex::new(Connection::default()));
        eprintln!("root {:?}", root_con);

        // FIXME: Use shortcut methods here
        let hidden = Gc::new(sync::Mutex::new(Connection::default()));
        eprintln!("hidden {:?}", hidden);
        let hider = Gc::new(sync::Mutex::new(Connection::default()));
        eprintln!("hider {:?}", hider);
        {
            let hidden_clone_1 = hidden.clone();
            eprintln!("hidden clone 1 {:?}", hidden_clone_1);
            root_con.lock().unwrap().connect = Some(hidden_clone_1);
            let hidden_clone_2 = hidden.clone();
            eprintln!("hidden clone 2 {:?}", hidden_clone_2);
            hider.lock().unwrap().connect = Some(hidden_clone_2);
        }
        drop(hidden);
        drop(hider);

        let root_gc_guard = root_con.get();
        let root_blocker = root_gc_guard.lock().unwrap();
        collect();
        assert_eq!(number_of_tracked_allocations(), 2);

        drop(root_blocker);
        drop(root_gc_guard);
        drop(root_con);

        collect();
        assert_eq!(number_of_tracked_allocations(), 0);
    });
}

#[derive(Scan)]
struct Finalizable<'a> {
    #[shredder(skip)]
    tracker: Arc<Mutex<String>>,
    _marker: R<'a, str>,
}

unsafe impl<'a> Finalize for Finalizable<'a> {
    unsafe fn finalize(&mut self) {
        let mut tracker = self.tracker.lock().unwrap();
        *tracker = String::from("finalized");
    }
}

impl<'a> Drop for Finalizable<'a> {
    fn drop(&mut self) {
        let mut tracker = self.tracker.lock().unwrap();
        *tracker = String::from("dropped");
    }
}

#[test]
fn drop_run() {
    let _guard = TEST_MUTEX.lock();
    let tracker = Arc::new(Mutex::new(String::from("none")));
    run_with_gc_cleanup(|| {
        let _to_drop = Gc::new(Finalizable {
            tracker: tracker.clone(),
            _marker: R::new("a static string, safe in drop :)"),
        });
    });
    assert_eq!(&*(tracker.lock().unwrap()), "dropped");
    assert_eq!(number_of_tracked_allocations(), 0);
}

#[test]
fn finalizers_run() {
    let _guard = TEST_MUTEX.lock();
    let tracker = Arc::new(Mutex::new(String::from("none")));
    run_with_gc_cleanup(|| {
        let s = String::from("just a temp string");
        let _to_drop = Gc::new_with_finalizer(Finalizable {
            tracker: tracker.clone(),
            _marker: R::new(&s),
        });
    });
    assert_eq!(&*(tracker.lock().unwrap()), "finalized");
    assert_eq!(number_of_tracked_allocations(), 0);
}

#[test]
fn no_drop_functional() {
    let _guard = TEST_MUTEX.lock();
    let tracker = Arc::new(Mutex::new(String::from("none")));
    run_with_gc_cleanup(|| {
        let s = String::from("just a temp string");
        let _to_drop = Gc::new_no_drop(Finalizable {
            tracker: tracker.clone(),
            _marker: R::new(&s),
        });
    });
    assert_eq!(&*(tracker.lock().unwrap()), "none");
    assert_eq!(number_of_tracked_allocations(), 0);
}
