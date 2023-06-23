mod fixed_block;

use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;
use fixed_block::FixedBlockAlloc;

pub const HEAP_START: usize = 0x_4444_4444_0000;
pub const HEAP_SIZE: usize = 0x100 * 0x1000;

#[global_allocator]
static ALLOCATOR: Locked<FixedBlockAlloc> = Locked::new(FixedBlockAlloc::new());

/// Initializes kernel heap
pub fn init_heap(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    // get pages needed for heap
    let page_range = {
        let heap_start_addr = VirtAddr::new(HEAP_START as u64);
        let heap_end_addr = heap_start_addr + HEAP_SIZE - 1u64;
        let heap_start_range = Page::containing_address(heap_start_addr);
        let heap_end_range = Page::containing_address(heap_end_addr);
        Page::range_inclusive(heap_start_range, heap_end_range)
    };

    for page in page_range {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;
        // set page to be present and writable
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        unsafe {
            mapper.map_to(page, frame, flags, frame_allocator)?
                .flush()
        };
    }

    unsafe {
        ALLOCATOR.lock().init(HEAP_START, HEAP_SIZE)
    }

    Ok(())
}

pub struct Locked<A> {
    inner: spin::Mutex<A>,
}

impl<A> Locked<A> {
    pub const fn new(inner: A) -> Self {
        Self {
            inner: spin::Mutex::new(inner)
        }
    }

    pub fn lock(&self) -> spin::MutexGuard<A> {
        self.inner.lock()
    }
}