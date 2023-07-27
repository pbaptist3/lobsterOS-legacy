use crate::memory::BootInfoFrameAllocator;

use crate::{gdt, memory, println, serial_println, userspace};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::arch::asm;
use core::mem::size_of_val;
use core::pin::Pin;
use core::ptr::addr_of;
use x86_64::structures::paging::{FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB, Translate};
use x86_64::{PhysAddr, VirtAddr};
use x86_64::registers::control::Cr3;
use crate::elf::ProgramHeader;
use crate::threading::thread::State;

const USERSPACE_VIRT_BASE: u64 = 0x400000;
const USERSPACE_STACK: u64 = 0x810000;
const USERSPACE_STACK_BASE: u64 = 0x800000;

pub struct Process {
    page_table: Box<PageTable>,
    task_state: TaskState,
    process_state: Option<ProcessState>,
    page_table_addr: PhysAddr,
    entry_offset: u64,
    regions: BTreeMap<u64, [u8; 0x1000]>,
}

impl Process {
    /*unsafe fn new(
        frame_allocator: &mut BootInfoFrameAllocator,
    ) -> Self {
        let mapper = crate::MAPPER.get()
            .expect("mapper not initialized");

        // create page table and map process to it
        let (page_table, page_table_addr, entry_offset) =
            Self::map_process(mapper, frame_allocator)
            .expect("Failed to create virtual memory for process");

        Self {
            page_table,
            task_state: TaskState::READY,
            process_state: None,
            page_table_addr,
            entry_offset,
            regions: Vec::new()
        }
    }*/

    pub unsafe fn spawn_from_file(
        file: &Vec<u8>, frame_allocator: &mut BootInfoFrameAllocator
    ) -> Self {
        let mapper = crate::MAPPER.get()
            .expect("mapper not initialized");

        let current_page_table = memory::active_level_4_table(
            mapper.phys_offset().as_u64()
        );
        let mut page_table = Self::new_page_table(current_page_table, frame_allocator, mapper)
            .expect("failed to create page table");
        let page_table_virt_addr = VirtAddr::new(addr_of!(*page_table) as u64);
        let page_table_addr = mapper.translate_addr(page_table_virt_addr)
            .unwrap();
        let mut new_mapper = OffsetPageTable::new(
            &mut page_table, mapper.phys_offset()
        );

        Self::map_stack(&mut new_mapper, frame_allocator);

        let (regions, entry_point) = crate::elf::do_stuff(
            file, &mut new_mapper, frame_allocator
        );
        serial_println!("{:x}", entry_point);

        Self {
            page_table,
            task_state: TaskState::READY,
            process_state: None,
            page_table_addr,
            entry_offset: entry_point,
            regions,
        }
    }

    /// Creates a page table for a new process, copying over kernel pages
    fn new_page_table(
        current_page_table: &PageTable,
        frame_allocator: &mut BootInfoFrameAllocator,
        mapper: &OffsetPageTable
    ) -> Option<Box<PageTable>> {
        let mut page_table = Box::new(PageTable::new());
        page_table.zero();
        page_table[0].set_addr(
            frame_allocator.allocate_frame()?.start_address(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        // map currently used kernel addresses
        // assumes page table entry 0 is a page table (just allocated so why not?)
        let page_table_0 = unsafe {
            &mut *((page_table[0].addr().as_u64() + mapper.phys_offset().as_u64()) as *mut PageTable)
        };
        // assumes the kernel level 4 table is a level 4 table
        let current_table_0 = unsafe {
            &*((current_page_table[0].addr().as_u64() + mapper.phys_offset().as_u64()) as *const PageTable)
        };

        for i in 1..512 {
            page_table[i] = current_page_table[i].clone();
        }

        page_table_0.zero();
        for i in 1..512 {
            page_table_0[i] = current_table_0[i].clone();
        }

        // map vga text buffer
        page_table_0[0].set_addr(
            frame_allocator.allocate_frame()?.start_address(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
        );
        let page_table_0_0 = unsafe {
            &mut *((page_table_0[0].addr().as_u64() + mapper.phys_offset().as_u64()) as *mut PageTable)
        };
        let current_table_0_0 = unsafe {
            &*((current_table_0[0].addr().as_u64() + mapper.phys_offset().as_u64()) as *const PageTable)
        };
        page_table_0_0[0] = current_table_0_0[0].clone();


        Some(page_table)
    }

    unsafe fn map_stack(
        mapper: &mut OffsetPageTable,
        frame_allocator: &mut BootInfoFrameAllocator
    ) {
        let start_page = Page::containing_address(VirtAddr::new(USERSPACE_STACK_BASE));
        let end_page = Page::containing_address(VirtAddr::new(USERSPACE_STACK));
        let stack_range = Page::range_inclusive(start_page, end_page);
        for page in stack_range {
            mapper
                .map_to(
                    page,
                    frame_allocator.allocate_frame().unwrap(), // TODO this leaks
                    PageTableFlags::PRESENT
                        | PageTableFlags::WRITABLE
                        | PageTableFlags::USER_ACCESSIBLE,
                    frame_allocator
                )
                .unwrap()
                .flush();
        }
    }

    /// Creates process virtual memory and maps to it
    unsafe fn map_process(
        old_mapper: &OffsetPageTable,
        frame_allocator: &mut BootInfoFrameAllocator,
    ) -> Option<(Box<PageTable>, PhysAddr)> {
        let page_table = memory::active_level_4_table(
            old_mapper.phys_offset().as_u64()
        );
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
        let mut new_mapper = OffsetPageTable::new(&mut new_page_table, old_mapper.phys_offset());

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


        Some((new_page_table, new_page_table_addr))
    }

    pub fn get_state(&self) -> TaskState {
        self.task_state
    }

    pub fn update_state(&mut self, state: TaskState) {
        self.task_state = state;
    }

    pub unsafe extern "C" fn activate(&mut self) -> bool {
        use x86_64::instructions::tlb;

        let (_, cr3_flags) = Cr3::read();
        Cr3::write(
            PhysFrame::containing_address(self.page_table_addr),
            cr3_flags
        );
        tlb::flush_all();
        x86_64::instructions::interrupts::enable();

        if let Some(state) = self.process_state.as_ref() {
            asm!(
            "mov rbx, rcx",
            in("rcx") state.rbx
            );
            asm!(
            "mov rbp, rcx",
            in("rcx") state.rbp
            );
            asm!(
            "mov r12, rcx",
            in("rcx") state.r12
            );
            asm!(
            "mov r13, rcx",
            in("rcx") state.r13
            );
            asm!(
            "mov r14, rcx",
            in("rcx") state.r14
            );
            asm!(
            "mov r15, rcx",
            in("rcx") state.r15
            );
            asm!(
            "mov rsp, rcx",
            in("rcx") state.rsp
            );
            asm!("\
                mov rax, 0x1
                add rsp, 0xc8 // DO NOT MODIFY; CORRESPONDS TO STACK OFFSET OF deactivate FUNCTION
                ret
            ")
        } else {
            self.switch_to_usermode();
        }
        true
    }

    pub unsafe extern "C" fn deactivate(&mut self) -> bool {
        let mut state = ProcessState {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: 0,
        };
        asm!(
        "mov rcx, rbx",
        out("rcx") state.rbx
        );
        asm!(
        "mov rcx, rbp",
        out("rcx") state.rbp
        );
        asm!(
        "mov rcx, r12",
        out("rcx") state.r12
        );
        asm!(
        "mov rcx, r13",
        out("rcx") state.r13
        );
        asm!(
        "mov rcx, r14",
        out("rcx") state.r14
        );
        asm!(
        "mov rcx, r15",
        out("rcx") state.r15
        );
        asm!(
        "mov rcx, rsp",
        out("rcx") state.rsp
        );
        self.process_state = Some(state);
        false
    }

    unsafe fn switch_to_usermode(&self) {
        serial_println!("{:x?}", *(0x201120 as *const [u8; 8]));
        let entry_point = self.entry_offset;
        let eflags = x86_64::registers::rflags::read().bits();

        let (cs_index, ds_index) = gdt::set_usermode_segments();
        //tlb::flush_all();

        asm!("\
        push rax
        push rsi
        push 0x200 // 0x200
        push rdx
        push rdi
        iretq",
        in("rdi") entry_point,
        in("rsi") USERSPACE_STACK,
        in("dx") cs_index,
        in("ax") ds_index,
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TaskState {
    READY,
    RUNNING,
    WAITING,
    DONE,
}

struct ProcessState {
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rsp: u64,
}