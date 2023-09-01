use alloc::ffi::CString;
use alloc::string::String;
use core::ptr::slice_from_raw_parts;
use crate::print;

/// prints text pointed to by arg0 on vga text buffer
///
/// * 0 indicates success
/// * -1 indicates utf8 error
pub unsafe fn print_vga_text(text_addr: u64, length: u64) -> i64 {
    let bytes = &*slice_from_raw_parts(text_addr as *const u8, length as usize);
    let string = match core::str::from_utf8(bytes) {
        Ok(str) => str,
        Err(_) => return -1
    };
    print!("{}", string);

    0
}