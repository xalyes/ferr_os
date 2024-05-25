use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::Ordering::Relaxed;
use shared_lib::logger::{FrameBufferInfo, Logger};
use crate::task::executor::STOP;

pub struct Shell {
    logger: Logger,
    input_buffer: Vec<char>,
}

impl Shell {
    pub fn new(fb_info: FrameBufferInfo) -> Self {
        let mut logger = Logger::new(fb_info);
        logger.write_str("# ").unwrap();
        Shell{ logger, input_buffer: Vec::new() }
    }

    pub fn char_input(&mut self, c: char) {
        self.logger.write_char(c);
        if c != '\n' {
            self.input_buffer.push(c);
            return;
        }

        if self.input_buffer == ['s', 'h', 'u', 't', 'd', 'o', 'w', 'n'] {
            self.logger.write_str("\nshutting down...\n").unwrap();
            STOP.store(true, Relaxed);
            return;
        } else if self.input_buffer == [ 'h', 'e', 'l', 'p' ] {
            self.logger.write_str("This is Rust OS! Commands list:\n").unwrap();
            self.logger.write_str("- help\n").unwrap();
            self.logger.write_str("- shutdown\n").unwrap();
        }

        self.input_buffer.clear();
        self.logger.write_str("# ").unwrap();
    }
}