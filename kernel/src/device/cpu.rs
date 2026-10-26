/*
 * Contains wrappers for different CPU functions, as well as the `IoPort` struct
 * for reading and writing to I/O-ports.
 *
 * Author: Michael Schoetter, Heinrich Heine University Duesseldorf, 2024-06-09
 *         Fabian Ruhland, Heinrich Heine University Duesseldorf, 2026-01-07
 */

use bitflags::bitflags;
use core::arch::asm;

/// Represents an I/O-port for reading and writing data.
pub struct IoPort {
    port: u16,
}

impl IoPort {
    /// Create a new IoPort object
    pub const fn new(port: u16) -> IoPort {
        IoPort { port }
    }

    /// Write a single byte to a port
    #[inline]
    pub unsafe fn outb(&mut self, data: u8) {
        unsafe {
            asm!(
            "out dx, al",
            in("dx") self.port,
            in("al") data,
            );
        }
    }

    /// Write a word (two bytes) to a port
    #[inline]
    pub unsafe fn outw(&mut self, data: u16) {
        unsafe {
            asm!(
            "out dx, ax",
            in("dx") self.port,
            in("ax") data,
            );
        }
    }

    /// Write a double word (four bytes) to a port
    #[inline]
    pub unsafe fn outdw(&mut self, data: u32) {
        unsafe {
            asm!(
            "out dx, eax",
            in("dx") self.port,
            in("eax") data,
            );
        }
    }

    /// Read a single byte from a port
    #[inline]
    pub unsafe fn inb(&mut self) -> u8 {
        let ret: u8;
        unsafe {
            asm!(
            "in al, dx",
            in("dx") self.port,
            out("al") ret,
            );
        }
        ret
    }

    /// Read a word (two bytes) from a port
    #[inline]
    pub unsafe fn inw(&mut self) -> u16 {
        let ret: u16;
        unsafe {
            asm!(
            "in ax, dx",
            in("dx") self.port,
            out("ax") ret,
            );
        }
        ret
    }

    /// Read a double word (four bytes) from a port
    #[inline]
    pub unsafe fn indw(&mut self) -> u32 {
        let ret: u32;
        unsafe {
            asm!(
            "in eax, dx",
            in("dx") self.port,
            out("eax") ret,
            );
        }
        ret
    }
}

/// Wait a very short amount of time (1-4 us) by writing a byte to an unused port (0x80).
/// This is not a reliable way to wait, but works for some cases (e.g. PIC initialization).
/// See OSDev for more details: https://wiki.osdev.org/Inline_Assembly/Examples#I/O_access.
#[inline]
pub fn io_wait() {
    unsafe {
        asm!(
        "out 0x80, al",
        in("al") 0u8,
        );
    }
}

/// Check if IE bit is set in RFLAGS
#[inline]
pub fn is_int_enabled() -> bool {
    let rflags: u64;

    unsafe {
        asm!(
        "pushf;",
        "pop {};",
        lateout(reg) rflags,
        options(nomem, nostack, preserves_flags)
        );
    };

    (rflags & (1u64 << 9)) != 0
}

/// Clear IE bit in RFLAGS and return the previous state
#[inline]
pub fn disable_int_nested() -> bool {
    let was_enabled = is_int_enabled();
    disable_int();
    was_enabled
}

/// Set IE bit in RFLAGS if it was set before
#[inline]
pub fn enable_int_nested(was_enabled: bool) {
    if was_enabled == true {
        enable_int();
    }
}

/// Set IE bit in RFLAGS
#[inline]
pub fn enable_int() {
    unsafe {
        asm!("sti");
    }
}

/// Clear IE bit in RFLAGS
#[inline]
pub fn disable_int() {
    unsafe {
        asm!("cli");
    }
}

/// Stop the CPU until an interrupt occurs
#[inline]
pub fn halt() {
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}

/// Get the current value of RFLAGS
#[inline]
pub fn get_flags() -> u64 {
    let rflags: u64;
    unsafe {
        asm!(
        "pushfq;",
        "pop {};",
        out(reg) rflags,
        options(nomem, preserves_flags)
        );
    }

    rflags
}

/// Execute a closure without interrupts
#[inline]
pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let ie = disable_int_nested();
    let ret = f();

    enable_int_nested(ie);
    ret
}

bitflags! {
    /// Flags in control register CR0 (see Intel SDM Vol. 3, Chapter 2.5: Control Registers).
    #[derive(Debug, Clone, Copy)]
    pub struct Cr0Flags: u64 {
        const PROTECTED_MODE_ENABLE = 1 << 0;
        const MONITOR_COPROCESSOR = 1 << 1;
        const EMULATE_COPROCESSOR = 1 << 2;
        const TASK_SWITCHED = 1 << 3;
        const EXTENSION_TYPE = 1 << 4;
        const NUMERIC_ERROR = 1 << 5;
        /// If set, the CPU prevents ring 0 code from writing to read-only pages.
        const WRITE_PROTECT = 1 << 16;
        const ALIGNMENT_MASK = 1 << 18;
        const NOT_WRITE_THROUGH = 1 << 29;
        const CACHE_DISABLE = 1 << 30;
        const PAGING = 1 << 31;
    }
}

/// Read the current value of control register CR0.
/// Unknown/reserved bits are retained, so that they are written back unchanged by `write_cr0()`.
#[inline]
pub fn read_cr0() -> Cr0Flags {
    let value: u64;
    unsafe {
        asm!("mov {}, cr0", out(reg) value, options(nomem, nostack, preserves_flags));
    }

    Cr0Flags::from_bits_retain(value)
}

/// Write the given flags into control register CR0.
/// No `nomem` option here, because changing CR0 (e.g. WRITE_PROTECT) affects memory accesses,
/// so the compiler must not move memory accesses across this instruction.
#[inline]
pub fn write_cr0(flags: Cr0Flags) {
    unsafe {
        asm!("mov cr0, {}", in(reg) flags.bits(), options(nostack, preserves_flags));
    }
}

/// Get the current privilege level (CPL), which is stored in bits 0-1 of the code segment register.
/// Returns 0 in kernel mode (ring 0) and 3 in user mode (ring 3).
/// Reading CS is allowed in every ring.
#[inline]
pub fn current_privilege_level() -> u8 {
    let cs: u16;
    unsafe {
        asm!("mov {:x}, cs", out(reg) cs, options(nomem, nostack, preserves_flags));
    }

    (cs & 0x3) as u8
}
