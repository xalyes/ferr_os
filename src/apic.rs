#![allow(dead_code)]
use core::arch::asm;
use shared_lib::addr::VirtAddr;
use crate::pic::Port;
use crate::interrupts;
use shared_lib::{read_u32_ptr, write_u32_ptr};
use crate::interrupts::InterruptIndex;

pub const APIC_APICID: u32     = 0x20;
pub const APIC_APICVER: u32    = 0x30;
pub const APIC_TASKPRIOR: u32  = 0x80;
pub const APIC_EOI: u32        = 0x0B0;
pub const APIC_LDR: u32        = 0x0D0;
pub const APIC_DFR: u32        = 0x0E0;
pub const APIC_SPURIOUS: u32   = 0x0F0;
pub const APIC_ESR: u32        = 0x280;
pub const APIC_ICRL: u32       = 0x300;
pub const APIC_ICRH: u32       = 0x310;
pub const APIC_LVT_TMR: u32    = 0x320;
pub const APIC_LVT_PERF: u32   = 0x340;
pub const APIC_LVT_LINT0: u32  = 0x350;
pub const APIC_LVT_LINT1: u32  = 0x360;
pub const APIC_LVT_ERR: u32    = 0x370;
pub const APIC_TMRINITCNT: u32 = 0x380;
pub const APIC_TMRCURRCNT: u32 = 0x390;
pub const APIC_TMRDIV: u32     = 0x3E0;
pub const APIC_LAST: u32       = 0x38F;
pub const APIC_DISABLE: u32    = 0x10000;
pub const APIC_SW_ENABLE: u32  = 0x100;
pub const APIC_CPUFOCUS: u32   = 0x200;
pub const APIC_NMI: u32        = 4<<8;
pub const TMR_PERIODIC: u32	= 0x20000;
pub const TMR_BASEDIV: u32	= 1 << 20;

pub struct Apic {
    apic_base: VirtAddr
}

impl Apic {
    pub const fn new() -> Apic {
        Apic{ apic_base: VirtAddr::new(0) }
    }

    pub unsafe fn initialize(&mut self, addr: VirtAddr) {
        self.apic_base = addr;

        self.apic_write(APIC_DFR, 0xFFFF_FFFF);
        let mut ldr = self.apic_read(APIC_LDR) & 0x00FFFFFF;

        ldr |= 0b0000_0001;
        self.apic_write(APIC_LDR, ldr);

        self.apic_write(APIC_LVT_TMR, APIC_DISABLE);
        self.apic_write(APIC_LVT_PERF, APIC_NMI);
        self.apic_write(APIC_LVT_LINT0, APIC_DISABLE);
        self.apic_write(APIC_LVT_LINT1, APIC_DISABLE);
        self.apic_write(APIC_TASKPRIOR, 0);
    }

    unsafe fn apic_read(&self, offset: u32) -> u32 {
        let apic_base = self.apic_base.0 as *mut u32;
        core::ptr::read_volatile(apic_base.offset((offset / 4) as isize))
    }

    unsafe fn apic_write(&self, offset: u32, value: u32) {
        let apic_base = self.apic_base.0 as *mut u32;
        core::ptr::write_volatile(apic_base.offset((offset / 4) as isize), value);
    }

    pub unsafe fn notify_end_of_interrupt(&mut self) {
        self.apic_write(APIC_EOI, 0);
    }
}

#[inline]
fn is_tsc_constant() -> bool {
    let mut edx: u32;
    unsafe {
        asm!(
        "push rbx",
        "mov eax, 80000007h",
        "cpuid",
        "pop rbx",
        inout("eax") 0 => _,
        out("ecx") _,
        out("edx") edx,
        );
    }
    log::info!("{:#x}", edx);
    return (edx & 0x100) == 0x100;
}

#[inline]
pub fn get_tsc() -> u64 {
    let mut edx: u32;
    let mut eax: u32;
    unsafe { asm!("rdtsc", out("edx") edx, out("eax") eax); }
    eax as u64 | ((edx as u64) << 32)
}

/*
 * Try to calibrate the TSC against the Programmable
 * Interrupt Timer and return the frequency of the TSC
 * in kHz.
 *
 * Return ULONG_MAX on failure to calibrate.
 */
pub fn pit_calibrate_tsc(latch: u32, ms: u64, loop_min: u16) -> u64 {
    unsafe {
        // Set the Gate high, disable speaker
        let mut pit_channel2_gate = Port::new(0x61);
        {
            let v = (pit_channel2_gate.read() & 0xfd) | 0x1;
            pit_channel2_gate.write(v);
        }

        /*
         * Setup CTC channel 2* for mode 0, (interrupt on terminal
         * count mode), binary count. Set the latch register to 50ms
         * (LSB then MSB) to begin countdown.
         */
        let mut pit_channel2_command = Port::new(0x43);
        pit_channel2_command.write(0xb0);

        let mut pit_channel2_data = Port::new(0x42);
        pit_channel2_data.write((latch & 0xff) as u8);
        pit_channel2_data.write((latch >> 8) as u8);

        let mut tsc = get_tsc();
        let t1 = tsc;
        let mut t2 = tsc;
        let mut delta;
        let mut tsc_max: u64 = 0;
        let mut tsc_min: u64 = 0xFFFF_FFFF_FFFF_FFFF;
        let mut pitcnt = 0;

        while (pit_channel2_gate.read() & 0x20) == 0 {
            t2 = get_tsc();
            delta = t2 - tsc;
            tsc = t2;
            if delta < tsc_min {
                tsc_min = delta;
            }
            if delta > tsc_max {
                tsc_max = delta;
            }
            pitcnt += 1;
        }

        log::info!("PIT values: {} {} {}", pitcnt, tsc_min, tsc_max);
        /*
         * Sanity checks:
         *
         * If we were not able to read the PIT more than loopmin
         * times, then we have been hit by a massive SMI
         *
         * If the maximum is 10 times larger than the minimum,
         * then we got hit by an SMI as well.
         */
        if pitcnt < loop_min || tsc_max > 10 * tsc_min {
            return 0xFFFF_FFFF_FFFF_FFFF;
        }

        delta = t2 - t1;
        log::info!("PIT: delta: {}", delta);
        delta / ms
    }
}

pub fn tsc_read_apic_ref(local_apic: VirtAddr) -> (u64, u32) {
    let max_retries = 5;
    let tsc_default_threshold = 0x20000;
    let mut t1: u64;
    let mut t2: u64;
    let apic_base = local_apic.0 as *mut u32;

    let mut apic_tmr: u32 = 0;
    for _ in 0..max_retries {
        t1 = get_tsc();
        apic_tmr = unsafe { read_u32_ptr(apic_base, APIC_TMRCURRCNT) };
        t2 = get_tsc();

        if t2 - t1 < tsc_default_threshold {
            log::info!("TSC read apic ref returning: {} {}", t2, apic_tmr);
            return (t2, apic_tmr);
        }
    }
    return (0x_FFFF_FFFF_FFFF_FFFF, apic_tmr);
}

// calculate the TSC frequency from apic timer reference
pub fn calc_apic_timer_ref(deltatsc: u64, pm1: u64, mut pm2: u64) -> u64 {
    let mut tmp: u64;

    if pm1 == 0 && pm2 == 0 {
        return 0x_FFFF_FFFF_FFFF_FFFF;
    }

    if pm2 < pm1 {
        pm2 += 1 << 24;
    }
    pm2 -= pm1;
    tmp = pm2 * 1000000000;
    tmp /= 3579545;
    deltatsc / tmp
}

// returns lowest CPU frequency
pub fn pit_hpet_ptimer_calibrate_cpu(local_apic: VirtAddr) -> u64 {
    // The clock frequency of the i8253/i8254 PIT
    let pit_tick_rate: u64 = 1193182;

    let cal_ms: u64 = 10;
    let cal_latch: u32 = (pit_tick_rate / (1000 / cal_ms)) as u32;
    let cal_pit_loops = 1000;

    let cal2_ms: u64 = 50;
    let cal2_latch: u32 = (pit_tick_rate / (1000 / cal2_ms)) as u32;
    let cal2_pit_loops = 5000;

    /*
     * Run 5 calibration loops to get the lowest frequency value
     * (the best estimate). We use two different calibration modes
     * here:
     *
     * 1) PIT loop. We set the PIT Channel 2 to oneshot mode and
     * load a timeout of 50ms. We read the time right after we
     * started the timer and wait until the PIT count down reaches
     * zero. In each wait loop iteration we read the TSC and check
     * the delta to the previous read. We keep track of the min
     * and max values of that delta. The delta is mostly defined
     * by the IO time of the PIT access, so we can detect when
     * any disturbance happened between the two reads. If the
     * maximum time is significantly larger than the minimum time,
     * then we discard the result and have another try.
     *
     * 2) Reference counter. If available we use the HPET or the
     * PMTIMER as a reference to check the sanity of that value.
     * We use separate TSC readouts and check inside of the
     * reference read for any possible disturbance. We discard
     * disturbed values here as well. We do that around the PIT
     * calibration delay loop as we have to wait for a certain
     * amount of time anyway.
     */

    let mut latch = cal_latch;
    let mut ms = cal_ms;
    let mut loopmin = cal_pit_loops;
    let mut tsc_pit_min: u64 = 0x_FFFF_FFFF_FFFF_FFFF;
    let mut tsc1: u64;
    let mut tsc2: u64;
    let mut ref1: u32;
    let mut ref2: u32;
    let mut tsc_ref_min = 0x_FFFF_FFFF_FFFF_FFFF;
    let mut delta: u64;

    for i in 0..3 {
        /*
         * Read the start value and the reference count of
         * hpet/pmtimer when available. Then do the PIT
         * calibration, which will take at least 50ms, and
         * read the end value.
         */

        (tsc1, ref1) = tsc_read_apic_ref(local_apic);
        let tsc_pit_khz = pit_calibrate_tsc(latch, ms, loopmin);
        (tsc2, ref2) = tsc_read_apic_ref(local_apic);
        log::info!("calibrated TSC-PIT Khz: {}", tsc_pit_khz);

        tsc_pit_min = u64::min(tsc_pit_min, tsc_pit_khz);

        if tsc1 == 0x_FFFF_FFFF_FFFF_FFFF || tsc2 == 0x_FFFF_FFFF_FFFF_FFFF {
            continue;
        }
        tsc2 = (tsc2 - tsc1) * 1000000;
        tsc2 = calc_apic_timer_ref(tsc2, ref1 as u64, ref2 as u64);

        tsc_ref_min = u64::min(tsc_ref_min, tsc2);

        // check the reference deviation
        delta = tsc_pit_min * 100;
        delta /= tsc_ref_min;

        if delta >= 90 && delta <= 110 {
            log::info!("PIT calibration matches APIC timer. {} loops", i + 1);
            return tsc_ref_min;
        }
        /*
         * Check whether PIT failed more than once. This
         * happens in virtualized environments. We need to
         * give the virtual PC a slightly longer timeframe for
         * the APIC timer to make the result precise.
         */
        if i == 1 && tsc_pit_min == 0x_FFFF_FFFF_FFFF_FFFF {
            log::warn!("PIT calibration failed more than once. Adjusting calibration params");
            latch = cal2_latch;
            ms = cal2_ms;
            loopmin = cal2_pit_loops;
        }
    }

    if tsc_pit_min == 0x_FFFF_FFFF_FFFF_FFFF {
        log::warn!("Unable to calibrate against PIT");

        if tsc_ref_min == 0x_FFFF_FFFF_FFFF_FFFF {
            panic!("Failed to calibrate TSC against PIT and APIC");
        }
        return tsc_ref_min;
    }

    log::info!("Using PIT calibration value");
    return tsc_pit_min;
}

pub fn disable_pic() {
    let mut p1 = Port::new(0x21);
    let mut p2 = Port::new(0xA1);

    unsafe {
        p1.write(0xff);
        p2.write(0xff);
    }
}

pub fn initialize_apic_timer(local_apic: VirtAddr) {
    unsafe { interrupts::APIC.lock().initialize(local_apic); };

    log::info!("Starting to initialize APIC timer");

    // Enable APIC
    unsafe {
        asm!(
        "mov ecx, 1bh; rdmsr; bts eax, 11; wrmsr", options(nomem, nostack)
        );
    }

    log::info!("APIC enabled");

    let apic_base = local_apic.0 as *mut u32;

    unsafe {
        write_u32_ptr(apic_base, APIC_SPURIOUS, read_u32_ptr(apic_base, APIC_SPURIOUS) | APIC_SW_ENABLE);

        asm!("cli", options(nomem, nostack));
        let cpu_khz = pit_hpet_ptimer_calibrate_cpu(local_apic);
        log::info!("Detected CPU freq: {} Khz", cpu_khz);

        let timer_frequency = 100; // x interrupts per sec
        let timer_value = (cpu_khz * 1000) / (16 * (timer_frequency));

        log::info!("Ok. let's enable APIC with proper value. timer init value: {}, timer_frequency per sec: {}", timer_value, timer_frequency);

        write_u32_ptr(apic_base, APIC_TMRINITCNT, timer_value as u32);
        write_u32_ptr(apic_base, APIC_LVT_TMR, InterruptIndex::Timer as u32 | TMR_PERIODIC);

        // setting divide value register again not needed by the manuals
        // although I have found buggy hardware that required it
        write_u32_ptr(apic_base, APIC_TMRDIV, 0x03);

        // enable hardware interrupts
        asm!("sti", options(nomem, nostack));
    }
}
