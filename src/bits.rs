
pub fn set_bit(num: &mut u64, n: u8, value: bool) {
    let mask = (value as u64) << n;

    *num |= mask;
}

pub fn set_bits(num: &mut u64, mask: u64, shift: u8) {
    let mask = mask << shift;

    *num |= mask;
}