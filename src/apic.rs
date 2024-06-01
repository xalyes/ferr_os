#![allow(dead_code)]
use core::arch::asm;
use shared_lib::addr::VirtAddr;
use crate::port::Port;
use crate::interrupts;
use shared_lib::{get_tsc, read_u32_ptr, write_u32_ptr};
use shared_lib::bits::{set_bit, set_bits};
use crate::interrupts::InterruptIndex;
use crate::xsdt::ApicAddresses;
use chrono::{DateTime, TimeZone};

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
        core::ptr::write_volatile(apic_base.byte_offset(offset as isize), value);
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

unsafe fn read_io_apic(io_apic: *mut u32, register: u32) -> u32 {
    write_u32_ptr(io_apic, 0, register & 0xff);
    read_u32_ptr(io_apic, 0x10)
}

unsafe fn write_io_apic(io_apic: *mut u32, register: u32, value: u32) {
    write_u32_ptr(io_apic, 0, register & 0xff);
    write_u32_ptr(io_apic, 0x10, value);
}

pub fn read_rtc() -> DateTime<chrono::Utc> {
    let mut century: u8;
    let mut year: u8;
    let mut month: u8;
    let mut day: u8;
    let mut hour: u8;
    let mut minute: u8;
    let mut second: u8;

    let update_in_progress = || -> bool {
        let mut cmos_control_port = Port::new(0x70);
        let mut cmos_data_port = Port::new(0x71);

        unsafe {
            cmos_control_port.write(0x0A);
            (cmos_data_port.read() & 0x80) != 0
        }
    };

    let get_rtc_register = |reg: u8| -> u8 {
        let mut cmos_control_port = Port::new(0x70);
        let mut cmos_data_port = Port::new(0x71);

        unsafe {
            cmos_control_port.write(reg);
            cmos_data_port.read()
        }
    };

    while update_in_progress() {};

    second = get_rtc_register(0x00);
    minute = get_rtc_register(0x02);
    hour = get_rtc_register(0x04);
    day = get_rtc_register(0x07);
    month = get_rtc_register(0x08);
    year = get_rtc_register(0x09);
    century = get_rtc_register(0x32);

    loop {
        let last_second = second;
        let last_minute = minute;
        let last_hour = hour;
        let last_day = day;
        let last_month = month;
        let last_year = year;
        let last_century = century;

        while update_in_progress() {};

        second = get_rtc_register(0x00);
        minute = get_rtc_register(0x02);
        hour = get_rtc_register(0x04);
        day = get_rtc_register(0x07);
        month = get_rtc_register(0x08);
        year = get_rtc_register(0x09);
        century = get_rtc_register(0x32);

        if second == last_second && minute == last_minute && hour == last_hour
            && day == last_day && month == last_month && year == last_year && century == last_century {
            break
        }
    }

    let register_b = get_rtc_register(0x0B);

    // Convert BCD to binary values if necessary
    if register_b & 0x04 == 0 {
        second = (second & 0x0F) + ((second / 16) * 10);
        minute = (minute & 0x0F) + ((minute / 16) * 10);
        hour = ( (hour & 0x0F) + (((hour & 0x70) / 16) * 10) ) | (hour & 0x80);
        day = (day & 0x0F) + ((day / 16) * 10);
        month = (month & 0x0F) + ((month / 16) * 10);
        year = (year & 0x0F) + ((year / 16) * 10);
        century = (century & 0x0F) + ((century / 16) * 10);
    }

    // Convert 12-hour clock to 24-hour clock if necessary
    if register_b & 0x02 == 0 && hour & 0x80 == 1 {
        hour = ((hour & 0x7F) + 12) % 24;
    }

    chrono::Utc.with_ymd_and_hms(century as i32 * 100 + year as i32, month as u32, day as u32, hour as u32, minute as u32, second as u32).unwrap()
}

pub fn initialize_apic(apic_addrs: ApicAddresses) {
    unsafe { interrupts::APIC.lock().initialize(apic_addrs.local_apic_addr); };

    log::info!("Starting to initialize APIC timer");

    // Enable APIC
    unsafe {
        asm!(
        "mov ecx, 1bh; rdmsr; bts eax, 11; wrmsr", options(nomem, nostack)
        );
        asm!("cli", options(nomem, nostack));
    }

    log::info!("APIC enabled");

    let apic_base = apic_addrs.local_apic_addr.0 as *mut u32;

    let mut date_time = read_rtc();
    log::info!("CMOS datetime: {:?}", date_time);

    unsafe {
        write_u32_ptr(apic_base, APIC_TMRDIV, 0x03);
        write_u32_ptr(apic_base, APIC_SPURIOUS, read_u32_ptr(apic_base, APIC_SPURIOUS) | APIC_SW_ENABLE);
    }

    let mut full_second_passing = false;
    let mut first_measure = 0;
    let mut second_measure = 0;
    let mut third_measure = 0;

    loop {
        let new_date_time = read_rtc();
        if date_time != new_date_time {
            let ticks_in_1s = 0xFFFFFFFF - unsafe {
                write_u32_ptr(apic_base, APIC_LVT_TMR, APIC_DISABLE);
                read_u32_ptr(apic_base, APIC_TMRCURRCNT)
            };
            if !full_second_passing {
                full_second_passing = true;
            } else if first_measure == 0 {
                first_measure = ticks_in_1s;
            } else if second_measure == 0 {
                second_measure = ticks_in_1s;
            } else if third_measure == 0 {
                third_measure = ticks_in_1s;
            } else {
                break;
            }

            log::info!("New datetime: {:?}. Ticks elapsed: {}", new_date_time, ticks_in_1s);
            date_time = new_date_time;

            unsafe {
                // one-shot mode
                write_u32_ptr(apic_base, APIC_LVT_TMR, InterruptIndex::Timer as u32);
                write_u32_ptr(apic_base, APIC_TMRINITCNT, 0xFFFFFFFF);
            }
        }
    }

    log::info!("In 1 second we had {} {} {} ticks", first_measure, second_measure, third_measure);
    let avg_ticks = (first_measure as u64 + second_measure as u64 + third_measure as u64) / 3;
    let bus_freq: u64 = avg_ticks * 16;
    log::info!("CPU bus freq: {} Mhz", ((bus_freq / 1000) as f64) / 1000.0);

    let timer_frequency = 100; // x interrupts per sec
    let timer_value = avg_ticks / timer_frequency; // x interrupts per sec

    log::info!("Ok. let's enable APIC with proper value. timer init value: {}, timer_frequency per sec: {}", timer_value, timer_frequency);

    unsafe {
        write_u32_ptr(apic_base, APIC_TMRINITCNT, timer_value as u32);
        write_u32_ptr(apic_base, APIC_LVT_TMR, InterruptIndex::Timer as u32 | TMR_PERIODIC);

        let local_apic_id = read_u32_ptr(apic_base, APIC_APICID);

        let io_apic_base = apic_addrs.io_apic_addr.0 as *mut u32;

        let version = read_io_apic(io_apic_base, 0x1);

        log::info!("IOAPIC[0]: version: {}, address: {:#x}", version as u8, apic_addrs.io_apic_addr.0);
        let mut low_reg = read_io_apic(io_apic_base, 0x12) as u64;

        set_bits(&mut low_reg, InterruptIndex::Keyboard as u64, 0);

        set_bits(&mut low_reg, 0, 8); // Fixed delivery mode
        set_bit(&mut low_reg, 11, false); // Physical destination
        set_bit(&mut low_reg, 13, false); // Pin polarity - active high
        set_bit(&mut low_reg, 15, false); // Trigger mode - edge
        set_bit(&mut low_reg, 16, false); // unmask interrupt

        write_io_apic(io_apic_base, 0x12, low_reg as u32);
        write_io_apic(io_apic_base, 0x13, local_apic_id);

        // enable hardware interrupts
        asm!("sti", options(nomem, nostack));
    }
}
