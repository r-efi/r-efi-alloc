//! Global Allocator Bridge
//!
//! The stabilized interface of the rust compiler and standard-library to the global allocator is
//! provided by the `core::alloc::GlobalAlloc` trait and the `global_allocator` attribute. Only
//! one crate in every dependency graph can use the `global_allocator` attribute to mark one
//! static variable as the global allocator. The type of it must implement `GlobalAlloc`. Note
//! that this attribute can only be used in the crate-root, not in sub-modules.
//!
//! UEFI is, however, not a natural fit for the global-allocator trait. On UEFI systems, access to
//! all system APIs is done through the system table, which is passed as argument to the
//! application entry-point. Therefore, it is up to the implementor of the entry-point to set up
//! the global state inherent to rust's global allocator.

use core::sync::atomic;

pub struct Bridge {
    attachment: atomic::AtomicPtr<crate::alloc::Allocator>,
}

pub struct Attachment<'alloc, 'bridge> {
    allocator: &'alloc mut crate::alloc::Allocator,
    bridge: &'bridge Bridge,
}

impl Bridge {
    /// Create new bridge
    ///
    /// The Bridge type represents the global allocator. Since it cannot be instantiated at
    /// compile-time (on UEFI, the system-table address can only be resolved at runtime, since it
    /// is passed as argument to the entry point), it is implemented as a bridge between the
    /// actual allocator object and the global allocator. By default, the bridge object has no
    /// allocator linked. Any allocation requests will thusly yield and allocation error.
    ///
    /// To make use of a bridge, you have to instantiate an allocator object and attach it via the
    /// `attach()` method.
    ///
    /// You can create as many bridges as you like. However, to mark a bridge as global allocator,
    /// you have to make it a global, static variable and annotate it with `#[global_allocator]`.
    /// Only one such variable is allowed to exist in any crate tree, and it must be declared in
    /// the root module of a given trait.
    pub const fn new() -> Bridge {
        Bridge {
            attachment: atomic::AtomicPtr::new(core::ptr::null_mut()),
        }
    }

    unsafe fn raw_attach(&self, ptr: *mut crate::alloc::Allocator) -> Option<()> {
        // Set @ptr as the attachment on this bridge. This only succeeds if there is not already
        // an attachment set.
        // We use a compare_and_swap() to change the attachment if it was NULL. We use Release
        // semantics, so any stores to your allocator are visible once the attachment is written.
        // On error, no ordering guarantees are given, since this interface is not meant to be a
        // programmatic query.
        // Note that the Release pairs with the Acquire in the GlobalAlloc trait below.
        //
        // This interface is unsafe since the caller must guarantee to detach the bridge before it
        // is destroyed. There are not runtime guarantees given by this interface, it is all left
        // to the caller.
        let p =
            self.attachment
                .compare_and_swap(core::ptr::null_mut(), ptr, atomic::Ordering::Release);

        if p.is_null() {
            Some(())
        } else {
            None
        }
    }

    unsafe fn raw_detach(&self, ptr: *mut crate::alloc::Allocator) {
        // Detach @ptr from this bridge. The caller must guarantee @ptr is already attached to the
        // bridge. This function will panic if @ptr is not the current attachment.
        //
        // We use compare_and_swap() to replace the old attachment with NULL. If it was not NULL,
        // we panic. No ordering guarantees are required, since there is no dependent state.
        let p =
            self.attachment
                .compare_and_swap(ptr, core::ptr::null_mut(), atomic::Ordering::Relaxed);
        assert!(p == ptr);
    }

    /// Attach an allocator
    ///
    /// This attaches the allocator given as @allocator to the bridge. If there is already an
    /// allocator attached, this will yield `None`. Otherwise, an attachment is returned that
    /// represents this link. Dropping the attachment will detach the allocator from the bridge.
    ///
    /// As long as an allocator is attached to a bridge, allocations through this bridge (via
    /// rust's `GlobalAlloc` trait) will be served by this allocator.
    ///
    /// This is an unsafe interface. It is the caller's responsibility to guarantee that the
    /// attachment survives all outstanding allocations. That is, any allocated memory must be
    /// released before detaching the allocator.
    pub unsafe fn attach<'alloc, 'bridge>(
        &'bridge self,
        allocator: &'alloc mut crate::alloc::Allocator,
    ) -> Option<Attachment<'alloc, 'bridge>> {
        match self.raw_attach(allocator) {
            None => None,
            Some(()) => Some(Attachment {
                allocator: allocator,
                bridge: self,
            }),
        }
    }
}

impl<'alloc, 'bridge> Drop for Attachment<'alloc, 'bridge> {
    fn drop(&mut self) {
        unsafe {
            self.bridge.raw_detach(self.allocator);
        }
    }
}

// This implements GlobalAlloc for our bridge. This trait is used by the rust ecosystem to serve
// global memory allocations. For this to work, you must have a bridge as static variable
// annotated as `#[global_allocator]`.
//
// We simply forward all allocation requests to the attached allocator. If the allocator is NULL,
// we fail the allocations.
//
// Note that the bridge interface must guarantee that an attachment survives all allocations. That
// is, you must drop/deallocate all memory before dropping your attachment. See the description of
// the bridge interface for details.
unsafe impl core::alloc::GlobalAlloc for Bridge {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let allocator = self.attachment.load(atomic::Ordering::Acquire);

        if allocator.is_null() {
            return core::ptr::null_mut();
        }

        core::alloc::AllocRef::alloc(&mut *allocator, layout)
            .map(|(ptr, _)| ptr.as_ptr())
            .unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let allocator = self.attachment.load(atomic::Ordering::Acquire);

        assert!(!allocator.is_null());

        core::alloc::AllocRef::dealloc(
            &mut *allocator,
            core::ptr::NonNull::new_unchecked(ptr),
            layout,
        );
    }
}
