/*
 * A simple spinlock implementation for mutual exclusion in a multithreaded environment.
 *
 * Author: Michael Schoetter, Heinrich Heine University Duesseldorf, 2024-06-13
 *         Fabian Ruhland, Heinrich Heine University Duesseldorf, 2026-01-07
 * License: GPLv3
 */
use crate::thread::scheduler::scheduler;
use core::arch::asm;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A simple spinlock implementation that spins in a loop until it acquires the lock.
/// It uses an atomic boolean to represent the lock state.
pub struct Spinlock<T> {
    /// The lock is represented by an atomic boolean that indicates whether the lock is held.
    lock: AtomicBool,
    /// The data protected by the spinlock, stored in an `UnsafeCell` to allow mutable access.
    /// We need to use `UnsafeCell` because we want to allow mutable access to the data from
    /// a const reference in `MutexGuard`. This effectively bypasses Rust's borrowing rules,
    /// but the `Spinlock` itself ensures that only one thread can access the data at a time.
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for Spinlock<T> where T: Send {}
unsafe impl<T> Send for Spinlock<T> where T: Send {}

impl<T> Spinlock<T> {
    /// Create a new `Spinlock` protecting the given data.
    pub const fn new(data: T) -> Self {
        Spinlock {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// Try to acquire the lock once without blocking.
    pub fn try_lock(&'_ self) -> Option<SpinlockGuard<'_, T>> {
        if !self.lock.swap(true, Ordering::Acquire) {
            Some(SpinlockGuard { lock: self })
        } else {
            None
        }
    }

    /// Spin until the lock is acquired, then return a guard that allows access to the data.
    pub fn lock(&'_ self) -> SpinlockGuard<'_, T> {
        while self.lock.swap(true, Ordering::Acquire) {
            scheduler().yield_cpu();

            unsafe {
                // Hint to the CPU that we are spinning.
                // This reduces power consumption and improves performance
                asm!("pause");
            }
        }

        SpinlockGuard { lock: self }
    }

    /// Unlock the spinlock, allowing other threads to acquire it.
    /// This is called automatically when the `SpinlockGuard` is dropped
    /// and thus is not publicly accessible.
    fn unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// Check if the spinlock is currently locked.
    pub fn is_locked(&self) -> bool {
        self.lock.load(Ordering::Relaxed)
    }

    /// Forcefully unlock the spinlock. This should only be used in exceptional cases.
    pub unsafe fn force_unlock(&self) {
        self.unlock();
    }

    /// Get a reference to the inner data without locking.
    /// This is unsafe because it can lead to data races if the spinlock is not held
    /// and should only be used in exceptional cases.
    pub unsafe fn inner(&self) -> &T {
        unsafe { self.data.get().as_ref().unwrap() }
    }
}

/// A guard that provides access to the data protected by the spinlock.
/// It implements `Deref` and `DerefMut` to allow transparent access to the data.
pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
}

impl<'a, T> Deref for SpinlockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.lock.data.get().as_ref().unwrap() }
    }
}

impl<'a, T> DerefMut for SpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.lock.data.get().as_mut().unwrap() }
    }
}

impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}
