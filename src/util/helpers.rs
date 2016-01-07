use std::cell::UnsafeCell;
use std::sync::{Arc, Weak};

pub struct SelfArc<T> {
    weak_ref: UnsafeCell<Option<Weak<T>>>,
}

unsafe impl<T> Sync for SelfArc<T> {
}

impl<T> SelfArc<T> {
    pub fn new() -> SelfArc<T> {
        SelfArc { weak_ref: UnsafeCell::new(None) }
    }

    pub fn init(&self, strong_ref: &Arc<T>) {
        let weak_ref = Some(Arc::downgrade(strong_ref));
        unsafe { *self.weak_ref.get() = weak_ref; }
    }

    pub fn get_weak(&self) -> Weak<T> {
        let weak_ref = unsafe { &*self.weak_ref.get() };
        match *weak_ref {
            Some(ref weak_ref) => weak_ref.clone(),
            None => panic!("Attempt to get an uninitialized weak reference to self"),
        }
    }
}
