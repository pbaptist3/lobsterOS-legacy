use alloc::boxed::Box;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::arch::asm;
use x86_64::registers::model_specific::{LStar, Msr};
use x86_64::registers::rflags::RFlags;
use x86_64::structures::gdt::GlobalDescriptorTable;
use x86_64::VirtAddr;
use crate::println;

const MSR_SCE: u32 = 0xC0000080;
const IA32_STAR: u32 = 0xC0000081;
const IA32_LSTAR: u32 = 0xC0000082;
const IA32_FMASK: u32 = 0xC0000084;

const STACK_SIZE: usize = 0x1000;

pub unsafe fn init() {
    // enable system call extensions
    let mut sce = Msr::new(MSR_SCE);
    let mut new_sce = sce.read() | 0b1;
    sce.write(new_sce);

    // magic value for clearing interrupt flag
    let mut sfmask = Msr::new(IA32_FMASK);
    sfmask.write(0x200);

    let handler_addr = syscall_wrapper as *const () as u64;
    let mut lstar = Msr::new(IA32_LSTAR);
    lstar.write(handler_addr);

    // magic value for enabling syscalls
    let mut ia32_star_reg = Msr::new(IA32_STAR);
    ia32_star_reg.write(0x23000800000000);
}

#[naked]
extern "C" fn syscall_wrapper() {
    unsafe { asm!("\
        push rcx // preserve callee-saved registers
        push r11
        push rbp
        push rbx
        push r12
        push r13
        push r14
        push r15
        mov rbp, rsp // start new stack frame
        push rax // preserve caller-saved registers (syscall args)
        push rdi
        push rsi
        push rdx
        push r10

        call {setup_stack} // setup stack pointer
        mov r9, rax

        sti
        pop r8 // restore caller-saved registers (syscall args)
        pop rcx
        pop rdx
        pop rsi
        pop rdi
        mov rsp, r9 // use new stack
        push r9 // store stack
        call {syscall_handler}

        cli
        pop rdi // restore stack pointer base
        call {free_stack}

        mov rsp, rbp // restore stack
        pop r15
        pop r14
        pop r13
        pop r12
        pop rbx
        pop rbp // restore stack
        pop r11
        pop rcx
        sysretq // return to ring 3
    ",
    setup_stack = sym setup_syscall_stack,
    syscall_handler = sym syscall_handler,
    free_stack = sym delete_syscall_stack,
    options(noreturn)
    ); }
}

unsafe extern "C" fn setup_syscall_stack() -> *mut u8 {
    //let syscall_stack = Box::<[u8; STACK_SIZE]>::new_uninit();
    let stack_layout = Layout::from_size_align_unchecked(STACK_SIZE, 0x8);
    let syscall_stack = alloc::alloc::alloc(stack_layout);
    let stack_pointer = syscall_stack.offset(stack_layout.size() as isize);
    stack_pointer
}

unsafe extern "C" fn delete_syscall_stack(stack_end_ptr: *mut u8) {
    let stack_layout = Layout::from_size_align_unchecked(STACK_SIZE, 0x8);
    let stack_ptr = stack_end_ptr.offset(-1 * stack_layout.size() as isize);
    alloc::alloc::dealloc(stack_ptr, stack_layout);
}

extern "C" fn syscall_handler(
    syscall: u64, arg0: u64, arg1: u64, arg2: u64, arg3: u64
) -> i64 {
    // body of syscall handler
    println!("{} {} {} {} {}", syscall, arg0, arg1, arg2, arg3);

    0
}

