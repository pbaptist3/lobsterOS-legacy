use core::arch::asm;
use crate::threading::scheduler::SCHEDULER;

/// exits process
pub unsafe fn exit(exit_code: u64, stack_addr: u64) -> ! {
    super::delete_syscall_stack(stack_addr as *mut u8);
    SCHEDULER.lock().end_current_task();

    unreachable!()
}