use core::ptr::read_volatile;
use core::slice::from_raw_parts_mut;
use font8x8::UnicodeFonts;

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

pub enum WriteMode {
    Log,
    Cli
}

pub struct Screen {
    fb_info: FrameBufferInfo,
    fb: &'static mut [u8],
    x_pos: usize,
    y_pos: usize,
    input_buffer: [char; 1000],
    input_buffer_idx: usize,
}

impl Screen {
    pub fn new(fb_info: FrameBufferInfo) -> Self {
        let fb_slice = unsafe { from_raw_parts_mut(fb_info.addr as *mut u8, fb_info.size) };
        fb_slice.fill(0);
        Screen{fb_info, fb: &mut *fb_slice, x_pos: 1, y_pos: 1,
            input_buffer: ['\0'; 1000],
            input_buffer_idx: 0,
        }
    }

    pub fn write_pixel(&mut self, x: usize, y: usize, intensity: u8) {
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

    pub fn width(&self) -> usize {
        self.fb_info.width
    }

    pub fn height(&self) -> usize {
        self.fb_info.height
    }

    fn newline(&mut self) {
        self.y_pos += 8;
        self.carriage_return();
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    pub fn clear(&mut self) {
        self.x_pos = 1;
        self.y_pos = 1;
        self.fb.fill(0);
    }

    pub fn get_cursor_pos(&self) -> Option<(usize, usize)> {
        let mut x_pos = self.x_pos;
        let mut y_pos = self.y_pos;
        if self.x_pos >= self.width() {
            x_pos = 0;
            y_pos += 8;
        }
        if y_pos >= (self.height() - 8) {
            return None;
        }
        Some((x_pos, y_pos))
    }

    pub fn write_8x8(&mut self, rendered: [u8; 8], x_pos: usize, y_pos: usize) {
        for (y, byte) in rendered.iter().enumerate() {
            for (x, bit) in (0..8).enumerate() {
                let intensity = if *byte & (1 << bit) == 0 { 0 } else { 255 };
                self.write_pixel(x_pos + x, y_pos + y, intensity);
            }
        }
    }

    fn apply_input(&mut self) {
        let mut y_pos = self.y_pos;
        if y_pos < self.height() - 16 {
            y_pos += 8;
        }
        let mut x_pos = 1;

        for i in self.input_buffer {
            if i == '\0' {
                break;
            }

            let rendered = font8x8::BASIC_FONTS
                .get(i)
                .unwrap();
            self.write_8x8(rendered, self.x_pos, self.y_pos);
            x_pos += 8;
        }
    }

    pub fn write_char(&mut self, c: char, write_mode: WriteMode) {
        if write_mode == WriteMode::Cli {
            if c == '\n' {

            } else {
                self.input_buffer[self.input_buffer_idx] = c;
                self.input_buffer_idx += 1;
            }

            self.apply_input();
        }

        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                if self.x_pos >= self.width() {
                    self.newline();
                }
                if self.y_pos >= (self.height() - 8) {
                    self.clear();
                }

                let rendered = font8x8::BASIC_FONTS
                    .get(c)
                    .unwrap();

                self.write_8x8(rendered, self.x_pos, self.y_pos);
                self.x_pos += 8;
            }
        }
        self.apply_input();
    }
}

