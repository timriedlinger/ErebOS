/*
 * A basic round-robin scheduler for cooperative threads.
 * Priorities are not supported.
 *
 * Author: Michael Schoettner, Heinrich Heine University Duesseldorf, 2023-05-15
 *         Fabian Ruhland, Heinrich Heine University Duesseldorf, 2026-01-15
 * License: GPLv3
 */

use crate::allocator;
use crate::device::cpu;
use crate::library::once::Once;
use crate::library::queue::LinkedQueue;
use crate::library::spinlock::Spinlock;
use crate::thread::idle_thread::idle_thread;
use crate::thread::thread::Thread;
use alloc::boxed::Box;
use core::fmt::Display;
use core::{fmt, ptr};

/// Global scheduler instance
static SCHEDULER: Once<Scheduler> = Once::new();

/// Global access to the scheduler.
pub fn scheduler() -> &'static Scheduler {
    SCHEDULER.init(Scheduler::new)
}

/// Unlock the scheduler state.
/// This function is called from assembly code.
/// Usually, the mutex would be unlocked automatically when going out of scope.
/// However, since we switch to a different thread in `yield_cpu()` and `exit()`,
/// the scope is not left and the mutex remains locked.
/// As a workaround, we provide this function to unlock the scheduler manually.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlock_scheduler() {
    unsafe {
        scheduler().state.force_unlock();
    }
}

/// The state of the scheduler.
/// It contains the active thread and the ready queue with all other threads.
/// The state is contained in its own struct so that it can be locked via a mutex.
struct SchedulerState {
    active_thread: Option<Box<Thread>>,
    ready_queue: LinkedQueue<Box<Thread>>,
    terminated_threads: LinkedQueue<Box<Thread>>,
    initialized: bool,
}

/// Represents the scheduler.
/// It is round-robin-based and uses a queue to manage the threads.
pub struct Scheduler {
    state: Spinlock<SchedulerState>,
}

impl Scheduler {
    /// Create a new scheduler instance with an empty ready queue
    /// and an idle thread as the active thread.
    pub fn new() -> Self {
        let state = SchedulerState {
            active_thread: Some(Thread::new_kernel_thread(idle_thread)),
            ready_queue: LinkedQueue::new(),
            terminated_threads: LinkedQueue::new(),
            initialized: false,
        };

        Scheduler {
            state: Spinlock::new(state),
        }
    }

    /// Get the ID of the currently active thread.
    pub fn get_active_tid(&self) -> usize {
        let state = self.state.lock();

        state.active_thread.as_ref().unwrap().id()
    }

    /// Start the scheduler.
    /// This function must only be called once.
    pub fn schedule(&self) {
        let mut state = self.state.lock();

        // Mark the scheduler as initialized.
        state.initialized = true;

        // The active thread is never None, since we must at least have the idle thread.
        state.active_thread.as_mut().unwrap().start();
    }

    /// Register a new thread in the ready queue.
    pub fn ready(&self, thread: Box<Thread>) {
        let mut state = self.state.lock();

        state.ready_queue.enqueue(thread);
    }

    /// Terminate the current (calling) thread and switch to the next one.
    pub fn exit(&self) {
        let mut state = self.state.lock();

        // The active thread is never None, since we must at least have the idle thread.
        let mut current = state.active_thread.take().unwrap();
        let current_ptr = ptr::from_mut(current.as_mut());

        // The idle thread never exits, so there must be at least one thread in the queue.
        let next = state.ready_queue.dequeue().unwrap();

        // Set the dequeued thread as the active thread, overwriting the current one, which we want to exit.
        state.active_thread = Some(next);

        // Enqueue the current thread into the list of terminated threads.
        // Its resources will be freed by the idle thread after the switch to the next thread has finished.
        state.terminated_threads.enqueue(current);

        unsafe {
            // Switch to the next thread.
            // `current` still contains the old thread we want to exit,
            // while `state.active_thread` contains the next one.
            Thread::switch(current_ptr, state.active_thread.as_mut().unwrap().as_mut());
        }
    }

    /// Yield the CPU and switch to the next thread in the ready queue.
    pub fn yield_cpu(&self) {
        // User threads (ring 3) must not switch threads by themselves, because the next thread would
        // continue running in ring 3. This can happen implicitly, e.g. when a user thread waits for
        // a locked spinlock (`Spinlock::lock()` calls `yield_cpu()`). In this case, we just return and
        // let the next timer interrupt (which is handled in ring 0) switch to the next thread.
        if cpu::current_privilege_level() != 0 {
            return;
        }

        if let Some(mut state) = self.state.try_lock() {
            if !state.initialized || allocator::global::is_allocator_locked() {
                return;
            }

            // Check if there is a next thread in the queue.
            // Otherwise, we just return to the current thread.
            if let Some(next) = state.ready_queue.dequeue() {
                // The active thread is never None, since we must at least have the idle thread.
                let mut current = state.active_thread.take().unwrap();
                let current_ptr = ptr::from_mut(current.as_mut());

                // Set the dequeued thread as the active thread and enqueue the old active thread.
                state.active_thread = Some(next);
                state.ready_queue.enqueue(current);

                unsafe {
                    // Switch to the next thread.
                    // We created `current_ptr` earlier to still have access to the old thread after
                    // it has been enqueued. Since we know that the thread object still exists,
                    // and its memory address has not changed, because it is wrapped in a Box,
                    // we can safely use it here.
                    Thread::switch(current_ptr, state.active_thread.as_mut().unwrap().as_mut());
                }
            }
        }
    }

    /// Kill the thread with the given ID by removing it from the ready queue.
    pub fn kill(&self, to_kill_id: usize) {
        // Cannot kill idle thread
        if to_kill_id == 0 {
            panic!("Cannot kill idle thread!");
        }

        // Cannot kill myself
        if to_kill_id == self.get_active_tid() {
            panic!("A thread cannot kill itself!");
        }

        let mut state = self.state.lock();
        state.ready_queue.remove(|thread| thread.id() == to_kill_id);
    }

    /// Clean up the resources of all terminated threads (called by the idle thread).
    /// When a thread exits, its resources are never freed because the scheduler switches
    /// to the next thread in `Scheduler::exit()`, before the terminated thread can go out of scope.
    /// To accommodate for that, terminated threads are gathered in `SchedulerState::terminated_threads`
    /// and dropped at a later point by this function.
    pub fn cleanup_terminated_threads(&self) {
        let mut state = self.state.lock();

        // Dequeue all terminated threads, causing their resources to be freed.
        while let Some(thread) = state.terminated_threads.dequeue() {}
    }
}

impl Display for Scheduler {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let state = self.state.lock();
        let active = state.active_thread.as_ref().unwrap();

        write!(f, "active: {}, ready: {}", active, state.ready_queue)
    }
}
