use core::fmt;
use core::fmt::{Arguments, Write};
use conquer_once::spin::OnceCell;
use spinning_top::{RawSpinlock, Spinlock};
use spinning_top::lock_api::MutexGuard;
use crate::interrupts;
use crate::serial::SerialPort;

pub struct SerialLogger {
    port: SerialPort
}

impl SerialLogger {
    pub fn new() -> Self {
        let mut port = unsafe{ SerialPort::new(0x3F8) };
        port.init();
        SerialLogger{ port }
    }

    pub fn send(&mut self, data: u8) {
        self.port.send(data);
    }
}

impl fmt::Write for SerialLogger {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.send(byte);
        }
        Ok(())
    }
}

pub static SERIAL_LOGGER: OnceCell<LockedSerialLogger> = OnceCell::uninit();

/// A [`SerialLogger`] instance protected by a spinlock.
pub struct LockedSerialLogger(Spinlock<SerialLogger>);

impl LockedSerialLogger {
    /// Create a new instance that logs to the given framebuffer.
    pub fn new() -> Self {
        LockedSerialLogger(Spinlock::new(SerialLogger::new()))
    }

    pub fn lock(&self) -> MutexGuard<'_, RawSpinlock, SerialLogger> {
        self.0.lock()
    }

    pub fn write_fmt(&self, arguments: Arguments ) {
        interrupts::without_interrupts(|| {
            self.0.lock().write_fmt(arguments).unwrap();
        });
    }

    /// Force-unlocks the logger to prevent a deadlock.
    ///
    /// This method is not memory safe and should be only used when absolutely necessary.
    pub unsafe fn force_unlock(&self) {
        self.0.force_unlock();
    }
}

impl log::Log for LockedSerialLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        interrupts::without_interrupts(|| {
            let mut logger = self.0.lock();
            writeln!(logger, "{}:    {}", record.level(), record.args()).unwrap();
        });
    }

    fn flush(&self) {}
}