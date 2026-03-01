use std::os::raw::c_void;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
    fn CFRelease(cf: *mut c_void);
}

#[link(name = "System", kind = "dylib")]
extern "C" {
    pub static _dispatch_main_q: c_void;
    pub fn dispatch_sync_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

extern "C" fn work(_ctx: *mut c_void) {
    println!("Work on main thread (sync)");
    unsafe {
        let source = TISCopyCurrentKeyboardInputSource();
        println!("Source: {:?}", source);
        if !source.is_null() {
            CFRelease(source);
        }
    }
}

fn main() {
    println!("Dispatching sync...");
    std::thread::spawn(|| {
        unsafe {
            dispatch_sync_f(
                &_dispatch_main_q as *const _ as *const c_void,
                std::ptr::null_mut(),
                work,
            );
        }
        println!("Sync work done");
        std::process::exit(0);
    });

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRunLoopRun();
    }
    unsafe {
        CFRunLoopRun();
    }
}
