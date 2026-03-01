use std::os::raw::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGEventCreateKeyboardEvent(
        source: *mut c_void,
        virtualKey: u16,
        keyDown: bool,
    ) -> *mut c_void;
    pub fn CGEventPost(tap: u32, event: *mut c_void);
    pub fn CFRelease(cf: *mut c_void);
}

const kCGSessionEventTap: u32 = 1;

fn main() {
    println!("Testing CGEventPost for 0x68 (Kana)");
    unsafe {
        let event = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0x68, true);
        if !event.is_null() {
            CGEventPost(kCGSessionEventTap, event);
            CFRelease(event);
            println!("Post 0x68 (Down) Success!");
        }
        
        let event_up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0x68, false);
        if !event_up.is_null() {
            CGEventPost(kCGSessionEventTap, event_up);
            CFRelease(event_up);
            println!("Post 0x68 (Up) Success!");
        }
    }
}
