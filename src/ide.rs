use crate::port;

#[derive(Clone, Copy)]
struct IDEChannelRegister {
    io_base: u16,
    ctrl: u16,
    bm_ide: u16, // Bus Master IDE
    nIEN: u8 // nIEN (No interrupt)
}

#[derive(Clone, Copy)]
enum ATAChannel {
    Primary = 0x0,
    Secondary = 0x1
}

#[derive(Clone, Copy)]
enum IDEInterfaceType {
    Ata = 0x00,
    Atapi = 0x01
}

#[derive(Clone, Copy)]
enum DriveType {
    Master = 0x0,
    Slave = 0x1
}

#[derive(Clone, Copy)]
struct IDEDevice {
    reserved: bool,          // false (Empty) or true (This Drive really exists).
    channel: ATAChannel,
    drive: DriveType,        // Master or Slave
    interface_type: IDEInterfaceType,      // 0: ATA, 1:ATAPI.
    signature: u16,   // Drive Signature
    capabilities: u16, // Features.
    command_sets: u32, // Command Sets Supported.
    size: u16,        // Size in Sectors.
    model: [u8; 41]   // Model in string.
}

pub const ATA_REG_DATA: u8 = 0x0;
pub const ATA_REG_ERROR: u8 = 0x01;
pub const ATA_REG_FEATURES: u8 =   0x01;
pub const ATA_REG_SECCOUNT0: u8 =  0x02;
pub const ATA_REG_LBA0: u8 =       0x03;
pub const ATA_REG_LBA1: u8 =       0x04;
pub const ATA_REG_LBA2: u8 =       0x05;
pub const ATA_REG_HDDEVSEL: u8 =   0x06;
pub const ATA_REG_COMMAND: u8 =    0x07;
pub const ATA_REG_STATUS: u8 =     0x07;
pub const ATA_REG_SECCOUNT1: u8 =  0x08;
pub const ATA_REG_LBA3: u8 =       0x09;
pub const ATA_REG_LBA4 : u8 =      0x0A;
pub const ATA_REG_LBA5  : u8 =     0x0B;
pub const ATA_REG_CONTROL: u8 =    0x0C;
pub const ATA_REG_ALTSTATUS: u8 = 0x0C;
pub const ATA_REG_DEVADDRESS: u8 = 0x0D;

static mut CHANNELS: [IDEChannelRegister; 2] = [IDEChannelRegister{ io_base: 0, ctrl: 0, bm_ide: 0, nIEN: 0 }; 2];

unsafe fn ide_write(channel: ATAChannel, reg: u8, data: u8) {
    if reg > 0x07 && reg < 0x0C {
        ide_write(channel, ATA_REG_CONTROL, 0x80 | CHANNELS[channel as usize].nIEN);
    }

    if (reg < 0x08) {
        port::write(CHANNELS[channel as usize].io_base + reg as u16 - 0x00, data);
    }
    else if (reg < 0x0C) {
        port::write(CHANNELS[channel as usize].io_base + reg as u16 - 0x06, data);
    }
    else if (reg < 0x0E) {
        port::write(CHANNELS[channel as usize].ctrl + reg as u16 - 0x0A, data);
    }
    else if (reg < 0x16) {
        port::write(CHANNELS[channel as usize].bm_ide + reg as u16 - 0x0E, data);
    }

    if reg > 0x07 && reg < 0x0C {
        ide_write(channel, ATA_REG_CONTROL, CHANNELS[channel as usize].nIEN);
    }
}

pub(crate) fn ide_initialize(prog_if: u8) {
    // IDE compatibility mode constants
    let bar0: u32 = 0x1F0;
    let bar1: u32 = 0x3F6;
    let bar2: u32 = 0x170;
    let bar3: u32 = 0x376;
    // don't use DMA now
    let bar4: u32 = 0x0;

    unsafe {
        CHANNELS[ATAChannel::Primary as usize].io_base = (bar0 & 0xFFFFFFFC) as u16;
        CHANNELS[ATAChannel::Primary as usize].ctrl = (bar1 & 0xFFFFFFFC) as u16;

        CHANNELS[ATAChannel::Secondary as usize].io_base = (bar2 & 0xFFFFFFFC)  as u16;
        CHANNELS[ATAChannel::Secondary as usize].ctrl = (bar3 & 0xFFFFFFFC) as u16;

        CHANNELS[ATAChannel::Primary as usize].bm_ide = ((bar4 & 0xFFFFFFFC) + 0) as u16; // Bus Master IDE
        CHANNELS[ATAChannel::Secondary as usize].bm_ide = ((bar4 & 0xFFFFFFFC) + 8) as u16; // Bus Master IDE

        // Disable IRQs
        ide_write(ATAChannel::Primary, ATA_REG_CONTROL, 2);
        ide_write(ATAChannel::Secondary, ATA_REG_CONTROL, 2);
    }
    
    let mut drives: [IDEDevice; 4] = [IDEDevice{
        reserved: false,
        channel: ATAChannel::Primary,
        drive: DriveType::Master,
        interface_type: IDEInterfaceType::Ata,
        signature: 0,
        capabilities: 0,
        command_sets: 0,
        size: 0,
        model: [0; 41],
    }; 4];

    let mut drives_num = 0;
    for channel in [ATAChannel::Primary, ATAChannel::Secondary] {
        for drive in [DriveType::Master, DriveType::Slave] {
            let err: u8 = 0;
            let interface_type = IDEInterfaceType::Ata;
            let status: u8;

            drives[drives_num].reserved = false;
            unsafe {
                ide_write(channel, ATA_REG_HDDEVSEL, 0xA0 | ((drive as u8) << 4));
            }


        }
    }
}