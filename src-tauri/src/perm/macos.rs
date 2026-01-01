use std::ffi::c_void;

use super::{PermissionCheckResult, RequestAccessibilityResult};

type Boolean = u8;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> Boolean;
}

#[repr(C)]
struct __CFDictionary(c_void);
type CFDictionaryRef = *const __CFDictionary;

#[repr(C)]
struct __CFString(c_void);
type CFStringRef = *const __CFString;

#[repr(C)]
struct __CFAllocator(c_void);
type CFAllocatorRef = *const __CFAllocator;

#[repr(C)]
struct __CFBoolean(c_void);
type CFBooleanRef = *const __CFBoolean;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: CFBooleanRef;
    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: isize,
        key_call_backs: *const c_void,
        value_call_backs: *const c_void,
    ) -> CFDictionaryRef;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
}

pub fn ax_is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn ax_request_trust_prompt() -> bool {
    unsafe {
        let key: *const c_void = kAXTrustedCheckOptionPrompt as *const c_void;
        let val: *const c_void = kCFBooleanTrue as *const c_void;
        let keys = [key];
        let vals = [val];
        // We can pass null callbacks; CF will use pointer identity.
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            vals.as_ptr(),
            1,
            std::ptr::null(),
            std::ptr::null(),
        );
        if dict.is_null() {
            return false;
        }
        let res = AXIsProcessTrustedWithOptions(dict) != 0;
        CFRelease(dict as *const c_void);
        res
    }
}

pub fn check_permissions() -> Result<PermissionCheckResult, String> {
    let trusted = ax_is_trusted();
    eprintln!("[kiklet][perm] ax_trusted={}", trusted);
    Ok(PermissionCheckResult {
        ok: trusted,
        need_accessibility: !trusted,
    })
}

pub fn request_accessibility() -> Result<RequestAccessibilityResult, String> {
    let _ = ax_request_trust_prompt();
    // The prompt is async; return whether we are trusted *now*.
    let trusted = ax_is_trusted();
    eprintln!("[kiklet][perm] ax_request_prompt trusted_now={}", trusted);
    Ok(RequestAccessibilityResult { requested: true })
}


