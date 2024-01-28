use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll};
use conquer_once::spin::OnceCell;
use futures_util::stream::{Stream, StreamExt};
use futures_util::task::AtomicWaker;

static TIMER_FLAG: OnceCell<AtomicBool> = OnceCell::uninit();
static WAKER: AtomicWaker = AtomicWaker::new();

/// Called by the timer interrupt handler
///
/// Must not block or allocate.
pub(crate) fn raise_timer() {
    if let Ok(bool_flag) = TIMER_FLAG.try_get() {
        bool_flag.store(true, Ordering::SeqCst);
        WAKER.wake();
    } else {
        //log::warn!("Timer flag uninitialized");
    }
}

pub struct TimerStream {
    _private: ()
}

impl TimerStream {
    pub fn new() -> Self {
        crate::task::timer::TIMER_FLAG.try_init_once(|| AtomicBool::from(false))
            .expect("TimerStream::new should only be called once");
        TimerStream { _private: () }
    }
}

impl Stream for TimerStream {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<()>> {
        let timer_flag = crate::task::timer::TIMER_FLAG.try_get().expect("not initialized");

        if Ok(true) == timer_flag.compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed) {
            return Poll::Ready(Some(()))
        }

        crate::task::timer::WAKER.register(&cx.waker());

        match timer_flag.compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed) {
            Ok(true) => {
                crate::task::timer::WAKER.take();
                Poll::Ready(Some(()))
            },
            Ok(false) => Poll::Pending,
            Err(_) => Poll::Pending
        }
    }
}

pub async fn timer_loop() {
    let mut timer_stream = TimerStream::new();

    while let Some(()) = timer_stream.next().await {
        unsafe {
            static mut I: u64 = 0;
            if I % 1000 == 0 {
                log::info!("10 sec timer tick. {}", I / 1000);
            }
            I += 1;
        }
    }
}
