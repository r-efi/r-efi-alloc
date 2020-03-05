//! UEFI Memory Allocators
//!
//! This module provides a memory allocator that integrates with the UEFI pool allocator. It
//! exports an `Allocator` type that wraps a System-Table together with a UEFI memory type and
//! forwards memory requests to the UEFI pool allocator.
//!
//! The allocator implements the `core::alloc::AllocRef` API defined by the rust standard library.
//! Apart from the constructors, no private extensions are defined. For documentation on the
//! allocation-API, see the rust standard library.
//!
//! Note that `core::alloc::AllocRef` is marked unstable as of time of this crate-release. That
//! is, future versions of this trait definition might be incompatible to the current version.
//! Make sure you use a crate-version that matches your standard-library.

use r_efi::efi;

// UEFI guarantees 8-byte alignments through `AllocatePool()`. Any request higher than this
// alignment needs to take special precautions to align the returned pointer, and revert that step
// when freeing the memory block again.
const POOL_ALIGNMENT: usize = 8usize;

// Alignment Marker
//
// Since UEFI has no functions to allocate blocks of arbitrary alignment, we have to work around
// this. We extend the allocation size by the required alignment and then offset the pointer
// before returning it. This will properly align the pointer to the given request.
//
// However, when freeing memory again, we have to somehow get back the original pointer.
// Therefore, we store the original address directly in front of the memory block that we just
// aligned. When freeing memory, we simply retrieve this marker and free the original address.
#[repr(C)]
struct Marker(*mut u8);

fn align_request(size: usize, align: usize) -> usize {
    // If the alignment request is within UEFI guarantees, there is no need to adjust the size
    // request. In all other cases, we might have to align the allocated memory block. Hence, we
    // increment the request size by the alignment size.
    // Strictly speaking, we only need `align - POOL_ALIGNMENT` as additional space, since the
    // pool alignment is always guaranteed by UEFI. However, by adding the full alignment we are
    // guaranteed `POOL_ALIGNMENT` extra space. This extra space is used to store a marker so we
    // can retrieve the original pointer when freeing the memory space.
    if align > POOL_ALIGNMENT {
        size + align
    } else {
        size
    }
}

unsafe fn align_block(ptr: *mut u8, align: usize) -> *mut u8 {
    // This function takes a pointer returned by the pool-allocator, and aligns it to the
    // requested alignment. If this alignment is smaller than the guaranteed pool alignment, there
    // is nothing to be done. If it is bigger, we will have to offset the pointer. We rely on the
    // caller using `align_request()` to increase the allocation size beforehand. We then store
    // the original address as `Marker` in front of the aligned pointer, so `unalign_block()` can
    // retrieve it again.
    if align > POOL_ALIGNMENT {
        // In `align_request()` we guarantee that allocation size includes an additional `align`
        // bytes. Since the pool allocation already guaranteed an alignment of `POOL_ALIGNMENT`,
        // we know that `offset >= POOL_ALIGNMENT` here. We then verify that `POOL_ALIGNMENT`
        // serves the needs of our `Marker` object. Note that all but the first assertion are
        // constant expressions, so the compiler will optimize them away.
        let offset = align - (ptr as usize & (align - 1));
        assert!(offset >= POOL_ALIGNMENT);
        assert!(POOL_ALIGNMENT >= core::mem::size_of::<Marker>());
        assert!(POOL_ALIGNMENT >= core::mem::align_of::<Marker>());

        // We calculated the alignment-offset, so adjust the pointer and store the original
        // address directly in front. This will allow `unalign_block()` to retrieve the original
        // address, so it can free the entire memory block.
        let aligned = ptr.add(offset);
        *(aligned as *mut Marker).offset(-1) = Marker(ptr);
        aligned
    } else {
        ptr
    }
}

unsafe fn unalign_block(ptr: *mut u8, align: usize) -> *mut u8 {
    // This undoes what `align_block()` did. That is, we retrieve the original address that was
    // stored directly in front of the aligned block, and return it to the caller. Note that this
    // is only the case if the alignment exceeded the guaranteed alignment of the allocator.
    if align > POOL_ALIGNMENT {
        (*(ptr as *mut Marker).offset(-1)).0
    } else {
        ptr
    }
}

/// Memory Allocator
///
/// This crate implements a rust memory allocator that forwards requests to the UEFI pool
/// allocator. It takes a System-Table as input, as well as the memory type to use as backing, and
/// then forwards all memory allocation requests to the `AllocatePool()` UEFI system.
///
/// The `core::alloc::AllocRef` trait is implemented for this allocator. Hence, this allocator can
/// also be used to back the global memory-allocator of `liballoc` (or `libstd`). See the `Global`
/// type for an implementation of the global allocator.
pub struct Allocator {
    system_table: *mut efi::SystemTable,
    memory_type: efi::MemoryType,
}

impl Allocator {
    /// Create Allocator from UEFI System-Table
    ///
    /// This creates a new Allocator object from a UEFI System-Table pointer and the memory-type
    /// to use for allocations. That is, all allocations on this object will be tunnelled through
    /// the `AllocatePool` API on the given System-Table. Allocations will always use the memory
    /// type given as @memtype.
    ///
    /// Note that this interface is unsafe, since the caller must guarantee that the System-Table
    /// is valid for as long as the Allocator is. Furthermore, the caller must guarantee validity
    /// of the system-table-interface. The latter is usually guaranteed by the provider of the
    /// System-Table. The former is usually just a matter of tearing down the allocator before
    /// returning from your application entry-point.
    pub unsafe fn from_system_table(
        st: *mut efi::SystemTable,
        memtype: efi::MemoryType,
    ) -> Allocator {
        Allocator {
            system_table: st,
            memory_type: memtype,
        }
    }
}

unsafe impl core::alloc::AllocRef for Allocator {
    unsafe fn alloc(
        &mut self,
        layout: core::alloc::Layout,
    ) -> Result<(core::ptr::NonNull<u8>, usize), core::alloc::AllocErr> {
        // We forward the allocation request to `AllocatePool()`. This takes the memory-type and
        // size as argument, and places a pointer to the allocation in an output argument. Note
        // that UEFI guarantees 8-byte alignment (i.e., `POOL_ALIGNMENT`). To support higher
        // alignments, see the `align_request() / align_block() / unalign_block()` helpers.
        let mut ptr: *mut core::ffi::c_void = core::ptr::null_mut();
        let align = layout.align();
        let size = align_request(layout.size(), align);

        let r = ((*(*self.system_table).boot_services).allocate_pool)(
            self.memory_type,
            size,
            &mut ptr,
        );

        // The only real error-scenario is OOM ("out-of-memory"). UEFI does not clearly specify
        // what a return value of NULL+success means (but indicates in a lot of cases that NULL is
        // never a valid pointer). Furthermore, since the 0-page is usually unmapped and not
        // available for EFI_CONVENTIONAL_MEMORY, a NULL pointer cannot be a valid return pointer.
        // Therefore, we treat both a function failure as well as a NULL pointer the same and
        // return `AllocErr`.
        if r.is_error() {
            ptr = core::ptr::null_mut()
        }

        core::ptr::NonNull::new(
            align_block(ptr as *mut u8, align)
        ).ok_or(core::alloc::AllocErr)
         .map(|p| (p, layout.size()))
    }

    unsafe fn dealloc(
        &mut self,
        ptr: core::ptr::NonNull<u8>,
        layout: core::alloc::Layout,
    ) {
        // The spec allows returning errors from `FreePool()`. However, it must serve any valid
        // requests. Only `INVALID_PARAMETER` is listed as possible error. Hence, there is no
        // point in forwarding the return value. We still assert on it to improve diagnostics in
        // early-boot situations. This should be a negligible performance penalty.
        let r = ((*(*self.system_table).boot_services).free_pool)(
            unalign_block(ptr.as_ptr(), layout.align()) as *mut core::ffi::c_void
        );
        assert!(!r.is_error());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align() {
        // UEFI ABI specifies that allocation alignment minimum is always 8. So this can be
        // statically verified.
        assert_eq!(POOL_ALIGNMENT, 8);

        // Loop over allocation-request sizes from 0-256 and alignments from 1-128, and verify
        // that in case of overalignment there is at least space for one additional pointer to
        // store in the allocation.
        for i in 0..256 {
            for j in &[1, 2, 4, 8, 16, 32, 64, 128] {
                if *j <= 8 {
                    assert_eq!(align_request(i, *j), i);
                } else {
                    assert!(align_request(i, *j) > i + std::mem::size_of::<*mut ()>());
                }
            }
        }
    }
}
