use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use core::ops::{Deref, DerefMut, Sub};
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};
use conquer_once::spin::OnceCell;
use futures_util::stream::{Stream, StreamExt};
use futures_util::task::AtomicWaker;
use crate::apic::read_rtc;
use core::borrow::BorrowMut;
use crate::task::TaskId;

static TIMER_FLAG: OnceCell<AtomicBool> = OnceCell::uninit();
static WAKER: AtomicWaker = AtomicWaker::new();

pub const TIMER_FREQUENCY: u16 = 250;

/// Called by the timer interrupt handler
///
/// Must not block or allocate.
pub(crate) fn raise_timer() {
    if let Ok(bool_flag) = TIMER_FLAG.try_get() {
        bool_flag.store(true, Ordering::SeqCst);
        if Ok(true) == bool_flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst) {
            log::error!("[timer] raised timer flag hasn't been consumed last time!");
        }
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

pub struct TimerTasksManager {
    pub tasks: alloc::collections::BTreeMap<u64, (u64, AtomicWaker)>, // task id -> (ticks counter, waker)
}

pub static mut TIMER_TASKS_MANAGER: TimerTasksManager = TimerTasksManager{ tasks: BTreeMap::new() };

impl TimerTasksManager {
    pub fn register_task(&mut self, id: u64, ticks: u64) -> Result<(), &'static str> {
        return if self.tasks.contains_key(&id) {
            Err("Task already registered")
        } else {
            self.tasks.insert(id, (ticks, AtomicWaker::new()));
            Ok(())
        }
    }

    pub fn decrement_all(&mut self) {
        for mut item in self.tasks.iter_mut() {
            let val = item.1.deref_mut();
            val.0 = val.0.checked_sub(1).unwrap_or(0);

            if val.0 == 0 {
                val.1.wake();
            }
        }
    }

    pub fn check_task(&mut self, id: u64) -> Result<bool, &'static str> {
        if self.tasks.get_mut(&id).expect("There is no such task").0.eq(&0) {
            self.tasks.remove(&id).expect("Failed to remove task from map");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn register_waker(&mut self, id: u64, waker: &Waker) -> Result<(), &'static str> {
        self.tasks.get_mut(&id).unwrap().1.register(waker);
        Ok(())
    }
}

pub async fn timer_loop() {
    let mut timer_stream = TimerStream::new();

    while let Some(()) = timer_stream.next().await {
        unsafe {
            TIMER_TASKS_MANAGER.borrow_mut().decrement_all();
        }
    }
}

pub async fn print_every_sec_task() {
    loop {
        sleep_for(1000).await;

        unsafe {
            static mut I: u64 = 1;
            log::info!("1 sec timer tick. {}. DateTime: {:?}", I, read_rtc());
            I += 1;
        }
    }

}

pub struct Sleep {
    task_id: u64
}

impl Sleep {
    pub fn new(sleep_for_ms: u64) -> Sleep {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

        let msec_freq = (1000 / TIMER_FREQUENCY) as u64; // every tick N msec passed

        let timer_value = if sleep_for_ms < msec_freq {
            1
        } else {
            sleep_for_ms / msec_freq
        };

        unsafe {TIMER_TASKS_MANAGER.borrow_mut().register_task(id, timer_value).unwrap();}

        Sleep{ task_id: id }
    }
}

pub fn sleep_for(sleep_for_ms: u64) -> Sleep {
    Sleep::new(sleep_for_ms)
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if unsafe { TIMER_TASKS_MANAGER.borrow_mut().check_task(self.task_id).unwrap() } {
            Poll::Ready(())
        } else {
            unsafe {TIMER_TASKS_MANAGER.borrow_mut().register_waker(self.task_id, &cx.waker()).unwrap();}
            Poll::Pending
        }
    }
}
