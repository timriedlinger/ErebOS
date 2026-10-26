/*
 * Contains functions to create, start, switch and end threads.
 * Threads can either run in kernel mode (ring 0) or in user mode (ring 3).
 *
 * Author: Michael Schoettner, Heinrich Heine University Duesseldorf, 2023-05-15
 *         Fabian Ruhland, Heinrich Heine University Duesseldorf, 2026-01-15
 * License: GPLv3
 */

use crate::consts::STACK_SIZE;
use crate::device::cpu;
use crate::thread::scheduler::scheduler;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::naked_asm;
use core::fmt::Display;
use core::sync::atomic::AtomicUsize;
use core::{fmt, ptr};

static THREAD_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn next_id() -> usize {
    THREAD_ID_COUNTER.fetch_add(1, core::sync::atomic::Ordering::SeqCst)
}

/// Segment selector for the user code segment (4th entry in the GDT -> 3 * 8, RPL = 3), see `gdt` in boot.asm.
const USER_CODE_SELECTOR: u64 = (3 * 8) | 3; // 0x1b
/// Segment selector for the user data segment (5th entry in the GDT -> 4 * 8, RPL = 3), see `gdt` in boot.asm.
const USER_DATA_SELECTOR: u64 = (4 * 8) | 3; // 0x23
/// RFLAGS for user threads: Interrupts enabled (IF, bit 9) and IOPL = 0 (no I/O-port access in ring 3).
/// Bit 1 is reserved and must always be set.
const USER_RFLAGS: u64 = 0x202;
/// Safety margin (in stack entries) between the top of the kernel stack and the stack frame built in
/// `switch_to_usermode()`. 128 entries * 8 bytes = 1 KiB.
const USER_FRAME_SAFETY_MARGIN: usize = 128;

unsafe extern "C" {
    /// Set the rsp0 entry in the TSS (implemented in boot.asm).
    /// This is the stack pointer the CPU loads, when an interrupt occurs while running in ring 3.
    fn tss_set_rsp0(rsp0: usize);
}

/// Low-level routine for starting a thread.
#[unsafe(naked)]
unsafe extern "C" fn thread_start(stack_ptr: usize) {
    naked_asm!(
        "mov rsp, rdi", // Switch stack
        "call unlock_scheduler", // Unlock scheduler
        "popf",         // Pop rflags
        "pop rbp",      // Pop all other registers
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "ret" // Return to 'kickoff_kernel_thread'
    )
}

/// Low-level routine for switching to the next thread.
/// `current_stack_ptr` is a pointer to `stack_ptr` of the current thread (where the rsp is saved).
/// `next_stack` is the value of `stack_ptr` of the next thread (the new rsp value).
/// `next_stack_end` points to the top of the next thread's kernel stack (used to set the rsp0 entry of the TSS)
#[unsafe(naked)]
unsafe extern "C" fn thread_switch(current_stack_ptr: *mut usize, next_stack: usize, next_stack_end: usize) {
    naked_asm!(
        // Save all registers of the current thread on its stack
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "pushf",
        // Save stackpointer in 'current_stack_ptr' (first parameter)
        "mov [rdi], rsp",
        // Set rsp0 in the TSS to the top of the next thread's kernel stack (third parameter).
        // If the next thread runs in user mode and gets interrupted, the CPU switches to this stack.
        // RSI is a caller-saved register, so we save it on the stack during the call.
        "push rsi",
        "mov rdi, rdx",
        "call tss_set_rsp0",
        "pop rsi",
        // Switch stack to 'next_stack' (second parameter)
        "mov rsp, rsi",
        // Unlock scheduler
        "call unlock_scheduler",
        // Load all registers of the next thread
        "popf",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "ret"
    )
}

#[unsafe(naked)]
/// Called by `switch_to_usermode()`.
/// Switches to the prepared user stack and uses `iret` to return to ring 3 code.
unsafe extern "C" fn thread_user_start(stack_ptr: usize) {
    naked_asm!(
        "mov rsp, rdi", // Switch stack
        "pop rdi",
        "iretq" // Return to user mode
    )
}

/// Represents a thread in the system.
/// It contains the stacks and the entry function.
/// Threads must be registered in the scheduler and are run automatically once the scheduler is started.
#[repr(C)]
pub struct Thread {
    id: usize,
    is_kernel_thread: bool,
    kernel_stack: Vec<u64>, // Memory for the kernel stack
    user_stack: Vec<u64>,   // Memory for the user stack
    stack_ptr: usize,       // Pointer on the stack to the saved context
    entry: fn(),
}

impl Thread {
    /// Create a new kernel thread (running in ring 0) with the given entry function.
    pub fn new_kernel_thread(entry: fn()) -> Box<Thread> {
        Thread::new(entry, true)
    }

    /// Create a new user thread (running in ring 3) with the given entry function.
    pub fn new_user_thread(entry: fn()) -> Box<Thread> {
        Thread::new(entry, false)
    }

    /// Create a new thread with the given entry function.
    /// For the sake of simplicity, each thread gets a kernel and a user stack (even pure kernel threads).
    /// Every thread starts in kernel mode via `kickoff_kernel_thread()`.
    /// User threads switch to user mode afterward (see `switch_to_usermode()`).
    fn new(entry: fn(), is_kernel_thread: bool) -> Box<Thread> {
        // Allocate memory for the stacks and initialize them to zero
        let kernel_stack = vec![0u64; STACK_SIZE / 8];
        let user_stack = vec![0u64; STACK_SIZE / 8];

        // Set the stack pointer to the top of the kernel stack
        let stack_ptr = ptr::from_ref(&kernel_stack[kernel_stack.len() - 1]) as usize;

        // Create a new thread object
        let mut thread = Box::new(Thread {
            id: next_id(),
            is_kernel_thread,
            kernel_stack,
            user_stack,
            stack_ptr,
            entry,
        });

        // Prepare the kernel stack for the thread so it can be started via `thread_start()`
        thread.prepare_kernel_stack();
        thread
    }

    /// Start the thread.
    /// This function is only once by the scheduler.
    /// The scheduler does further thread switching via `switch()`.
    pub fn start(&mut self) {
        unsafe {
            thread_start(self.stack_ptr);
        }
    }

    /// Switch from the `current` thread to the `next` thread.
    /// This function is called by the scheduler to switch between threads.
    pub unsafe fn switch(current: *mut Thread, next: *mut Thread) {
        unsafe {
            let next_stack_end = Self::get_top_of_stack(&(*next).kernel_stack) as usize;
            thread_switch(&mut (*current).stack_ptr, (*next).stack_ptr, next_stack_end);
        }
    }

    /// Get the ID of the thread.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Prepare the kernel stack of a newly created thread in a way that it can be used
    /// to return to the 'kickoff_kernel_thread' function with the thread itself as parameter.
    /// The prepared stack is used in 'thread_start' to start the first thread.
    /// Other threads are started by 'thread_switch' with the prepared stack.
    fn prepare_kernel_stack(&mut self) {
        let kickoff = (Thread::kickoff_kernel_thread as *const ()) as u64;
        let thread = ptr::from_mut(self) as u64;
        let length = self.kernel_stack.len();

        self.kernel_stack[length - 1] = kickoff; // Address of 'kickoff_kernel_thread'
        self.kernel_stack[length - 2] = 0; // r8
        self.kernel_stack[length - 3] = 0; // r9
        self.kernel_stack[length - 4] = 0; // r10
        self.kernel_stack[length - 5] = 0; // r11
        self.kernel_stack[length - 6] = 0; // r12
        self.kernel_stack[length - 7] = 0; // r13
        self.kernel_stack[length - 8] = 0; // r14
        self.kernel_stack[length - 9] = 0; // r15
        self.kernel_stack[length - 10] = 0; // rax
        self.kernel_stack[length - 11] = 0; // rbx
        self.kernel_stack[length - 12] = 0; // rcx
        self.kernel_stack[length - 13] = 0; // rdx
        self.kernel_stack[length - 14] = 0; // rsi
        self.kernel_stack[length - 15] = thread; // rdi -> First parameter for 'kickoff_kernel_thread'
        self.kernel_stack[length - 16] = 0; // rbp
        self.kernel_stack[length - 17] = 0x2; // rflags (IE = 0); interrupts disabled

        self.stack_ptr = self.stack_ptr - (size_of::<u64>() * 16);
    }

    /// Switch this thread from Ring 0 to Ring 3.
    /// For this, the kernel stack is prepared in a way that an 'iretq' instruction
    /// switches to user mode (Ring 3) and the user stack is used. If this function works correctly,
    /// the thread continues in user mode in the function 'kickoff_user_thread'.
    fn switch_to_usermode(&mut self) {
        // We are currently running on this very kernel stack (near its top).
        // To avoid overwriting our own stack frames, the new stack frame is built below a safety margin.
        let frame_end = self.kernel_stack.len() - USER_FRAME_SAFETY_MARGIN;
        let frame_start = frame_end - 6;

        let kickoff = (Thread::kickoff_user_thread as *const ()) as u64;
        let thread = ptr::from_mut(self) as u64;
        let user_stack_end = Self::get_top_of_stack(&self.user_stack) as u64;

        // Build a stack frame, that looks exactly like the CPU created it for an interrupt
        // with privilege level change (without error code), so that 'iretq' can "return" into ring 3.
        // The first entry is not part of the interrupt stack frame. It is popped into rdi by 'thread_user_start',
        // so that it becomes the first parameter for 'kickoff_user_thread'.
        self.kernel_stack[frame_start] = thread; //                  rdi    -> First parameter for 'kickoff_user_thread'
        self.kernel_stack[frame_start + 1] = kickoff; //             rip    -> Address of 'kickoff_user_thread'
        self.kernel_stack[frame_start + 2] = USER_CODE_SELECTOR; //  cs     -> User code segment (ring 3)
        self.kernel_stack[frame_start + 3] = USER_RFLAGS; //         rflags -> Interrupts enabled, IOPL = 0
        self.kernel_stack[frame_start + 4] = user_stack_end; //      rsp    -> Top of the user stack
        self.kernel_stack[frame_start + 5] = USER_DATA_SELECTOR; //  ss     -> User data segment (ring 3)

        unsafe {
            let frame = ptr::from_ref(&self.kernel_stack[frame_start]) as usize;
            thread_user_start(frame);
        }
    }

    /// Called indirectly by using the prepared stack in 'thread_start' and 'thread_switch'.
    fn kickoff_kernel_thread(&mut self) {
        // Set TSS rsp0 to the top of the kernel stack of this thread
        unsafe {
            let rsp0 = Self::get_top_of_stack(&self.kernel_stack);
            tss_set_rsp0(rsp0 as usize);
        }

        if self.is_kernel_thread {
            cpu::enable_int(); // Interrupts are disabled during thread start
            ((*self).entry)();
        } else {
            // Interrupts are enabled by 'iretq' (USER_RFLAGS), so we do not enable them here.
            self.switch_to_usermode();
        }

        scheduler().exit();
    }

    /// Called indirectly by using the prepared stack in 'switch_to_usermode'.
    /// This function runs in user mode (ring 3).
    fn kickoff_user_thread(&self) {
        ((*self).entry)();

        // User threads may currently not exit, because `Scheduler::exit()` must not be called in ring 3.
        // This will be resolved later, when we implement system calls.
        loop {}
    }

    /// Get a pointer to the top of the given stack.
    fn get_top_of_stack(stack: &Vec<u64>) -> *const u64 {
        unsafe { ptr::from_ref(&stack[stack.len() - 1]).offset(1) }
    }
}

impl Display for Thread {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "T{}", self.id)
    }
}
