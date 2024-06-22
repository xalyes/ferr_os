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

#[allow(dead_code)]
pub trait BlockDevice {
    fn read(&self, lba: u32, num: u8) -> Result<Vec<[u16; 256]>, AtaError>;

    fn write(&self, lba: u32, data: Vec<[u16; 256]>) -> Result<(), AtaError>;

    fn size(&self) -> u32;

    fn model(&self) -> [u8; 41];

    fn channel(&self) -> ATAChannel;

    fn drive_type(&self) -> DriveType;
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
#[derive(Debug)]
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

#[repr(u8)]
#[allow(dead_code)]
#[derive(Debug)]
pub enum AtaError {
    NoError = 0,
    DeviceFault = 19,
    NoAddressMarkFound = 7, // 'Track 0 not found' or 'Media change request' or 'Media changed'
    NoMediaOrMediaError = 3,
    CommandAborted = 20,
    IdMarkNotFound = 21,
    UncorrectableDataError = 22,
    BadSectors = 13,
    ReadsNothing = 23,
    WriteProtected = 8,

    OutOfRange = 255,
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

pub(crate) async fn ide_initialize(_prog_if: u8) -> Vec<impl BlockDevice> {
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
        log::info!("Found ATA Drive {} kB - '{}'. 48-bit addressing: {}", (drive.size * 512) / 1024, core::str::from_utf8(&drive.model).unwrap(), drive.enabled_48bit);
    }
    drives
}

unsafe fn ide_polling(channel: ATAChannel, advanced_check: bool) -> AtaError {
    // Delay 400 nanosecond for BSY to be set:
    for _ in 0..4 {
        // Reading the Alternate Status port wastes 100ns; loop four times.
        ide_read(channel, AtaRegister::ControlAndAltStatus);
    }

    while (ide_read(channel, AtaRegister::CommandAndStatus) & AtaStatus::Busy as u8) != 0 {}

    if advanced_check {
        let status = ide_read(channel, AtaRegister::CommandAndStatus);

        if (status & AtaStatus::Error as u8) != 0 {
            let err = ide_read(channel, AtaRegister::ErrorAndFeatures);

            if (err & 0x01) != 0  { return AtaError::NoAddressMarkFound; }
            if (err & 0x02) != 0  { return AtaError::NoMediaOrMediaError; }
            if (err & 0x04) != 0  { return AtaError::CommandAborted; }
            if (err & 0x08) != 0  { return AtaError::NoMediaOrMediaError; }
            if (err & 0x10) != 0  { return AtaError::IdMarkNotFound; }
            if (err & 0x20) != 0  { return AtaError::NoMediaOrMediaError; }
            if (err & 0x40) != 0  { return AtaError::UncorrectableDataError; }
            if (err & 0x80) != 0  { return AtaError::BadSectors; }

            return AtaError::DeviceFault;
        }

        if (status & AtaStatus::DriveWriteFault as u8) != 0 {
            return AtaError::DeviceFault;
        }

        // BSY = 0; DF = 0; ERR = 0 so we should check for DRQ now.
        /*if (status & AtaStatus::DataRequestReady as u8) != 0 {
            return AtaError::ReadsNothing; // DRQ should be set
        }*/
    }

    return AtaError::NoError;
}

#[allow(dead_code)]
enum LbaMode {
    Lba48,
    Lba28,
    Chs
}

impl IDEDevice {
    unsafe fn io_prepare(&self, lba: u32, numsects: u8, dma: bool, is_write: bool) -> LbaMode {
        CHANNELS[self.channel as usize].no_interrupt = 0x02;
        ide_write(self.channel, AtaRegister::ControlAndAltStatus, CHANNELS[self.channel as usize].no_interrupt);

        let lba_mode;
        let mut lba_io = [0u8; 6];
        let head: u8;
        if lba >= 0x10000000 { // with this lba drive must support LBA48
            // LBA48
            lba_mode = LbaMode::Lba48;
            lba_io[0] = ((lba & 0x000000FF) >> 0) as u8;
            lba_io[1] = ((lba & 0x0000FF00) >> 8) as u8;
            lba_io[2] = ((lba & 0x00FF0000) >> 16) as u8;
            lba_io[3] = ((lba & 0xFF000000) >> 24) as u8;
            lba_io[4] = 0; // These Registers are not used here.
            lba_io[5] = 0; // These Registers are not used here.
            head = 0;      // Lower 4-bits of HDDEVSEL are not used here.

        } else if (self.capabilities & 0x200) != 0 {
            // LBA28
            lba_mode = LbaMode::Lba28;
            lba_io[0] = ((lba & 0x000000FF) >> 0) as u8;
            lba_io[1] = ((lba & 0x0000FF00) >> 8) as u8;
            lba_io[2] = ((lba & 0x00FF0000) >> 16) as u8;
            lba_io[3] = 0; // These Registers are not used here.
            lba_io[4] = 0; // These Registers are not used here.
            lba_io[5] = 0; // These Registers are not used here.
            head = ((lba & 0xF000000) >> 24) as u8;
        } else {
            // CHS
            //lba_mode = LbaMode::Chs;
            unimplemented!();
        }

        // wait if busy
        while (ide_read(self.channel, AtaRegister::CommandAndStatus) & AtaStatus::Busy as u8) != 0 {}

        let slavebit: u8 = match self.drive { DriveType::Master => 0b0000, DriveType::Slave => 0b10000 };
        match lba_mode {
            LbaMode::Chs => unimplemented!(),
            LbaMode::Lba28 => {
                ide_write(self.channel, AtaRegister::HddEvSel, 0xE0 | slavebit | head);
            }
            LbaMode::Lba48 => {
                ide_write(self.channel, AtaRegister::HddEvSel, 0xE0 | slavebit | head);

                ide_write(self.channel, AtaRegister::SecCount1,   0);
                ide_write(self.channel, AtaRegister::Lba3,   lba_io[3]);
                ide_write(self.channel, AtaRegister::Lba4,   lba_io[4]);
                ide_write(self.channel, AtaRegister::Lba5,   lba_io[5]);
            }
        }
        ide_write(self.channel, AtaRegister::SecCount0,   numsects);
        ide_write(self.channel, AtaRegister::Lba0,   lba_io[0]);
        ide_write(self.channel, AtaRegister::Lba1,   lba_io[1]);
        ide_write(self.channel, AtaRegister::Lba2,   lba_io[2]);

        let command = match lba_mode {
            LbaMode::Chs => unimplemented!(),
            LbaMode::Lba28 => {
                match (dma, is_write) {
                    (false, false) => AtaCommand::ReadPio,
                    (false, true) => AtaCommand::WritePio,
                    (true, false) => AtaCommand::ReadDma,
                    (true, true) => AtaCommand::WriteDma
                }
            }
            LbaMode::Lba48 => {
                match (dma, is_write) {
                    (false, false) => AtaCommand::ReadPioExt,
                    (false, true) => AtaCommand::WritePioExt,
                    (true, false) => AtaCommand::ReadDmaExt,
                    (true, true) => AtaCommand::WriteDmaExt
                }
            }
        };

        ide_write(self.channel, AtaRegister::CommandAndStatus, command as u8);

        lba_mode
    }

    unsafe fn write_impl(&self, lba: u32, data: Vec<[u16; 256]>) -> Result<(), AtaError> {
        // DMA is not implemented for now
        let dma = false;

        let lba_mode = self.io_prepare(lba, data.len() as u8, dma, true);

        if dma {
            unimplemented!();
        } else {
            let mut port = Port::new(CHANNELS[self.channel as usize].io_base);

            for sector in data {
                ide_polling(self.channel, false);
                for word in sector {
                    port.write_u16(word);
                }
            }

            match lba_mode {
                LbaMode::Lba48 => ide_write(self.channel, AtaRegister::CommandAndStatus, AtaCommand::CacheFlushExt as u8),
                LbaMode::Chs | LbaMode::Lba28 => ide_write(self.channel, AtaRegister::CommandAndStatus, AtaCommand::CacheFlush as u8)
            }
            match ide_polling(self.channel, false) {
                AtaError::NoError => Ok(()),
                err @ _ => Err(err)
            }
        }
    }
    unsafe fn read_impl(&self, lba: u32, numsects: u8) -> Result<Vec<[u16; 256]>, AtaError> {
        // DMA is not implemented for now
        let dma = false;

        self.io_prepare(lba, numsects, dma, false);

        if dma {
            unimplemented!();
        } else {
            let mut port = Port::new(CHANNELS[self.channel as usize].io_base);

            let mut buffer = [0u16; 256];
            let mut result = Vec::new();
            result.reserve(numsects as usize);

            for _ in 0..numsects {
                let err = ide_polling(self.channel, true);
                match err {
                    AtaError::NoError => {
                        for i in 0..256 {
                            let word = port.read_u16();
                            buffer[i] = word;
                        }
                        result.push(buffer.clone());
                    },
                    _ => { return Err(err); }
                }
            }

            return Ok(result);
        }
    }
}

impl BlockDevice for IDEDevice {
    fn read(&self, lba: u32, num: u8) -> Result<Vec<[u16; 256]>, AtaError> {
        if lba + num as u32 > self.size {
            return Err(AtaError::OutOfRange);
        }

        unsafe { self.read_impl(lba, num) }
    }

    fn write(&self, lba: u32, data: Vec<[u16; 256]>) -> Result<(), AtaError> {
        if lba + data.len() as u32 > self.size {
            return Err(AtaError::OutOfRange);
        }

        unsafe { self.write_impl(lba, data) }
    }

    fn size(&self) -> u32 {
        self.size
    }

    fn model(&self) -> [u8; 41] {
        self.model
    }

    fn channel(&self) -> ATAChannel {
        self.channel
    }

    fn drive_type(&self) -> DriveType {
        self.drive
    }
}