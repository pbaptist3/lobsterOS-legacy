use alloc::{format, vec};
use alloc::alloc::alloc_zeroed;
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::cmp::min;
use core::error::Error;
use core::fmt::{Display, Formatter};
use core::mem::{size_of, transmute};
use core::pin::Pin;
use core::ptr::{addr_of, read_unaligned, slice_from_raw_parts, slice_from_raw_parts_mut};
use x86_64::structures::paging::{Mapper, OffsetPageTable, Page, PageTableFlags, PhysFrame, Size4KiB, Translate};
use x86_64::structures::paging::frame::PhysFrameRange;
use x86_64::structures::paging::mapper::MappedFrame;
use x86_64::{align_down, align_up, VirtAddr};
use crate::elf::ElfVerifyError::{BadMagicNum, Elf32Bit, BadSliceSize, BadEndianness, BadArch};
use crate::elf::ProgramHeaderType::Load;
use crate::{MAPPER, serial_println};
use crate::memory::BootInfoFrameAllocator;

const MAGIC_NUM: &[u8; 4] = b"\x7FELF";

/// 64-bit elf header
#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct ElfHeader {
    magic_num: [u8; 4],
    bits: u8,
    endianness: u8,
    elf_header_version: u8,
    os_abi: u8,
    _reserved: [u8; 8],
    elf_type: u16,
    instruction_set: u16,
    elf_version: u32,
    program_entry: u64,
    program_header_table: u64,
    section_header_table: u64,
    flags: u32,
    header_size: u16,
    program_entry_size: u16,
    program_entry_count: u16,
    section_entry_size: u16,
    section_entry_count: u16,
    section_names_idx: u16,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct ProgramHeader {
    header_type: ProgramHeaderType,
    flags: u32,
    offset: u64,
    virt_addr: u64,
    _reserved: [u8; 8],
    file_size: u64,
    mem_size: u64,
    align: u64,
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ProgramHeaderType {
    Null = 0x0,
    Load = 0x1,
    Dynamic = 0x2,
    Interp = 0x3,
    Note = 0x4,
    SHLib = 0x5,
    PHDR = 0x6,
    LoProc = 0x70000000,
    HiProc = 0x7FFFFFFF,
}

#[derive(Copy, Clone, Debug)]
enum ElfVerifyError {
    BadSliceSize(usize),
    BadMagicNum,
    Elf32Bit,
    BadEndianness,
    BadArch,
}

impl Display for ElfVerifyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "failed to verify elf header: {}", match self {
            BadSliceSize(size) => format!("incorrect slice size of {} provided", size),
            BadMagicNum => "incorrect ELF magic num".to_string(),
            Elf32Bit => "only 64-bit elf is supported".to_string(),
            BadEndianness => "ELF is not little endian".to_string(),
            BadArch => "ELF is not x64".to_string(),
        })
    }
}

impl Error for ElfVerifyError {}

impl ElfHeader {
    /// creates an elf header reference from a byte slice
    /// requires that bytes is a valid elf header struct
    unsafe fn from_slice(bytes: &[u8]) -> Result<&Self, ElfVerifyError> {
        if bytes.len() != size_of::<Self>() {
            return Err(BadSliceSize(bytes.len()));
        }

        let header = &*(bytes.as_ptr() as *const Self);

        if header.magic_num != *MAGIC_NUM {
            return Err(BadMagicNum);
        }
        if header.bits != 2 {
            return Err(Elf32Bit);
        }
        if header.endianness != 1 {
            return Err(BadEndianness);
        }
        if header.instruction_set != 0x3E {
            return Err(BadArch);
        }

        Ok(header)
    }
}

pub unsafe fn do_stuff(
    file: &Vec<u8>, mapper: &mut OffsetPageTable, frame_allocator: &mut BootInfoFrameAllocator
) -> (BTreeMap<u64, [u8; 0x1000]>, u64) {
    let elf_header = ElfHeader::from_slice(&file[..size_of::<ElfHeader>()])
        .expect("invalid elf header");
    serial_println!("{:#?}", elf_header);

    // make array of program headers
    let program_headers = &*slice_from_raw_parts(
        addr_of!(file[elf_header.program_header_table as usize]) as *const ProgramHeader,
        elf_header.program_entry_count as usize
    );

    let mut regions: BTreeMap<u64, [u8; 0x1000]> = BTreeMap::new();
    for program_header in program_headers {
        serial_println!("{:?}", program_header);
        if read_unaligned(addr_of!(program_header.header_type)) != Load {
            continue;
        }

        let mem_size = program_header.mem_size;
        let offset = program_header.offset;
        let file_size = program_header.file_size;
        let virt_addr = program_header.virt_addr;

        let page_count = mem_size / 0x1000 + 1;
        // allocate pages
        for i in 0..page_count {
            let page_addr = i*0x1000 + align_down(virt_addr, 0x1000);
            if !regions.contains_key(&page_addr) {
                let layout = Layout::from_size_align_unchecked(0x1000, 0x1000);
                let mut page: [u8; 0x1000] = *(alloc_zeroed(layout) as *mut [u8; 0x1000]);
                regions.insert(page_addr, page);
            }
        }

        // copy over data
        let mut working_offset = 0;
        let file_data_page_count = file_size / 0x1000 + 1;
        for i in 0..file_data_page_count {
            let lower_bound = offset + working_offset;
            let page_addr = align_down(virt_addr + working_offset, 0x1000);
            let write_size = min(page_addr+0x1000, file_size - working_offset + 1);
            let lower_bound_offset = lower_bound % 0x1000;
            let mut page = regions.get_mut(&page_addr).unwrap();
            serial_println!(
                "lb: {:x}\tpa: {:x}\tws: {:x}\tlbo: {:x}",
                lower_bound, page_addr, write_size, lower_bound_offset
            );
            page[(lower_bound_offset as usize)..((lower_bound_offset+write_size) as usize)]
                .copy_from_slice(
                    &file[(lower_bound as usize)..((lower_bound+write_size) as usize)]
                );
            serial_println!("{:x?}", &file[(lower_bound as usize)..((lower_bound+write_size) as usize)]);
            serial_println!("{:x?}", &page[(lower_bound_offset as usize)..((lower_bound_offset+write_size) as usize)]);
            working_offset += write_size;
        }
    }

    // map pages
    for (virt_addr, page) in regions.iter() {
        let phys_addr = mapper.translate_addr(VirtAddr::from_ptr(page.as_ptr()))
            .unwrap();
        serial_println!("p_addr: {:x}", phys_addr.as_u64());
        mapper
            .map_to(
                Page::<Size4KiB>::from_start_address(VirtAddr::new(*virt_addr)).unwrap(),
                PhysFrame::containing_address(phys_addr),
                PageTableFlags::PRESENT
                    | PageTableFlags::WRITABLE
                    | PageTableFlags::USER_ACCESSIBLE,
                frame_allocator
            )
            .unwrap()
            .flush();
    }

    (regions, elf_header.program_entry)
}