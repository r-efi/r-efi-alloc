//! UEFI Memory Allocator Integration
//!
//! The r-efi-alloc crate provides access to the UEFI memory allocators via rust's allocation
//! traits. The `core::alloc::Alloc` API is implemented on a new type `alloc::Allocator`, which
//! takes the UEFI System-Table and a memory type as input, and then forwards all allocation
//! requests to the UEFI system.
//!
//! Furthermore, this module provides a bridge to allow registering said allocators as global
//! allocators in the rust standard libraries. This bridge is provided as `global::Bridge` and
//! implements the `GlobalAlloc` trait, which is required by rust to provide allocators to the
//! standard library.
//!
//! Note that the allocator API of rust is not stable, as of this writing. While the `GlobalAlloc`
//! trait and its attributes are stabilized as of 1.31.0, several details of the allocation APIs
//! are not. Hence, this crate requires a nightly compiler, and requires your sources to be
//! up-to-date.

// The `core::alloc::Alloc` trait is still unstable and hidden behind the `allocator_api` feature.
// Make sure to enable it, so we can implement this trait. The `alloc_layout_extra` feature
// provides additional extensions to the stable `Layout` object.
#![cfg_attr(feature = "allocator_api", feature(alloc_layout_extra, allocator_api))]
// We need no features of std, so mark the crate as `no_std` (more importantly, `std` might not
// even be available on UEFI systems). However, pull in `std` during tests, so we can run them on
// the host.
#![cfg_attr(not(test), no_std)]

#[cfg(feature = "allocator_api")]
pub mod alloc;
#[cfg(feature = "allocator_api")]
pub mod global;
pub mod raw;
