use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::slice::from_raw_parts_mut;
use core::ptr::read_volatile;
use spinning_top::{RawSpinlock, Spinlock};
use conquer_once::spin::OnceCell;
use core::fmt::{Arguments, Write};
use font8x8::UnicodeFonts;
use spinning_top::lock_api::MutexGuard;
use crate::interrupts;

#[derive(Clone, Copy)]
pub enum PixelFormat {
    Rgb,
    Bgr,
    Bitmask,
    BltOnly
}

#[derive(Clone, Copy)]
pub struct FrameBufferInfo {
    pub addr: u64,
    pub size: usize,
    pub width: usize,
    pub height: usize,
    pub pixel_format: PixelFormat,
    pub stride: usize
}

pub struct Logger {
    fb_info: FrameBufferInfo,
    fb: &'static mut [u8],
    x_pos: usize,
    y_pos: usize,

    char_buffer: VecDeque<Vec<char>>,
    char_buffer_width: usize,
    char_buffer_height: usize
}

impl Logger {
    pub fn new(fb_info: FrameBufferInfo) -> Self {
        let fb_slice = unsafe { from_raw_parts_mut(fb_info.addr as *mut u8, fb_info.size) };
        fb_slice.fill(0);

        let w = (fb_info.width - 1) / 8;
        let h = (fb_info.height - 1) / 8;

        let mut char_buffer = VecDeque::with_capacity(h);
        for _ in 0..w {
            char_buffer.push_back(vec!['\0'; w]);
        }

        Logger{fb_info, fb: &mut *fb_slice, x_pos: 0, y_pos: 0, char_buffer, char_buffer_width: w, char_buffer_height: h }
    }

    pub fn draw_char_buffer(&mut self) {
        for y in 0..self.char_buffer_height {
            for x in 0..self.char_buffer_width {
                let rendered = font8x8::BASIC_FONTS
                    .get(self.char_buffer[y][x])
                    .unwrap();

                self.write_8x8(rendered, 1 + x * 8, 1 + y * 8);
            }
        }
    }

    fn write_pixel(&mut self, x: usize, y: usize, intensity: u8) {
        let pixel_offset = y * self.fb_info.stride + x;
        let color = match &self.fb_info.pixel_format {
            PixelFormat::Rgb => [intensity, intensity, intensity / 2, 0],
            PixelFormat::Bgr => [intensity / 2, intensity, intensity, 0],
            _other => {
                loop {}
            }
        };
        let bytes_per_pixel = 4;
        let byte_offset = pixel_offset * bytes_per_pixel;
        self.fb[byte_offset..(byte_offset + bytes_per_pixel)]
            .copy_from_slice(&color[..bytes_per_pixel]);
        let _ = unsafe { read_volatile(&self.fb[byte_offset]) };
    }

    fn newline(&mut self) {
        self.y_pos += 1;
        self.carriage_return();

        if self.y_pos >= self.char_buffer_height {
            self.char_buffer.pop_front();
            self.char_buffer.push_back(vec!['\0'; self.char_buffer_width]);
            self.y_pos = self.char_buffer_height - 1;
            self.x_pos = 0;
            self.draw_char_buffer();
        }
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    pub fn clear(&mut self) {
        self.x_pos = 0;
        self.y_pos = 0;
        self.fb.fill(0);

        for i in 0..self.char_buffer_width {
            self.char_buffer[i].fill('\0');
        }
    }

    pub fn width(&self) -> usize {
        self.fb_info.width
    }
    pub fn height(&self) -> usize {
        self.fb_info.height
    }

    pub fn write_8x8(&mut self, rendered: [u8; 8], x_pos: usize, y_pos: usize) {
        for (y, byte) in rendered.iter().enumerate() {
            for (x, bit) in (0..8).enumerate() {
                let intensity = if *byte & (1 << bit) == 0 { 0 } else { 255 };
                self.write_pixel(x_pos + x, y_pos + y, intensity);
            }
        }
    }

    pub fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                if self.x_pos >= self.char_buffer_width {
                    self.newline();
                }

                self.char_buffer[self.y_pos][self.x_pos] = c;

                if c != '\0' {
                    let rendered = font8x8::BASIC_FONTS
                        .get(c);
                    if rendered.is_none() {
                        panic!("Failed to render char {}", c as u32);
                    }
                    self.write_8x8(rendered.unwrap(), 1 + self.x_pos * 8, 1 + self.y_pos * 8);
                } else {
                    let rendered = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
                    self.write_8x8(rendered, 1 + self.x_pos * 8, 1 + self.y_pos * 8);
                }

                self.x_pos += 1;
            }
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

