// EasyNet CLI — FFI string marshalling
// ======================================
//
// File: src/ffi/strings.rs
// Description: Safe conversion helpers between C strings crossing
//              the ABI and Rust `&str` / `String` used internally.
//              These helpers centralise the two places where Rust
//              and C's string conventions meet: reading input
//              pointers (must be valid UTF-8, non-null) and handing
//              out output strings (must outlive until the caller
//              frees them).
//
// Allocation contract for *output* strings
// ----------------------------------------
// When an exported function returns a `*mut c_char`, the caller
// owns the buffer and MUST call `easynet_string_free()` to release
// it. The buffer is a standalone heap allocation produced by
// `CString::into_raw`; the caller must not `free()` it directly
// (that would skip Rust's `Vec` allocator metadata cleanup).
//
// `CString::into_raw` and `CString::from_raw` are inverse operations;
// `easynet_string_free` performs the latter and drops the resulting
// `CString`, which frees the heap storage.
//
// Why one dedicated free function, not two
// ----------------------------------------
// Every output string — whether it came from an invocation result, a
// `last_error` copy, or a future diagnostic blob — is
// produced the same way. Having one `easynet_string_free` means
// Client bindings can register one cleanup function and use it for
// every output pointer.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// Read a borrowed UTF-8 string from a raw pointer crossing the
/// ABI boundary. Returns `Err` on null / non-UTF-8. Used by every
/// exported function that accepts a string argument.
pub(crate) fn read_cstr<'a>(ptr: *const c_char) -> Result<&'a str, StringError> {
    if ptr.is_null() {
        return Err(StringError::Null);
    }
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().map_err(|_| StringError::NotUtf8)
}

/// Produce an owned heap-allocated C string the caller will free
/// via `easynet_string_free`. Returns NULL on allocation failure
/// (extremely rare; an OOM at that point is already unrecoverable,
/// but we do not want to panic across the ABI).
pub(crate) fn alloc_output_cstring(s: impl Into<String>) -> *mut c_char {
    let s = s.into();
    let sanitized: String = s.chars().filter(|c| *c != '\0').collect();
    match CString::new(sanitized) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a buffer previously returned from an exported function.
///
/// # Safety
/// `ptr` must be a pointer previously returned from an exported
/// function documented as "caller frees via easynet_string_free",
/// or NULL. Double-free is undefined behaviour; the caller is
/// responsible for not reusing the pointer after freeing.
#[no_mangle]
pub unsafe extern "C" fn easynet_string_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // CString::from_raw transfers ownership back to Rust; when the
    // CString is dropped at the end of this expression, the heap
    // storage is released via the same allocator that produced it.
    drop(unsafe { CString::from_raw(ptr) });
}

/// Reasons `read_cstr` fails; the caller maps these to the
/// corresponding `ERR_*` code in `errors.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringError {
    Null,
    NotUtf8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_cstr_rejects_null() {
        let err = read_cstr(std::ptr::null()).unwrap_err();
        assert_eq!(err, StringError::Null);
    }

    #[test]
    fn read_cstr_accepts_well_formed_utf8() {
        // The happy path: a valid C-string round-trips to a Rust
        // &str with the same bytes.
        let c = CString::new("hello").unwrap();
        let s = read_cstr(c.as_ptr()).unwrap();
        assert_eq!(s, "hello");
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn alloc_output_cstring_allocates_standalone_buffer() {
        // The returned pointer must be freeable via
        // `easynet_string_free` without borrowing anything from the
        // input source. Round-trip tests that property by
        // constructing from a String, reading the pointer, freeing,
        // and confirming the function is sound.
        let p = alloc_output_cstring("hello");
        assert!(!p.is_null());
        let back = unsafe { CStr::from_ptr(p) }.to_str().unwrap();
        assert_eq!(back, "hello");
        unsafe { easynet_string_free(p) };
    }

    #[test]
    fn easynet_string_free_accepts_null_without_crashing() {
        // The C convention for `free(NULL)` is "no-op". The ABI
        // cleanup function honours that convention so clients do
        // not have to branch before every call.
        unsafe { easynet_string_free(std::ptr::null_mut()) };
    }
}
