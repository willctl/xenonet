//! Locks that outlive a panic. std poisons a lock whose holder panicked, and
//! every later `lock().unwrap()` panics too, so one bad request would take
//! the whole service down. The state XenoNet keeps under a lock is plain
//! data that each call leaves whole, as the JS it was ported from did when a
//! request threw, so a later call takes the lock as it is.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// `Mutex::lock`, poisoned or not.
pub trait Locked<T: ?Sized> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `RwLock::read` and `RwLock::write`, poisoned or not.
pub trait ReadWrite<T: ?Sized> {
    fn reading(&self) -> RwLockReadGuard<'_, T>;
    fn writing(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> ReadWrite<T> for RwLock<T> {
    fn reading(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn writing(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_lock_whose_holder_panicked_still_opens() {
        let m = Arc::new(Mutex::new(1));
        let held = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let _g = held.lock().unwrap();
            panic!("the holder panics");
        })
        .join();
        assert!(m.is_poisoned());
        *m.locked() += 1;
        assert_eq!(*m.locked(), 2);

        let rw = Arc::new(RwLock::new(1));
        let held = Arc::clone(&rw);
        let _ = std::thread::spawn(move || {
            let _g = held.write().unwrap();
            panic!("the holder panics");
        })
        .join();
        *rw.writing() += 1;
        assert_eq!(*rw.reading(), 2);
    }
}
