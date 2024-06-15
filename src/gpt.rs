use alloc::boxed::Box;
use crate::ide::BlockDevice;

pub fn parse_gpt(device: Box<dyn BlockDevice>) {
    log::info!("[gpt] Parsing GPT for {}kb block {:?} device on channel {:?}", (device.size() * 512) / 1024, device.drive_type(), device.channel());
}