
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use crate::{gdt, hlt_loop, println, serial_println};
use crate::threading::scheduler::SCHEDULER;
use lazy_static::lazy_static;
use spin::Mutex;
use pic8259::ChainedPics;
use x86_64::instructions::port::Port;

pub const PIC_1_OFFSET: u8 = 32;
pub const TIMER_FREQUENCY: u32 = 1073;
const TIMER_FREQUENCY_BASE: u32 = 1193182;

pub static PICS: Mutex<ChainedPics> = Mutex::new(unsafe {
    ChainedPics::new_contiguous(PIC_1_OFFSET)
});

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            // backup stack for double fault
            idt.double_fault.set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt[InterruptIndex::Timer.as_usize()]
            .set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_usize()]
            .set_handler_fn(keyboard_interrupt_handler);
        idt.general_protection_fault.set_handler_fn(protection_fault_handler);
        idt.stack_segment_fault.set_handler_fn(stack_segment_fault_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.invalid_tss.set_handler_fn(invalid_tss_handler);
        idt.alignment_check.set_handler_fn(alignment_handler);
        idt.segment_not_present.set_handler_fn(segment_not_present_handler);
        idt
    };
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 { self as u8 }
    fn as_usize(self) -> usize { usize::from(self.as_u8()) }
}

pub fn init_idt() {
    // set timer frequency
    let mut timer_port = Port::new(0x40);
    unsafe {
        timer_port.write(TIMER_FREQUENCY_BASE / TIMER_FREQUENCY);
    }

    IDT.load();
}

extern "x86-interrupt" fn segment_not_present_handler(
    stack_frame: InterruptStackFrame, error_code: u64
) {
    println!("SEGMENT NOT PRESENT {}\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn alignment_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    println!("ALIGNMENT CHECK {}\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn invalid_tss_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    println!("INVALID TSS {}\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    println!("INVALID OPCODE\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("BREAKPOINT EXCEPTION\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn stack_segment_fault_handler(
    stack_frame: InterruptStackFrame, error_code: u64
) {
    println!("STACK SEGMENT FAULT: code {}\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn protection_fault_handler(
    stack_frame: InterruptStackFrame, error_code: u64
) {
    println!("PROTECTION FAULT: code {}\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame, error_code: u64) -> !
{
    panic!("DOUBLE FAULT EXCEPTION\nERROR CODE: {}\n{:#?}", error_code, stack_frame);
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // signal end of interrupt
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    SCHEDULER.lock().tick();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    use crate::task::keyboard;

    // keyboard ps/2 port
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    keyboard::add_scancode(scancode);

    // signal end of interrupt
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode)
{
    use x86_64::registers::control::Cr2;

    serial_println!(
        "PAGE FAULT EXCEPTION\nAddress: {:?}\nError Code: {:?}\n{:#?}",
        Cr2::read(),
        error_code,
        stack_frame
    );

    hlt_loop();
}
