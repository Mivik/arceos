use core::iter;

use alloc::sync::Arc;
use axalloc::global_allocator;
use axhal::mem::{phys_to_virt, virt_to_phys};
use axhal::paging::{MappingFlags, PageSize, PageTable};
use memory_addr::{PAGE_SIZE_4K, PhysAddr, VirtAddr};

use super::Backend;

fn alloc_frame(zeroed: bool) -> Option<PhysAddr> {
    let vaddr = VirtAddr::from(global_allocator().alloc_pages(1, PAGE_SIZE_4K).ok()?);
    if zeroed {
        unsafe { core::ptr::write_bytes(vaddr.as_mut_ptr(), 0, PAGE_SIZE_4K) };
    }
    let paddr = virt_to_phys(vaddr);
    Some(paddr)
}

fn dealloc_frame(frame: PhysAddr) {
    let vaddr = phys_to_virt(frame);
    global_allocator().dealloc_pages(vaddr.as_usize(), 1);
}

impl Backend {
    /// Creates a new allocation mapping backend.
    pub fn new_shared(page_num: usize, source: Option<Arc<[PhysAddr]>>) -> Self {
        let pages = if let Some(source) = source {
            assert_eq!(source.len(), page_num);
            source
        } else {
            iter::repeat_with(|| alloc_frame(true).unwrap())
                .take(page_num)
                .collect()
        };
        Self::Shared { pages }
    }

    pub(crate) fn map_shared(
        start: VirtAddr,
        pages: &[PhysAddr],
        flags: MappingFlags,
        pt: &mut PageTable,
    ) -> bool {
        debug!(
            "map_shared: [{:#x}, {:#x}) {:?}",
            start,
            start + pages.len() * PAGE_SIZE_4K,
            flags,
        );
        // allocate all possible physical frames for populated mapping.
        for (i, frame) in pages.iter().enumerate() {
            let addr = start + i * PAGE_SIZE_4K;
            if let Ok(tlb) = pt.map(addr, *frame, PageSize::Size4K, flags) {
                tlb.ignore(); // TLB flush on map is unnecessary, as there are no outdated mappings.
            } else {
                return false;
            }
        }
        true
    }

    pub(crate) fn unmap_shared(
        start: VirtAddr,
        pages: &Arc<[PhysAddr]>,
        pt: &mut PageTable,
    ) -> bool {
        debug!(
            "unmap_shared: [{:#x}, {:#x})",
            start,
            start + pages.len() * PAGE_SIZE_4K
        );
        let should_dealloc = Arc::strong_count(pages) == 1;
        for i in 0..pages.len() {
            let addr = start + i * PAGE_SIZE_4K;
            if let Ok((frame, page_size, tlb)) = pt.unmap(addr) {
                // Deallocate the physical frame if there is a mapping in the
                // page table.
                if page_size.is_huge() {
                    return false;
                }
                tlb.flush();
                // TODO: this is wrong
                if should_dealloc {
                    dealloc_frame(frame);
                }
            } else {
                // Deallocation is needn't if the page is not mapped.
            }
        }
        true
    }
}
