use alloc::collections::BTreeMap;
use core::future::Future;
use core::ops::{DerefMut};
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};
use conquer_once::spin::OnceCell;
use futures_util::stream::{Stream, StreamExt};
use futures_util::task::AtomicWaker;

static TIMER_FLAG: OnceCell<AtomicBool> = OnceCell::uninit();
static WAKER: AtomicWaker = AtomicWaker::new();

pub const TIMER_FREQUENCY: u16 = 250;

/// Called by the timer interrupt handler
///
/// Must not block or allocate.
pub fn raise_timer() {
    if let Ok(bool_flag) = TIMER_FLAG.try_get() {
        bool_flag.store(true, Ordering::SeqCst);
        if Ok(true) == bool_flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst) {
            log::error!("[timer] raised timer flag hasn't been consumed last time!");
        }
        WAKER.wake();
    }
}

struct TimerStream {
    _private: ()
}

impl TimerStream {
    pub fn new() -> Self {
        TIMER_FLAG.try_init_once(|| AtomicBool::from(false))
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

        WAKER.register(&cx.waker());

        match timer_flag.compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed) {
            Ok(true) => {
                WAKER.take();
                Poll::Ready(Some(()))
            },
            Ok(false) => Poll::Pending,
            Err(_) => Poll::Pending
        }
    }
}

struct TimerTasksManager {
    tasks: BTreeMap<u64, (u64, AtomicWaker)>, // task id -> (ticks counter, waker)
}

static TIMER_TASKS_MANAGER: spin::Mutex<TimerTasksManager> = spin::Mutex::new(TimerTasksManager{ tasks: BTreeMap::new() });

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
        TIMER_TASKS_MANAGER.lock().decrement_all();
    }
}

struct Sleep {
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

        TIMER_TASKS_MANAGER
            .lock()
            .register_task(id, timer_value)
            .expect("Failed to register task");

        Sleep{ task_id: id }
    }
}

pub async fn sleep_for(sleep_for_ms: u64) {
    Sleep::new(sleep_for_ms).await;
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if TIMER_TASKS_MANAGER.lock().check_task(self.task_id).expect("Failed to check task") {
            Poll::Ready(())
        } else {
            TIMER_TASKS_MANAGER.lock().register_waker(self.task_id, &cx.waker()).expect("Failed to register waker");
            Poll::Pending
        }
    }
}
