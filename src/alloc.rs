//! UEFI Memory Allocators
//!
//! This module provides a memory allocator that integrates with the UEFI pool
//! allocator. It exports an `Allocator` type that wraps a System-Table
//! together with a UEFI memory type and forwards memory requests to the UEFI
//! pool allocator.
//!
//! The allocator implements the `core::alloc::Allocator` API defined by the
//! rust standard library. Apart from the constructors, no private extensions
//! are defined. For documentation on the allocation-API, see the rust standard
//! library.
//!
//! Note that `core::alloc::Allocator` is marked unstable as of time of this
//! crate-release. That is, future versions of this trait definition might be
//! incompatible to the current version. Make sure you use a crate-version that
//! matches your standard-library. Furthermore, this module is only provided if
//! the `allocator_api` feature is enabled.

use r_efi::efi;

/// Memory Allocator
///
/// This crate implements a rust memory allocator that forwards requests to the
/// UEFI pool allocator. It takes a System-Table as input, as well as the
/// memory type to use as backing, and then forwards all memory allocation
/// requests to the `AllocatePool()` UEFI system.
///
/// The `core::alloc::Allocator` trait is implemented for this allocator.
/// Hence, this allocator can also be used to back the global memory-allocator
/// of `liballoc` (or `libstd`). See the `Global` type for an implementation of
/// the global allocator.
pub struct Allocator {
    system_table: *mut efi::SystemTable,
    memory_type: efi::MemoryType,
}

impl Allocator {
    /// Create Allocator from UEFI System-Table
    ///
    /// This creates a new Allocator object from a UEFI System-Table pointer
    /// and the memory-type to use for allocations. That is, all allocations on
    /// this object will be tunnelled through the `AllocatePool` API on the
    /// given System-Table. Allocations will always use the memory type given
    /// as `memtype`.
    ///
    /// Note that this interface is unsafe, since the caller must guarantee
    /// that the System-Table is valid for as long as the Allocator is.
    /// Furthermore, the caller must guarantee validity of the
    /// system-table-interface. The latter is usually guaranteed by the
    /// provider of the System-Table. The former is usually just a matter of
    /// tearing down the allocator before returning from your application
    /// entry-point.
    pub unsafe fn from_system_table(
        st: *mut efi::SystemTable,
        memtype: efi::MemoryType,
    ) -> Allocator {
        Allocator {
            system_table: st,
            memory_type: memtype,
        }
    }

    /// Allocate Memory from UEFI Boot-Services
    ///
    /// Use the UEFI `allocate_pool` boot-services to request a block of memory
    /// satisfying the given memory layout. The memory type tied to this
    /// allocator object is used.
    ///
    /// This returns a null-pointer if the allocator could not serve the
    /// request (which on UEFI implies out-of-memory). Otherwise, a non-null
    /// pointer to the aligned block is returned.
    ///
    /// Safety
    /// ------
    ///
    /// To ensure safety of this interface, the caller must guarantee:
    ///
    ///  * The allocation size must not be 0. The function will panic
    ///    otherwise.
    ///
    ///  * The returned pointer is not necessarily the same pointer as returned
    ///    by `allocate_pool` of the boot-services. A caller must not assume
    ///    this when forwarding the pointer to other allocation services
    ///    outside of this module.
    pub unsafe fn alloc(self, layout: core::alloc::Layout) -> *mut u8 {
        crate::raw::alloc(self.system_table, layout, self.memory_type)
    }

    /// Deallocate Memory from UEFI Boot-Services
    ///
    /// Use the UEFI `free_pool` boot-services to release a block of memory
    /// previously allocated through `alloc()`.
    ///
    /// Safety
    /// ------
    ///
    /// To ensure safety of this interface, the caller must guarantee:
    ///
    ///  * The memory block must be the same as previously returned by a call
    ///    to `alloc()`. Every memory block must be released exactly once.
    ///
    ///  * The passed layout must match the layout used to allocate the memory
    ///    block.
    pub unsafe fn dealloc(self, ptr: *mut u8, layout: core::alloc::Layout) {
        crate::raw::dealloc(self.system_table, ptr, layout)
    }
}

unsafe impl core::alloc::Allocator for Allocator {
    fn allocate(
        &self,
        layout: core::alloc::Layout,
    ) -> Result<core::ptr::NonNull<[u8]>, core::alloc::AllocError> {
        let size = layout.size();
        let align = layout.align();

        // We overallocate in case of high alignment requirements. Make sure
        // those do not overflow, and if they do refuse the allocation as there
        // cannot be enough address-space to serve it.
        if size.checked_add(align).is_none() {
            return Err(core::alloc::AllocError);
        }

        let ptr = if size > 0 {
            unsafe {
                crate::raw::alloc(self.system_table, layout, self.memory_type)
            }
        } else {
            layout.dangling().as_ptr() as *mut _
        };

        if ptr.is_null() {
            Err(core::alloc::AllocError)
        } else {
            Ok(
                core::ptr::NonNull::new(
                    core::ptr::slice_from_raw_parts(ptr, size) as *mut _,
                ).unwrap(),
            )
        }
    }

    unsafe fn deallocate(
        &self,
        ptr: core::ptr::NonNull<u8>,
        layout: core::alloc::Layout,
    ) {
        if layout.size() != 0 {
            crate::raw::dealloc(self.system_table, ptr.as_ptr(), layout)
        }
    }
}
