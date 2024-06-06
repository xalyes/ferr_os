use chrono::{DateTime, TimeZone};
use crate::port::Port;

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
