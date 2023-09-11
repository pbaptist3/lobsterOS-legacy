use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};
use bootloader::bootinfo::{MemoryMap, MemoryRegionType};
use crate::println;

/// Initialize physical memory offset page table
pub unsafe fn init() -> OffsetPageTable<'static> {
    let phys_offset = crate::BOOT_INFO.get()
        .expect("boot info not initialized")
        .physical_memory_offset;
    let level_4_table = active_level_4_table(phys_offset);
    OffsetPageTable::new(level_4_table, VirtAddr::new(phys_offset))
}

pub unsafe fn active_level_4_table(physical_memory_offset: u64) -> &'static mut PageTable
{
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();

    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt as *mut PageTable;

    &mut *page_table_ptr
}

/// Allocates frames from bootlaoder's memory map
pub struct BootInfoFrameAllocator {
    memory_map: &'static MemoryMap,
    frame_iter: Box<dyn Iterator<Item = PhysFrame>>
}

impl BootInfoFrameAllocator {
    /// Create a frame allocator from the memory map provides
    ///
    /// Memory map must be valid and all USABLE frames must be truly unused
    pub unsafe fn init(memory_map: &'static MemoryMap, used: usize) -> Self {
        let frame_iter = Self::usable_frames(memory_map);
        let frame_iter = frame_iter.skip(used);
        Self {
            memory_map,
            frame_iter: Box::new(frame_iter)
        }
    }

    /// Returns an iterator over all usable frames
    fn usable_frames(memory_map: &'static MemoryMap) -> impl Iterator<Item = PhysFrame> {
        let regions = memory_map.iter();
        let usable_regions = regions.filter(|r| r.region_type == MemoryRegionType::Usable);
        let addr_ranges = usable_regions.map(|r| r.range.start_addr()..r.range.end_addr());
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        let frame_iter = frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)));
        frame_iter
    }
}

unsafe impl FrameAllocator::<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.frame_iter.next()
    }
}

impl FrameDeallocator::<Size4KiB> for BootInfoFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        todo!()
    }
}

/// Allocates frames from bootloader's memory map
pub struct LinearFrameAllocator {
    memory_map: &'static MemoryMap,
    next: usize,
}

impl LinearFrameAllocator {
    /// Create a frame allocator from the memory map provides
    ///
    /// Memory map must be valid and all USABLE frames must be truly unused
    pub unsafe fn init(memory_map: &'static MemoryMap) -> Self {
        Self {
            memory_map,
            next: 0,
        }
    }

    /// Returns an iterator over all usable frames
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.region_type == MemoryRegionType::Usable);
        let addr_ranges = usable_regions.map(|r| r.range.start_addr()..r.range.end_addr());
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        let frame_iter = frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)));
        frame_iter
    }

    pub fn get_used(&self) -> usize {
        self.next
    }
}

unsafe impl FrameAllocator::<Size4KiB> for LinearFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.next += 1;
        self.usable_frames().nth(self.next)
    }
}