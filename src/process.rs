use crate::memory::BootInfoFrameAllocator;
use crate::threading::thread::Thread;
use crate::{allocator, gdt, memory, println, serial_println, userspace};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::arch::asm;
use x86_64::structures::paging::{FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB, Translate};
use x86_64::{PhysAddr, PrivilegeLevel, VirtAddr};
use x86_64::registers::control::Cr3;
use x86_64::registers::segmentation::Segment;
use x86_64::structures::paging::mapper::MapperFlush;

const USERSPACE_VIRT_BASE: u64 = 0x400000;
const USERSPACE_STACK: u64 = 0x810000;
const USERSPACE_STACK_BASE: u64 = 0x800000;

pub struct Process {
    page_table: Box<PageTable>
}

impl Process {
    pub unsafe fn new(
        mapper: &mut OffsetPageTable,
        frame_allocator: &mut BootInfoFrameAllocator,
    ) -> Self {
        // create page table and map process to it
        let (page_table, page_table_addr, entry_offset) =
            Self::map_process(mapper, frame_allocator)
            .expect("Failed to create virtual memory for process");

        // activate page table
        let (_, cr3_flags) = Cr3::read();
        Cr3::write(
            PhysFrame::containing_address(page_table_addr),
            cr3_flags
        );

        // jump to usermode
        switch_to_usermode(
            VirtAddr::new(USERSPACE_VIRT_BASE + entry_offset),
            VirtAddr::new(USERSPACE_STACK)
        );

        Self {
            page_table
        }
    }

    /// Creates a page table for a new process, copying over kernel pages
    fn new_page_table(
        current_page_table: &PageTable,
        frame_allocator: &mut BootInfoFrameAllocator,
        mapper: &mut OffsetPageTable
    ) -> Option<Box<PageTable>> {
        let mut page_table = Box::new(PageTable::new());
        page_table.zero();
        page_table[0].set_addr(
            frame_allocator.allocate_frame()?.start_address(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        // map currently used kernel addresses
        // assumes page table entry 0 is a page table (just allocated so why not?)
        let mut page_table_0 = unsafe {
            &mut *((page_table[0].addr().as_u64() + mapper.phys_offset().as_u64()) as *mut PageTable)
        };
        // assumes the kernel level 4 table is a level 4 table
        let current_table_0 = unsafe {
            &*((current_page_table[0].addr().as_u64() + mapper.phys_offset().as_u64()) as *const PageTable)
        };

        page_table_0.zero();
        for i in 1..512 {
            page_table_0[i] = current_table_0[i].clone();
        }

        for i in 1..512 {
            page_table[i] = current_page_table[i].clone();
        }

        Some(page_table)
    }

    /// Creates process virtual memory and maps to it
    unsafe fn map_process(
        old_mapper: &mut OffsetPageTable,
        frame_allocator: &mut BootInfoFrameAllocator,
    ) -> Option<(Box<PageTable>, PhysAddr, u64)> {
        let mut page_table = memory::active_level_4_table(old_mapper.phys_offset());
        let userspace_fn_in_kernel = VirtAddr::new(userspace::example_process as *const () as u64);
        let userspace_fn_phys = old_mapper
            .translate_addr(userspace_fn_in_kernel)
            .expect("Failed to translate to physical address");
        let userspace_fn_frame: PhysFrame<Size4KiB> = PhysFrame::containing_address(userspace_fn_phys);
        let userspace_fn_offset = userspace_fn_phys - userspace_fn_frame.start_address();

        let mut new_page_table =
            Self::new_page_table(page_table, frame_allocator, old_mapper)?;

        let new_page_table_virt_addr = VirtAddr::new(&*new_page_table as *const _ as u64);
        let new_page_table_addr = old_mapper.translate_addr(new_page_table_virt_addr)?;
        let mut new_mapper = OffsetPageTable::new(&mut *new_page_table, old_mapper.phys_offset());

        // map a stack
        let start_page = Page::containing_address(VirtAddr::new(USERSPACE_STACK_BASE));
        let end_page = Page::containing_address(VirtAddr::new(USERSPACE_STACK));
        let stack_range = Page::range_inclusive(start_page, end_page);
        assert!(!stack_range.is_empty());
        for page in stack_range {
            new_mapper
                .map_to(
                    page,
                    frame_allocator.allocate_frame()?,
                    PageTableFlags::PRESENT
                        | PageTableFlags::WRITABLE
                        | PageTableFlags::USER_ACCESSIBLE,
                    frame_allocator
                )
                .unwrap()
                .flush();
        }

        // TEMP
        // map the userspace fn
        let map_result = new_mapper
            .map_to(
                Page::containing_address(VirtAddr::new(USERSPACE_VIRT_BASE)),
                userspace_fn_frame,
                PageTableFlags::PRESENT
                    | PageTableFlags::WRITABLE
                    | PageTableFlags::USER_ACCESSIBLE,
                frame_allocator,
            )
            .unwrap();
        map_result.flush();

        Some((new_page_table, new_page_table_addr, userspace_fn_offset))
    }
}

unsafe fn switch_to_usermode(code: VirtAddr, stack_end: VirtAddr) {
    use x86_64::instructions::tlb;
    use x86_64::registers::model_specific::{Msr, LStar, SFMask};

    let (cs_index, ds_index) = gdt::set_usermode_segments();
    tlb::flush_all();

    asm!("\
        push rax
        push rsi
        push 0x200
        push rdx
        push rdi
        iretq",
    in("rdi") code.as_u64(),
    in("rsi") stack_end.as_u64(),
    in("dx") cs_index,
    in("ax") ds_index
    );
}