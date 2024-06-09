use alloc::vec::Vec;
use crate::port;
use crate::port::Port;
use crate::task::timer::sleep_for;

#[derive(Clone, Copy)]
struct IDEChannelRegister {
    io_base: u16,
    ctrl: u16,
    bm_ide: u16, // Bus Master IDE
    no_interrupt: u8
}

#[derive(Clone, Copy, Debug)]
pub enum ATAChannel {
    Primary = 0x0,
    Secondary = 0x1
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
enum IDEInterfaceType {
    Ata = 0x00,
    Atapi = 0x01
}


#[derive(Clone, Copy, Debug)]
pub enum DriveType {
    Master = 0x0,
    Slave = 0x1
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct IDEDevice {
    reserved: bool,          // false (Empty) or true (This Drive really exists).
    pub channel: ATAChannel,
    pub drive: DriveType,        // Master or Slave
    interface_type: IDEInterfaceType,      // 0: ATA, 1:ATAPI.
    signature: u16,   // Drive Signature
    capabilities: u16, // Features.
    command_sets: u32, // Command Sets Supported.
    pub size: u32,        // Size in Sectors.
    pub model: [u8; 41],   // Model in string.
    enabled_48bit: bool // 48 bit addressing supported
}

#[repr(usize)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum IdentifyBufferOffset {
    DeviceType   = 0,
    Cylinders    = 1,
    Heads        = 3,
    Sectors      = 6,
    Serial       = 10,
    Model        = 27,
    Capabilities = 49,
    Fieldvalid   = 53,
    MaxLba       = 60,
    Commandsets  = 82,
    MaxLbaExt    = 100
}

#[repr(u8)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum AtaRegister {
    Data = 0x0,
    ErrorAndFeatures = 0x01,
    SecCount0 =  0x02,
    Lba0 =       0x03,
    Lba1 =       0x04,
    Lba2 =       0x05,
    HddEvSel =   0x06,
    CommandAndStatus =    0x07,
    SecCount1 =  0x08,
    Lba3 =       0x09,
    Lba4  =      0x0A,
    Lba5   =     0x0B,
    ControlAndAltStatus =    0x0C,
    DevAddress = 0x0D,
}

#[repr(u8)]
#[allow(dead_code)]
enum AtaCommand {
    ReadPio          = 0x20,
    ReadPioExt      = 0x24,
    ReadDma          = 0xC8,
    ReadDmaExt      = 0x25,
    WritePio         = 0x30,
    WritePioExt     = 0x34,
    WriteDma         = 0xCA,
    WriteDmaExt     = 0x35,
    CacheFlush       = 0xE7,
    CacheFlushExt   = 0xEA,
    Packet            = 0xA0,
    IdentifyPacket   = 0xA1,
    Identify          = 0xEC,
}

#[repr(u8)]
#[allow(dead_code)]
enum AtaStatus {
    Busy               = 0x80,
    DriveReady         = 0x40,
    DriveWriteFault    = 0x20,
    DriveSeekComplete  = 0x10,
    DataRequestReady   = 0x08,
    CorrectedData      = 0x04,
    Index              = 0x02,
    Error              = 0x01,
}

static mut CHANNELS: [IDEChannelRegister; 2] = [IDEChannelRegister{ io_base: 0, ctrl: 0, bm_ide: 0, no_interrupt: 0 }; 2];

unsafe fn ide_write(channel: ATAChannel, reg: AtaRegister, data: u8) {
    if (reg as u8) > 0x07 && (reg as u8) < 0x0C {
        ide_write(channel, AtaRegister::ControlAndAltStatus, 0x80 | CHANNELS[channel as usize].no_interrupt);
    }

    if (reg as u8) < 0x08 {
        port::write(CHANNELS[channel as usize].io_base + reg as u16 - 0x00, data);
    }
    else if (reg as u8) < 0x0C {
        port::write(CHANNELS[channel as usize].io_base + reg as u16 - 0x06, data);
    }
    else if (reg as u8) < 0x0E {
        port::write(CHANNELS[channel as usize].ctrl + reg as u16 - 0x0A, data);
    }
    else if (reg as u8) < 0x16 {
        port::write(CHANNELS[channel as usize].bm_ide + reg as u16 - 0x0E, data);
    }

    if (reg as u8) > 0x07 && (reg as u8) < 0x0C {
        ide_write(channel, AtaRegister::ControlAndAltStatus, CHANNELS[channel as usize].no_interrupt);
    }
}

unsafe fn ide_read(channel: ATAChannel, reg: AtaRegister) -> u8 {
    let mut result: u8 = 0;
    if (reg as u8) > 0x07 && (reg as u8) < 0x0C {
        ide_write(channel, AtaRegister::ControlAndAltStatus, 0x80 | CHANNELS[channel as usize].no_interrupt);
    }

    if (reg as u8) < 0x08 {
        result = port::read(CHANNELS[channel as usize].io_base + reg as u16 - 0x00);
    } else if (reg as u8) < 0x0C {
        result = port::read(CHANNELS[channel as usize].io_base + reg as u16 - 0x06);
    }
    else if (reg as u8) < 0x0E {
        result = port::read(CHANNELS[channel as usize].ctrl + reg as u16 - 0x0A);
    }
    else if (reg as u8) < 0x16 {
        result = port::read(CHANNELS[channel as usize].bm_ide + reg as u16 - 0x0E);
    }

    if (reg as u8) > 0x07 && (reg as u8) < 0x0C {
        ide_write(channel, AtaRegister::ControlAndAltStatus, CHANNELS[channel as usize].no_interrupt);
    }
    result
}

unsafe fn ide_read_buffer(channel: ATAChannel, reg: AtaRegister, words: u16, buffer: &mut [u16; 1024]) {
    if (reg as u8) > 0x07 && (reg as u8) < 0x0C {
        ide_write(channel, AtaRegister::ControlAndAltStatus, 0x80 | CHANNELS[channel as usize].no_interrupt);
    }

    let mut port: Option<Port> = if (reg as u8) < 0x08 {
        Some(Port::new(CHANNELS[channel as usize].io_base + reg as u16 - 0x00))
    } else if (reg as u8) < 0x0C {
        Some(Port::new(CHANNELS[channel as usize].io_base + reg as u16 - 0x06))
    } else if (reg as u8) < 0x0E {
        Some(Port::new(CHANNELS[channel as usize].ctrl + reg as u16 - 0x0A))
    } else if (reg as u8) < 0x0E {
        Some(Port::new(CHANNELS[channel as usize].bm_ide + reg as u16 - 0x0E))
    } else {
        None
    };

    for i in 0..words as usize {
        let res_u16 = port.as_mut().unwrap().read_u16();
        buffer[i] = res_u16;
    }

    if (reg as u8) > 0x07 && (reg as u8) < 0x0C {
        ide_write(channel, AtaRegister::ControlAndAltStatus, CHANNELS[channel as usize].no_interrupt);
    }
}

fn construct_u32(input: [u16; 2]) -> u32 {
    (input[1] as u32) << 16 | input[0] as u32
}

fn get_u16_from_buffer(buffer: [u16; 1024], offset: IdentifyBufferOffset) -> u16 {
    buffer[offset as usize]
}

fn get_u32_from_buffer(buffer: [u16; 1024], offset: IdentifyBufferOffset) -> u32 {
    construct_u32(buffer[offset as usize .. offset as usize + 2].try_into().unwrap())
}

pub(crate) async fn ide_initialize(_prog_if: u8) -> Vec<IDEDevice> {
    log::info!("IDE initializing");
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
        ide_write(ATAChannel::Primary, AtaRegister::ControlAndAltStatus, 2);
        ide_write(ATAChannel::Secondary, AtaRegister::ControlAndAltStatus, 2);
    }

    let mut drives = Vec::new();

    for channel in [ATAChannel::Primary, ATAChannel::Secondary] {
        for drive in [DriveType::Master, DriveType::Slave] {
            log::info!("Checking {:?} {:?}", channel, drive);

            let mut err: u8 = 0;
            let interface_type = IDEInterfaceType::Ata;
            let mut status: u8;

            unsafe {
                ide_write(channel, AtaRegister::HddEvSel, 0xA0 | ((drive as u8) << 4));
            }
            sleep_for(1).await;

            unsafe {
                ide_write(channel, AtaRegister::CommandAndStatus, AtaCommand::Identify as u8)
            }
            sleep_for(1).await;

            unsafe {
                if ide_read(channel, AtaRegister::CommandAndStatus) == 0 { continue; } // No Device

                loop {
                    status = ide_read(channel, AtaRegister::CommandAndStatus);
                    log::info!("status: {}", status);
                    if (status & AtaStatus::Error as u8) != 0 {
                        err = 1;
                        break; // Device is not ATA
                    }
                    if ((status & AtaStatus::Busy as u8) == 0) && ((status & AtaStatus::DataRequestReady as u8) != 0) {
                        break; // Everything is good
                    }
                }

                if err != 0 {
                    // It's a place to probe for ATAPI Devices, but I don't want it
                    continue;
                }
            }

            let mut ide_buf: [u16; 1024] = [0; 1024];

            unsafe {
                ide_read_buffer(channel, AtaRegister::Data, 256, &mut ide_buf);
            }

            let command_sets = get_u32_from_buffer(ide_buf, IdentifyBufferOffset::Commandsets);
            let size: u32;
            let mut model: [u8; 41] = [0; 41];
            let enabled_48bit: bool;

            if command_sets & (1 << 26) != 0 {
                // Device uses 48-Bit Addressing:
                enabled_48bit = true;
                size = get_u32_from_buffer(ide_buf, IdentifyBufferOffset::MaxLbaExt);
            } else {
                // Device uses CHS or 28-bit Addressing:
                enabled_48bit = false;
                size = get_u32_from_buffer(ide_buf, IdentifyBufferOffset::MaxLba);
            }

            let mut i: usize = 0;
            while i < 40 {
                let chars: [u8; 2] = ide_buf[IdentifyBufferOffset::Model as usize + i / 2].to_be_bytes();

                model[i] = chars[0];
                model[i + 1] = chars[1];
                i += 2;
            }
            model[40] = 0;

            drives.push(IDEDevice {
                reserved: true,
                channel,
                drive,
                interface_type,
                signature: get_u16_from_buffer(ide_buf, IdentifyBufferOffset::DeviceType),
                capabilities: get_u16_from_buffer(ide_buf, IdentifyBufferOffset::Capabilities),
                command_sets,
                size,
                model,
                enabled_48bit,
            });
        }
    }

    for drive in &drives {
        if drive.reserved == true {
            log::info!("Found ATA Drive {} kB - '{}'. 48-bit addressing: {}", (drive.size * 512) / 1024, core::str::from_utf8(&drive.model).unwrap(), drive.enabled_48bit);
        }
    }
    drives
}