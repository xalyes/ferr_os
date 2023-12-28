use core::fmt;
use spinning_top::{RawSpinlock, Spinlock};
use conquer_once::spin::OnceCell;
use core::fmt::{Arguments, Write};
use spinning_top::lock_api::MutexGuard;
use crate::interrupts;
use crate::screen::{FrameBufferInfo, Screen};

pub struct Logger {
    screen: Screen,
    input_buffer: [char; 1000],
    input_buffer_idx: usize,
    input_buffer_processed: bool
}

impl Logger {
    pub fn new(fb_info: FrameBufferInfo) -> Self {
        Logger{
            screen: Screen::new(fb_info),
            input_buffer: ['\0'; 1000],
            input_buffer_idx: 0,
            input_buffer_processed: false
        }
    }

    pub fn write_char(&mut self, c: char) {
        if !self.input_buffer.starts_with(&['\0']) {
            if c != '\n' && !self.input_buffer_processed {
                // clear line
                let mut x = 1;
                if let Some((_, y)) = self.screen.get_cursor_pos() {
                    while x < self.screen.width() {
                        let rendered_cursor = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
                        self.screen.write_8x8(rendered_cursor, x, y);
                        x += 8;
                    }
                }

                self.screen.write_char(c);

                for i in self.input_buffer {
                    if i == '\0' {
                        break;
                    }

                    self.screen.write_char(i);
                }
                self.input_buffer_processed = true;
            } else if c == '\n' {
                self.input_buffer_processed = false;
            }
        } else {
            self.screen.write_char(c);
        }
    }

    pub fn handle_keypress(&mut self, c: char) {
        if let Some((x, y)) = self.screen.get_cursor_pos() {
            let rendered_cursor = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
            self.screen.write_8x8(rendered_cursor, x, y);
        }

        self.screen.write_char(c);
        self.input_buffer[self.input_buffer_idx] = c;
        self.input_buffer_idx += 1;

        static SHUTDOWN_COMMAND: [char; 10] = ['s', 'h', 'u', 't', 'd', 'o', 'w', 'n', '\n', '\0'];

        if c == '\n' {
            if self.input_buffer.starts_with(&SHUTDOWN_COMMAND) {
                self.write_str("\nshutting down...\n").expect("Failed to write str");
                // STOP.store(true, Relaxed);
            }
            self.input_buffer_idx = 0;
            self.input_buffer = ['\0'; 1000];
        }

        if let Some((x, y)) = self.screen.get_cursor_pos() {
            let rendered_cursor = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
            self.screen.write_8x8(rendered_cursor, x, y);
        }
    }
}

impl fmt::Write for Logger {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}

pub static LOGGER: OnceCell<LockedLogger> = OnceCell::uninit();

/// A [`Logger`] instance protected by a spinlock.
pub struct LockedLogger(Spinlock<Logger>);

impl LockedLogger {
    /// Create a new instance that logs to the given framebuffer.
    pub fn new(fb_info: FrameBufferInfo) -> Self {
        LockedLogger(Spinlock::new(Logger::new(fb_info)))
    }

    pub fn lock(&self) -> MutexGuard<'_, RawSpinlock, Logger> {
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

impl log::Log for LockedLogger {
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

#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => {
        LOGGER.get().unwrap().write_fmt(format_args!($($arg)*));
    };
}

