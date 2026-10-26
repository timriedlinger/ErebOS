/*
 * A basic driver for the serial port, only supporting output.
 *
 * Author: Michael Schoetter, Heinrich Heine University Duesseldorf, 2023-03-07
 *         Fabian Ruhland, Heinrich Heine University Duesseldorf, 2026-01-07
 */

use crate::device::cpu::IoPort;
use crate::library::spinlock::Spinlock;
use bitflags::bitflags;
use core::fmt;

/// Standard COM port for kernel output via the logger
pub static COM1: Spinlock<ComPort> = Spinlock::new(ComPort::new(ComBaseAddress::Com1));

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
/// Standardized IO-Port base addresses for the first four COM ports.
/// We usually only use `Com1`.
pub enum ComBaseAddress {
    Com1 = 0x3f8,
    Com2 = 0x2f8,
    Com3 = 0x3e8,
    Com4 = 0x2e8,
}

bitflags! {
    /// Status flags for the line protocol.
    /// The most important one for use is `READY_TO_WRITE`, as it indicates
    /// whether we can currently write to the data port or need to wait.
    pub struct LineStatus: u8 {
        const READY_TO_READ = 0x01;
        const OVERRUN_ERROR = 0x02;
        const PARITY_ERROR = 0x04;
        const FRAMING_ERROR = 0x08;
        const BREAK_INDICATOR = 0x10;
        const READY_TO_WRITE = 0x20;
        const TRANSMITTER_EMPTY = 0x0f;
        const IMPENDING_ERROR = 0x01;
    }
}

/// Struct representing a COM port
pub struct ComPort {
    /// IO-port where output is written to
    data_port: IoPort,
    /// Interrupt control register (i.e., enable/disable interrupts)
    interrupt_control_port: IoPort,
    /// Configuration register for the line protocol (e.g., baud rate)
    line_control_port: IoPort,
    /// Status register for the line protocol (e.g., ready to read or write)
    line_status_port: IoPort,
}

impl ComPort {
    /// Create a new COM port
    pub const fn new(base_addr: ComBaseAddress) -> ComPort {
        ComPort {
            data_port: IoPort::new(base_addr as u16),
            interrupt_control_port: IoPort::new(base_addr as u16 + 1),
            line_control_port: IoPort::new(base_addr as u16 + 3),
            line_status_port: IoPort::new(base_addr as u16 + 5),
        }
    }

    /// Initialize the COM port.
    /// This function disables interrupts and sets the baud rate to 115200 (max rate)
    /// with 8 data bits, 1 stop bit, and no parity bits.
    pub fn init(&mut self) {
        unsafe {
            // Disable all interrupts
            self.interrupt_control_port.outb(0x00);

            // Enable DLAB, so that the divisor can be set
            self.line_control_port.outb(0x80);

            // Set divisor to 1 (115200 baud)
            self.data_port.outb(0x01); // Divisor low byte
            self.interrupt_control_port.outb(0x00); // Divisor high byte

            // Set line protocol configuration: 8 data bits, 1 stop bit, no parity
            self.line_control_port.outb(0x03);
        }
    }

    /// Write a single byte to the COM port.
    pub fn write_byte(&mut self, byte: u8) {
        if byte == '\n' as u8 {
            self.write_byte('\r' as u8);
        }

        loop {
            unsafe {
                let status = LineStatus::from_bits_retain(self.line_status_port.inb());
                if status.contains(LineStatus::READY_TO_WRITE) {
                    break;
                }
            }
        }

        unsafe {
            self.data_port.outb(byte);
        }
    }
}

// Implement the `fmt::Write` trait for the serial port to support formatted output.
// We only need to implement the `write_str()` method, which writes a string to the serial port.
// This allows formatted output via the `write_fmt()` method provided by the `fmt::Write` trait.
impl fmt::Write for ComPort {
    /// Write a string to the COM port by iterating over each byte in the string and writing it using `write_byte()`.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            self.write_byte(b);
        }
        Ok(())
    }
}
