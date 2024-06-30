use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::cmp::min;
use crate::bits::set_bit;

enum Error {
    PartitionTooSmall,
    OutOfBlocks,
    OutOfFiles,
    InvalidFileOffset,
}

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error>;
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error>;
    fn flush(&mut self) -> Result<(), Error>;
}

pub enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Error>;
}

#[repr(C, packed)]
struct SuperBlock {
    signature: u64, // Expect FERR__FS
    total_blocks: u64,
}

#[repr(C, packed)]
struct Inode {
    name: String,
    size: usize,
    blocks: Vec<u32>,
}

// Supernode 512 bytes (signature and total blocks)
// Inode bitmap 8 bytes
// data blocks bitmap vec of u64 with size (total blocks / 64)
// 64 Inodes
// Data blocks

pub struct FileSystem<Partition: Read + Write + Seek> {
    partition: Partition,
    total_blocks: u64,
    inodes_bitmap: u64, // max 64 files
    inodes: [Inode; 64],
    blocks_bitmap: Vec<u64>,
}

pub struct File<'a, T: Read + Write + Seek + 'a> {
    offset: u64,
    inode_idx: usize,
    fs: &'a FileSystem<T>
}

pub const BLOCK_SIZE: usize = 512;

impl<T: Read + Write + Seek> FileSystem<T> {
    pub fn new(mut partition: T) -> Result<Self, Error> {
        let mut buffer = [0u8; BLOCK_SIZE];
        if partition.read(&mut buffer)? != BLOCK_SIZE {
            return Err(Error::PartitionTooSmall);
        }

        if buffer.split_at(8).0 == "FERR__FS".as_bytes() {
            // existing FS
            unimplemented!();
        } else {
            // Need to format partition
            let total_bytes = partition.seek(SeekFrom::End(0))?;
            let total_blocks = total_bytes / BLOCK_SIZE as u64;

            let mut blocks_bitmap: Vec<u64> = Vec::new();
            for _ in 0..=(total_blocks / 64) {
                blocks_bitmap.push(0x0);
            }
            partition.seek(SeekFrom::Start(0))?;
            partition.write("FERR__FS".as_bytes())?;
            partition.write(total_blocks.to_le_bytes().as_ref())?;
            partition.write(0.to_le_bytes().as_ref())?; // total inodes
            partition.seek(SeekFrom::Start(BLOCK_SIZE))?;
            for b in blocks_bitmap {
                partition.write(b.to_le_bytes().as_ref())?;
            }
            Ok(FileSystem{ partition, total_blocks, inodes_bitmap: 0, blocks_bitmap, inodes: Vec::new() })
        }
    }

    fn find_free_block(&self) -> Option<usize> {
        for (idx, block) in self.blocks_bitmap.iter().enumerate() {
            for i in 0..64u8 {
                if (*block >> (i as u64)) & 1 == 0 {
                    if idx * 64 + (i as usize) >= self.total_blocks as usize {
                        return None;
                    }
                    return Some(idx * 64 + i as usize);
                }
            }
        }
        None
    }

    fn find_free_inode(&self) -> Option<usize> {
        for i in 0..64u8 {
            if self.inodes_bitmap >> (i as u64) & 1 == 0 {
                return Some(i as usize);
            }
        }
        None
    }

    pub fn create_file(&mut self, file_name: &str) -> Result<File<T>, Error> {
        let free_block = self.find_free_block().ok_or(Error::OutOfBlocks)?;
        let free_inode = self.find_free_inode().ok_or(Error::OutOfFiles)?;

        set_bit(&mut self.inodes_bitmap, free_inode as u8, true);
        set_bit(&mut self.blocks_bitmap[free_block / 64], (free_block % 64) as u8, true);

        self.inodes.push(Inode{name: file_name.parse().unwrap(), size: 0, blocks: vec![free_block as u32]});

        Ok(File{offset: 0, inode_idx: free_inode, fs: self })
    }
}

impl<'a, T: Read + Write + Seek> Write for File<'a, T> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        let total_blocks_required_to_allocate = (self.offset as usize % BLOCK_SIZE + buf.len() + BLOCK_SIZE - 1) / BLOCK_SIZE - 1;

        let inode = self.fs.inodes[self.inode_idx];
        let block_idx_to_write = self.offset as usize / BLOCK_SIZE;
        let first_block_to_write = *inode.blocks.get(block_idx_to_write).ok_or(Error::InvalidFileOffset)?;

        let mut blocks_to_write = Vec::with_capacity(1 + total_blocks_required_to_allocate);
        blocks_to_write[0] = first_block_to_write as usize;
        for _ in 0..total_blocks_required_to_allocate {
            blocks_to_write.push(self.fs.find_free_block().ok_or(Error::OutOfBlocks)?)
        }

        let remain_offset = self.offset as usize - block_idx_to_write * BLOCK_SIZE;
        let mut partition = &self.fs.partition;
        partition.seek(SeekFrom::Start((*first_block_to_write as usize * BLOCK_SIZE + remain_offset) as u64))?;

        let mut offset = 0;
        let mut current_offset = remain_offset;
        let mut current_block_index = 0;

        while offset < buf.len() {
            let block_address = blocks_to_write[current_block_index];
            current_block_index += 1;

            let block_remaining = BLOCK_SIZE - (current_offset % BLOCK_SIZE);
            let end = min(offset + block_remaining, buf.len());
            let chunk = &buf[offset..end];

            partition.write(chunk)?;

            // If the chunk is smaller than the block size and we are at the end of the buffer,
            // pad the rest of the block with zeros
            if chunk.len() < block_remaining && end == buf.len() {
                let padding = vec![0; block_remaining - chunk.len()];
                partition.write(&padding)?;
            }

            set_bit(&mut self.fs.blocks_bitmap[block_address / 64], (block_address % 64) as u8, true);

            offset += chunk.len();
            current_offset += chunk.len();
        }

        return Ok(buf.len());
    }

    fn flush(&mut self) -> Result<(), Error> {
        return Ok(())
    }
}

