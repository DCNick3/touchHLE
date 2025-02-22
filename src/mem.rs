/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Types related to the virtual memory of the emulated application, or the
//! "guest memory".
//!
//! The virtual address space is 32-bit, as is the pointer size.
//!
//! No attempt is made to do endianness conversion for reads and writes to
//! memory, because all supported emulated and host platforms are little-endian.
//!
//! Relevant Apple documentation:
//! * [Memory Usage Performance Guidelines](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/ManagingMemory/ManagingMemory.html)

mod allocator;

/// Equivalent of `usize` for guest memory.
pub type GuestUSize = u32;

/// Equivalent of `isize` for guest memory.
pub type GuestISize = i32;

/// [std::mem::size_of], but returning a [GuestUSize].
pub const fn guest_size_of<T: Sized>() -> GuestUSize {
    assert!(std::mem::size_of::<T>() <= u32::MAX as usize);
    std::mem::size_of::<T>() as u32
}

/// Internal type for representing an untyped virtual address.
type VAddr = GuestUSize;

/// Pointer type for guest memory, or the "guest pointer" type.
///
/// The `MUT` type parameter determines whether this is mutable or not.
/// Don't write it out explicitly, use [ConstPtr], [MutPtr], [ConstVoidPtr] or
/// [MutVoidPtr] instead instead.
///
/// The implemented methods try to mirror the Rust [pointer] type's methods,
/// where possible.
#[repr(transparent)]
pub struct Ptr<T, const MUT: bool>(VAddr, std::marker::PhantomData<T>);

// #[derive(...)] doesn't work for this type because it expects T to have the
// trait we want implemented
impl<T, const MUT: bool> Clone for Ptr<T, MUT> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, const MUT: bool> Copy for Ptr<T, MUT> {}
impl<T, const MUT: bool> PartialEq for Ptr<T, MUT> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T, const MUT: bool> Eq for Ptr<T, MUT> {}
impl<T, const MUT: bool> std::hash::Hash for Ptr<T, MUT> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// Constant guest pointer type (like Rust's `*const T`).
pub type ConstPtr<T> = Ptr<T, false>;
/// Mutable guest pointer type (like Rust's `*mut T`).
pub type MutPtr<T> = Ptr<T, true>;
#[allow(dead_code)]
/// Constant guest pointer-to-void type (like C's `const void *`)
pub type ConstVoidPtr = ConstPtr<std::ffi::c_void>;
/// Mutable guest pointer-to-void type (like C's `void *`)
pub type MutVoidPtr = MutPtr<std::ffi::c_void>;

impl<T, const MUT: bool> Ptr<T, MUT> {
    pub const fn null() -> Self {
        Ptr(0, std::marker::PhantomData)
    }

    pub fn to_bits(self) -> VAddr {
        self.0
    }
    pub fn from_bits(bits: VAddr) -> Self {
        Ptr(bits, std::marker::PhantomData)
    }

    pub fn cast<U>(self) -> Ptr<U, MUT> {
        Ptr::<U, MUT>::from_bits(self.to_bits())
    }

    pub fn is_null(self) -> bool {
        self.to_bits() == 0
    }
}

impl<T> ConstPtr<T> {
    #[allow(dead_code)]
    pub fn cast_mut(self) -> MutPtr<T> {
        Ptr::from_bits(self.to_bits())
    }
}
impl<T> MutPtr<T> {
    pub fn cast_const(self) -> ConstPtr<T> {
        Ptr::from_bits(self.to_bits())
    }
}

impl<T, const MUT: bool> std::fmt::Debug for Ptr<T, MUT> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#x}", self.to_bits())
    }
}

// C-like pointer arithmetic
impl<T, const MUT: bool> std::ops::Add<GuestUSize> for Ptr<T, MUT> {
    type Output = Self;

    fn add(self, other: GuestUSize) -> Self {
        let size: GuestUSize = guest_size_of::<T>();
        Self::from_bits(
            self.to_bits()
                .checked_add(other.checked_mul(size).unwrap())
                .unwrap(),
        )
    }
}
impl<T, const MUT: bool> std::ops::AddAssign<GuestUSize> for Ptr<T, MUT> {
    fn add_assign(&mut self, rhs: GuestUSize) {
        *self = *self + rhs;
    }
}
impl<T, const MUT: bool> std::ops::Sub<GuestUSize> for Ptr<T, MUT> {
    type Output = Self;

    fn sub(self, other: GuestUSize) -> Self {
        let size: GuestUSize = guest_size_of::<T>();
        Self::from_bits(
            self.to_bits()
                .checked_sub(other.checked_mul(size).unwrap())
                .unwrap(),
        )
    }
}
impl<T, const MUT: bool> std::ops::SubAssign<GuestUSize> for Ptr<T, MUT> {
    fn sub_assign(&mut self, rhs: GuestUSize) {
        *self = *self - rhs;
    }
}

/// Marker trait for types that can be safely read from guest memory.
///
/// See also [SafeWrite] and [crate::abi].
///
/// # Safety
/// Reading from guest memory is essentially doing a [std::mem::transmute],
/// which is notoriously unsafe in Rust. Only types for which all possible bit
/// patterns are legal (e.g. integers) should have this trait.
pub unsafe trait SafeRead: Sized {}
unsafe impl SafeRead for i8 {}
unsafe impl SafeRead for u8 {}
unsafe impl SafeRead for i16 {}
unsafe impl SafeRead for u16 {}
unsafe impl SafeRead for i32 {}
unsafe impl SafeRead for u32 {}
unsafe impl SafeRead for i64 {}
unsafe impl SafeRead for u64 {}
unsafe impl SafeRead for f32 {}
unsafe impl SafeRead for f64 {}
unsafe impl<T, const MUT: bool> SafeRead for Ptr<T, MUT> {}

/// Marker trait for types that can be written to guest memory.
///
/// Unlike for [SafeRead], there is no (Rust) safety consideration here; it's
/// just a way to catch accidental use of types unintended for guest use.
/// This was added after discovering that `()` is "[Sized]" and therefore a
/// single stray semicolon can wreak havoc...
///
/// Especially for structs, be careful that the type matches the expected ABI.
/// At minimum you should have `#[repr(C, packed)]` and appropriate padding
/// members.
///
/// See also [SafeRead] and [crate::abi].
pub trait SafeWrite: Sized {}
impl<T: SafeRead> SafeWrite for T {}

type Bytes = [u8; 1 << 32];

/// The type that owns the guest memory and provides accessors for it.
pub struct Mem {
    /// This array is 4GiB in size so that it can cover the entire 32-bit
    /// virtual address space, but it should not use that much physical memory,
    /// assuming that the host OS backs it with lazily-allocated pages and we
    /// are careful to avoid accessing most of it.
    ///
    /// iPhone OS devices only had 128MiB or 256MiB of RAM total, with no swap
    /// space, so less than 6.25% of this array should be used, assuming no
    /// fragmentation.
    ///
    /// This is a raw pointer because inevitably we will have to hand out
    /// pointers to memory sometimes, and being able to hold a `&mut` on this
    /// array simultaneously seems like an undefined behavior trap. This also
    /// means that the underlying memory should never be moved, and therefore
    /// the array can't be growable.
    ///
    /// One advantage of `[u8; 1 << 32]` over `[u8]` is that it might help rustc
    /// optimize away bounds checks for `memory.bytes[ptr_32bit as usize]`.
    bytes: *mut Bytes,

    allocator: allocator::Allocator,
}

impl Drop for Mem {
    fn drop(&mut self) {
        let layout = std::alloc::Layout::new::<Bytes>();
        unsafe {
            std::alloc::dealloc(self.bytes as *mut _, layout);
        }
    }
}

impl Mem {
    /// The first 4KiB of address space on iPhone OS is unused, so null pointer
    /// accesses can be trapped.
    ///
    /// We don't have full memory protection, but we can check accesses in that
    /// range.
    pub const NULL_PAGE_SIZE: VAddr = 0x1000;

    /// [According to Apple](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/CreatingThreads/CreatingThreads.html)
    /// among others, the iPhone OS main thread stack size is 1MiB.
    pub const MAIN_THREAD_STACK_SIZE: GuestUSize = 1024 * 1024;

    /// Address of the lowest byte (not the base) of the main thread's stack.
    ///
    /// We are arbitrarily putting the stack at the top of the virtual address
    /// space (see also: stack.rs), I have no idea if this matches iPhone OS.
    pub const MAIN_THREAD_STACK_LOW_END: VAddr = 0u32.wrapping_sub(Self::MAIN_THREAD_STACK_SIZE);

    /// iPhone OS secondary thread stack size.
    pub const SECONDARY_THREAD_STACK_SIZE: GuestUSize = 512 * 1024;

    pub fn new() -> Mem {
        // This will hopefully get the host OS to lazily allocate the memory.
        let layout = std::alloc::Layout::new::<Bytes>();
        let bytes = unsafe { std::alloc::alloc_zeroed(layout) as *mut Bytes };

        let allocator = allocator::Allocator::new();

        Mem { bytes, allocator }
    }

    fn bytes(&self) -> &Bytes {
        unsafe { &*self.bytes }
    }
    fn bytes_mut(&mut self) -> &mut Bytes {
        unsafe { &mut *self.bytes }
    }

    // the performance characteristics of this hasn't been profiled, but it
    // seems like a good idea to help the compiler optimise for the fast path
    #[cold]
    fn null_check_fail(at: VAddr, size: GuestUSize) {
        panic!(
            "Attempted null-page access at {:#x} ({:#x} bytes)",
            at, size
        )
    }

    /// Get a slice for reading `count` bytes. This is the basic primitive for
    /// safe read-only memory access.
    pub fn bytes_at<const MUT: bool>(&self, ptr: Ptr<u8, MUT>, count: GuestUSize) -> &[u8] {
        if ptr.to_bits() < Self::NULL_PAGE_SIZE {
            Self::null_check_fail(ptr.to_bits(), count)
        }
        &self.bytes()[ptr.to_bits() as usize..][..count as usize]
    }
    /// Get a slice for reading or writing `count` bytes. This is the basic
    /// primitive for safe read-write memory access.
    pub fn bytes_at_mut(&mut self, ptr: MutPtr<u8>, count: GuestUSize) -> &mut [u8] {
        if ptr.to_bits() < Self::NULL_PAGE_SIZE {
            Self::null_check_fail(ptr.to_bits(), count)
        }
        &mut self.bytes_mut()[ptr.to_bits() as usize..][..count as usize]
    }

    /// Get a pointer for reading an array of `count` elements of type `T`.
    /// Only use this for interfacing with unsafe C-like APIs.
    ///
    /// The `count` argument is purely for bounds-checking and does not affect
    /// the result.
    ///
    /// No guarantee is made about the alignment of the resulting pointer!
    /// Pointers that are well-aligned for the guest are not necessarily
    /// well-aligned for the host. Rust strictly requires pointers to be
    /// well-aligned when dereferencing them, or when constructing references or
    /// slices from them, so **be very careful**.
    pub fn ptr_at<T, const MUT: bool>(&self, ptr: Ptr<T, MUT>, count: GuestUSize) -> *const T
    where
        T: SafeRead,
    {
        let size = count.checked_mul(guest_size_of::<T>()).unwrap();
        self.bytes_at(ptr.cast(), size).as_ptr().cast()
    }
    /// Get a pointer for reading or writing to an array of `count` elements of
    /// type `T`. Only use this for interfacing with unsafe C-like APIs.
    ///
    /// The `count` argument is purely for bounds-checking and does not affect
    /// the result.
    ///
    /// No guarantee is made about the alignment of the resulting pointer!
    /// Pointers that are well-aligned for the guest are not necessarily
    /// well-aligned for the host. Rust strictly requires pointers to be
    /// well-aligned when dereferencing them, or when constructing references or
    /// slices from them, so **be very careful**.
    pub fn ptr_at_mut<T>(&mut self, ptr: MutPtr<T>, count: GuestUSize) -> *mut T
    where
        T: SafeRead + SafeWrite,
    {
        let size = count.checked_mul(guest_size_of::<T>()).unwrap();
        self.bytes_at_mut(ptr.cast(), size).as_mut_ptr().cast()
    }

    /// Read a value for memory. This is the preferred way to read memory in
    /// most cases.
    pub fn read<T, const MUT: bool>(&self, ptr: Ptr<T, MUT>) -> T
    where
        T: SafeRead,
    {
        // This is unsafe unless we are careful with which types SafeRead is
        // implemented for!
        // This would also be unsafe if the non-unaligned method was used.
        unsafe { self.ptr_at(ptr, 1).read_unaligned() }
    }
    /// Write a value to memory. This is the preferred way to write memory in
    /// most cases.
    pub fn write<T>(&mut self, ptr: MutPtr<T>, value: T)
    where
        T: SafeWrite,
    {
        let size = guest_size_of::<T>();
        assert!(size > 0);
        let slice = self.bytes_at_mut(ptr.cast(), size);
        let ptr: *mut T = slice.as_mut_ptr().cast();
        // It's unaligned because what is well-aligned for the guest is not
        // necessarily well-aligned for the host.
        // This would be unsafe if the non-unaligned method was used.
        unsafe { ptr.write_unaligned(value) }
    }

    /// Allocate `size` bytes.
    pub fn alloc(&mut self, size: GuestUSize) -> MutVoidPtr {
        let ptr = Ptr::from_bits(self.allocator.alloc(size));
        log_dbg!("Allocated {:?} ({:#x} bytes)", ptr, size);
        ptr
    }

    /// Free an allocation made with one of the `alloc` methods on this type.
    pub fn free(&mut self, ptr: MutVoidPtr) {
        let size = self.allocator.free(ptr.to_bits());
        self.bytes_at_mut(ptr.cast(), size).fill(0);
        log_dbg!("Freed {:?} ({:#x} bytes)", ptr, size);
    }

    /// Allocate memory large enough for a value of type `T` and write the value
    /// to it. Equivalent to [Self::alloc] + [Self::write].
    pub fn alloc_and_write<T>(&mut self, value: T) -> MutPtr<T>
    where
        T: SafeWrite,
    {
        let ptr = self.alloc(guest_size_of::<T>()).cast();
        self.write(ptr, value);
        ptr
    }

    /// Allocate and write a C string. This method will add a null terminator,
    /// so it is optimal if the input slice does not already contain one.
    pub fn alloc_and_write_cstr(&mut self, str_bytes: &[u8]) -> MutPtr<u8> {
        let len = str_bytes.len().try_into().unwrap();
        let ptr = self.alloc(len + 1).cast();
        self.bytes_at_mut(ptr, len).copy_from_slice(str_bytes);
        self.write(ptr + len, b'\0');
        ptr
    }

    /// Get a C string (null-terminated) as a slice. The null terminator is not
    /// included in the slice.
    pub fn cstr_at<const MUT: bool>(&self, ptr: Ptr<u8, MUT>) -> &[u8] {
        let mut len = 0;
        while self.read(ptr + len) != b'\0' {
            len += 1;
        }
        self.bytes_at(ptr, len)
    }

    /// Get a C string (null-terminated) as a string slice, panicking if it is
    /// not UTF-8. The null terminator is not included in the slice.
    pub fn cstr_at_utf8<const MUT: bool>(&self, ptr: Ptr<u8, MUT>) -> &str {
        std::str::from_utf8(self.cstr_at(ptr)).unwrap()
    }

    /// Permanently mark a region of address space as being unusable to the
    /// memory allocator.
    pub fn reserve(&mut self, base: VAddr, size: GuestUSize) {
        self.allocator.reserve(allocator::Chunk::new(base, size));
    }
}
