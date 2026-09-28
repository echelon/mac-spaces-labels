//! Minimal, allocation-light readers for the CoreFoundation plists SkyLight
//! returns. Every accessor type-checks with `CFGetTypeID`, so a changed or
//! missing field reads as `None` instead of crashing.

use core_foundation_sys::array::{
  kCFTypeArrayCallBacks, CFArrayCreate, CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex,
  CFArrayRef,
};
use core_foundation_sys::base::{CFGetTypeID, CFIndex, CFRelease, CFTypeRef};
use core_foundation_sys::dictionary::{
  CFDictionaryGetTypeID, CFDictionaryGetValue, CFDictionaryRef,
};
use core_foundation_sys::number::{
  kCFNumberSInt64Type, CFNumberCreate, CFNumberGetTypeID, CFNumberGetValue, CFNumberRef,
};
use core_foundation_sys::string::{
  kCFStringEncodingUTF8, CFStringCreateWithBytes, CFStringGetCString, CFStringGetCStringPtr,
  CFStringGetLength, CFStringGetMaximumSizeForEncoding, CFStringGetTypeID, CFStringRef,
};
use std::ffi::{c_void, CStr};

/// An owned CF object released on drop (the "Create/Copy rule").
pub struct Owned(CFTypeRef);

impl Owned {
  /// Takes ownership of a +1 reference. Returns `None` for NULL.
  pub unsafe fn new(ptr: CFTypeRef) -> Option<Self> {
    (!ptr.is_null()).then_some(Self(ptr))
  }

  pub fn as_ptr(&self) -> CFTypeRef {
    self.0
  }
}

impl Drop for Owned {
  fn drop(&mut self) {
    unsafe { CFRelease(self.0) }
  }
}

pub unsafe fn array_items(value: CFTypeRef) -> impl Iterator<Item = CFTypeRef> {
  let array = is_type(value, CFArrayGetTypeID()).then_some(value as CFArrayRef);
  let count = array.map_or(0, |a| CFArrayGetCount(a));
  (0..count).map(move |i| CFArrayGetValueAtIndex(array.unwrap(), i) as CFTypeRef)
}

/// Looks up a string key. `key` must be a static C string literal.
pub unsafe fn dict_get(dict: CFTypeRef, key: &Key) -> Option<CFTypeRef> {
  if !is_type(dict, CFDictionaryGetTypeID()) {
    return None;
  }
  let value = CFDictionaryGetValue(dict as CFDictionaryRef, key.0.as_ptr());
  (!value.is_null()).then_some(value as CFTypeRef)
}

pub unsafe fn number_i64(value: CFTypeRef) -> Option<i64> {
  if !is_type(value, CFNumberGetTypeID()) {
    return None;
  }
  let mut out: i64 = 0;
  CFNumberGetValue(
    value as CFNumberRef,
    kCFNumberSInt64Type,
    &mut out as *mut i64 as *mut c_void,
  )
  .then_some(out)
}

pub unsafe fn string(value: CFTypeRef) -> Option<String> {
  if !is_type(value, CFStringGetTypeID()) {
    return None;
  }
  let s = value as CFStringRef;
  let fast = CFStringGetCStringPtr(s, kCFStringEncodingUTF8);
  if !fast.is_null() {
    return Some(CStr::from_ptr(fast).to_string_lossy().into_owned());
  }
  let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(s), kCFStringEncodingUTF8) + 1;
  let mut buf = vec![0u8; size as usize];
  (CFStringGetCString(s, buf.as_mut_ptr() as *mut i8, size, kCFStringEncodingUTF8) != 0).then(
    || {
      CStr::from_bytes_until_nul(&buf)
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_default()
    },
  )
}

pub fn new_string(text: &str) -> Owned {
  unsafe {
    Owned::new(CFStringCreateWithBytes(
      std::ptr::null(),
      text.as_ptr(),
      text.len() as CFIndex,
      kCFStringEncodingUTF8,
      0,
    ) as CFTypeRef)
    .expect("CFStringCreateWithBytes")
  }
}

/// A CFArray of CFNumbers, as the window-id arguments to SkyLight expect.
pub fn new_number_array(values: &[i64]) -> Owned {
  unsafe {
    let numbers: Vec<Owned> = values
      .iter()
      .map(|v| {
        let ptr = CFNumberCreate(
          std::ptr::null(),
          kCFNumberSInt64Type,
          v as *const i64 as *const c_void,
        );
        Owned::new(ptr as CFTypeRef).expect("CFNumberCreate")
      })
      .collect();
    let raw: Vec<*const c_void> = numbers.iter().map(|n| n.as_ptr()).collect();
    Owned::new(CFArrayCreate(
      std::ptr::null(),
      raw.as_ptr(),
      raw.len() as CFIndex,
      &kCFTypeArrayCallBacks,
    ) as CFTypeRef)
    .expect("CFArrayCreate")
  }
}

unsafe fn is_type(value: CFTypeRef, type_id: usize) -> bool {
  !value.is_null() && CFGetTypeID(value) == type_id
}

/// A dictionary key, created once and reused (keys are compared by value).
pub struct Key(Owned);

impl Key {
  pub fn new(text: &str) -> Self {
    Self(new_string(text))
  }
}

// CFStrings are immutable and CF reference counting is thread-safe.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}
