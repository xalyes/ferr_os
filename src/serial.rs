use core::arch::asm;
use core::fmt;
use spin::Mutex;
use lazy_static::lazy_static;
use bitflags::bitflags;

bitflags! {
    /// Line status flags
    struct LineStsFlags: u8 {
        const INPUT_FULL = 1;
        // 1 to 4 unknown
        const OUTPUT_EMPTY = 1 << 5;
        // 6 and 7 unknown
    }
}

pub unsafe fn outb(port: u16, val: u8) {
    asm!("outb %al, %dx", in("al") val, in("dx") port, options(att_syntax));
}
pub unsafe fn inb(port: u16) -> u8 {
    let ret: u8;
    asm!("inb %dx, %al", in("dx") port, out("al") ret, options(att_syntax));
    ret
}

macro_rules! wait_for {
    ($cond:expr) => {
        while !$cond {
            core::hint::spin_loop()
        }
    };
}

fn get_bit_at(input: u8, n: u8) -> bool {
    if n < 32 {
        input & (1 << n) != 0
    } else {
        false
    }
}

#[derive(Debug)]
pub struct SerialPort(u16 /* base port */);

impl SerialPort {
    pub const unsafe fn new(base: u16) -> Self {
        Self(base)
    }

    pub fn init(&mut self) {
        let port = self.0;
        unsafe {
            // Disable interrupts
            outb(port + 1, 0x00);

            // Enable DLAB
            outb(port + 3, 0x80);

            // Set maximum speed to 38400 bps by configuring DLL and DLM
            outb(port, 0x03);
            outb(port + 1, 0x00);

            // Disable DLAB and set data word length to 8 bits
            outb(port + 3, 0x03);

            // Enable FIFO, clear TX/RX queues and
            // set interrupt watermark at 14 bytes
            outb(port + 2, 0xc7);

            // Mark data terminal ready, signal request to send
            // and enable auxilliary output #2 (used as interrupt line for CPU)
            outb(port + 4, 0x0b);

            // Enable interrupts
            outb(port + 1, 0x01);
        }
    }

    fn line_sts(&mut self) -> LineStsFlags {
        unsafe { LineStsFlags::from_bits_truncate(inb(self.0 + 5)) }
    }

    pub fn send(&mut self, data: u8) {
        let port = self.0;
        unsafe {
            match data {
                8 | 0x7F => {
                    wait_for!(self.line_sts().contains(LineStsFlags::OUTPUT_EMPTY));
                    outb(port, 8);
                    wait_for!(self.line_sts().contains(LineStsFlags::OUTPUT_EMPTY));
                    outb(port, b' ');
                    wait_for!(self.line_sts().contains(LineStsFlags::OUTPUT_EMPTY));
                    outb(port, 8);
                }
                _ => {
                    wait_for!(self.line_sts().contains(LineStsFlags::OUTPUT_EMPTY));
                    outb(port, data);
                }
            }
        }
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.send(byte);
        }
        Ok(())
    }
}

lazy_static! {
    pub static ref SERIAL1: Mutex<SerialPort> = {
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();
        Mutex::new(serial_port)
    };
}

#[doc(hidden)]
pub fn _print(args: ::core::fmt::Arguments) {
    use core::fmt::Write;
    unsafe { SERIAL1.lock().write_fmt(args).expect("Printing to serial failed"); }
}

/// Prints to the host through the serial interface.
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*));
    };
}

/// Prints to the host through the serial interface, appending a newline.
#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(
        concat!($fmt, "\n"), $($arg)*));
}