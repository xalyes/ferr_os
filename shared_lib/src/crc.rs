// CRC-32
// CCITT32 ANSI CRC with the polynomial 0x04c11db7 / 0xEDB88320

static CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];

    let mut i = 0;
    while i < 256 {
        let mut ch = i;
        let mut crc = 0u32;

        let mut j = 0;
        while j < 8 {
            let b = (ch ^ crc) & 1;

            crc >>= 1;

            if b != 0 {
                crc = crc ^ 0xEDB88320;
            }

            ch >>= 1;
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }

    table
};

pub fn calculate_crc32_partial(input: &[u8], mut crc: u32) -> u32 {
    for byte in input {
        let idx = (*byte as u32 ^ crc) & 0xFF;
        crc = (crc >> 8) ^ CRC32_TABLE[idx as usize];
    }
    crc
}

pub fn calculate_crc32(input: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;

    for byte in input {
        let idx = (*byte as u32 ^ crc) & 0xFF;
        crc = (crc >> 8) ^ CRC32_TABLE[idx as usize];
    }

    !crc
}

#[test_case]
fn simple_crc32_test() {
    assert_eq!(1267612143, calculate_crc32("abcdef".as_bytes()));
    assert_eq!(0xCBF43926, calculate_crc32("123456789".as_bytes()));
}