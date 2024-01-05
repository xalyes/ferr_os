use alloc::vec::Vec;
use conquer_once::spin::OnceCell;
use core::{pin::Pin, task::{Poll, Context}};
use core::sync::atomic::Ordering::Relaxed;
use crossbeam_queue::ArrayQueue;
use futures_util::stream::{Stream, StreamExt};
use futures_util::task::AtomicWaker;
use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};
use shared_lib::interrupts::without_interrupts;
use shared_lib::out;
use shared_lib::logger::LOGGER;
use crate::task::executor::STOP;

static SCANCODE_QUEUE: OnceCell<ArrayQueue<u8>> = OnceCell::uninit();
static WAKER: AtomicWaker = AtomicWaker::new();

/// Called by the keyboard interrupt handler
///
/// Must not block or allocate.
pub(crate) fn add_scancode(scancode: u8) {
    if let Ok(queue) = SCANCODE_QUEUE.try_get() {
        if let Err(_) = queue.push(scancode) {
            log::warn!("scancode queue full; dropping keyboard input");
        } else {
            WAKER.wake();
        }
    } else {
        log::warn!("scancode queue uninitialized");
    }
}

pub struct ScancodeStream {
    _private: ()
}

impl ScancodeStream {
    pub fn new() -> Self {
        SCANCODE_QUEUE.try_init_once(|| ArrayQueue::new(100))
            .expect("ScancodeStream::new should only be called once");
        ScancodeStream{ _private: () }
    }
}

impl Stream for ScancodeStream {
    type Item = u8;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<u8>> {
        let queue = SCANCODE_QUEUE.try_get().expect("not initialized");

        let scancode = queue.pop();
        if scancode.is_some() {
            return Poll::Ready(scancode)
        }

        WAKER.register(&cx.waker());

        match queue.pop() {
            Some(scancode) => {
                WAKER.take();
                Poll::Ready(Some(scancode))
            },
            None => Poll::Pending
        }
    }
}

pub async fn print_keypresses() {
    let mut scancodes = ScancodeStream::new();
    let mut keyboard = Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore);
    let mut input_buffer: Vec<char> = Vec::new();

    while let Some(scancode) = scancodes.next().await {
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            if let Some(key) = keyboard.process_keyevent(key_event) {
                match key {
                    DecodedKey::Unicode(character) => {
                        without_interrupts(|| LOGGER.get().unwrap().lock().write_char_to_command_line(character));

                        //out!("{}", character);
                        if character == '\n' {
                            if input_buffer == ['s', 'h', 'u', 't', 'd', 'o', 'w', 'n'] {
                                out!("\nshutting down...\n");
                                STOP.store(true, Relaxed);
                                return;
                            } else {
                                input_buffer.clear();
                            }
                        } else {
                            input_buffer.push(character);
                        }
                    },
                    DecodedKey::RawKey(key) => out!("{:?}", key)
                }
            }
        }
    }
}