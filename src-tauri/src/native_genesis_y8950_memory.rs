//! Writable Y8950 ADPCM RAM, prepared and reclaimed on the control thread.
//! The callback exchanges full pages and copies at most two boundary pages.
use super::extra_budget::{Budget, Permit};
use std::{
    cell::UnsafeCell,
    sync::{Arc, Mutex},
};
pub const LIMIT: usize = 2 * 1024 * 1024;
pub const PAGE: usize = 4096;
pub const PAGES: usize = LIMIT / PAGE;
const PAGE_CHARGE: usize = PAGE + 128;
#[derive(Debug)]
struct Page {
    bytes: UnsafeCell<[u8; PAGE]>,
    _permit: Permit,
}
// Only the callback accesses bytes after publication; control only retains and
// releases Arcs, never reads a live page or changes it behind the callback.
unsafe impl Send for Page {}
unsafe impl Sync for Page {}
#[derive(Debug)]
pub struct Upload {
    address: usize,
    pages: Vec<Arc<Page>>,
}
#[derive(Debug)]
pub struct Arena {
    budget: Arc<Budget>,
    retained: Mutex<Vec<Arc<Page>>>,
}
impl Default for Arena {
    fn default() -> Self {
        Self::new(Arc::new(Budget::default()))
    }
}
impl Arena {
    pub fn new(budget: Arc<Budget>) -> Self {
        Self {
            budget,
            retained: Mutex::new(Vec::new()),
        }
    }
    pub fn collect(&self) {
        if let Ok(mut pages) = self.retained.lock() {
            pages.retain(|page| Arc::strong_count(page) > 1);
            if pages.capacity() > 64.max(pages.len().saturating_mul(2)) {
                pages.shrink_to_fit();
            }
        }
    }
    pub fn prepare(&self, address: usize, bytes: &[u8]) -> Result<Arc<Upload>, String> {
        validate(address, bytes.len())?;
        self.collect();
        let count = (address + bytes.len()).div_ceil(PAGE);
        // Reserve every page before allocating any data. Failed admission has
        // no applied-page or playback mutation and drops reservations here.
        let permits = (0..count)
            .map(|_| self.budget.reserve(PAGE_CHARGE))
            .collect::<Result<Vec<_>, _>>()?;
        let pages: Vec<_> = permits
            .into_iter()
            .map(|permit| {
                Arc::new(Page {
                    bytes: UnsafeCell::new([0; PAGE]),
                    _permit: permit,
                })
            })
            .collect();
        for (index, page) in pages.iter().enumerate() {
            let start = address.max(index * PAGE);
            let end = (address + bytes.len()).min((index + 1) * PAGE);
            if start < end {
                unsafe {
                    (&mut *page.bytes.get())[start - index * PAGE..end - index * PAGE]
                        .copy_from_slice(&bytes[start - address..end - address]);
                }
            }
        }
        let upload = Arc::new(Upload { address, pages });
        self.retained
            .lock()
            .map_err(|_| "Y8950 ADPCM memory control unavailable")?
            .extend(upload.pages.iter().cloned());
        Ok(upload)
    }
}
pub fn validate(address: usize, size: usize) -> Result<(), String> {
    if address > LIMIT || size > LIMIT - address {
        return Err("Y8950 ADPCM memory range must fit within 2 MiB".into());
    }
    Ok(())
}
pub struct Applied {
    pages: Vec<Option<Arc<Page>>>,
    size: usize,
}
impl Default for Applied {
    fn default() -> Self {
        Self::new()
    }
}
impl Applied {
    pub fn new() -> Self {
        Self {
            pages: (0..PAGES).map(|_| None).collect(),
            size: 0,
        }
    }
    pub fn size(&self) -> u32 {
        self.size as u32
    }
    pub fn clear(&mut self, mut install: impl FnMut(usize, *mut u8)) {
        for (index, page) in self.pages.iter_mut().enumerate() {
            install(index, std::ptr::null_mut());
            *page = None;
        }
        self.size = 0;
    }
    pub fn apply(
        &mut self,
        upload: &Upload,
        bytes: &[u8],
        mut install: impl FnMut(usize, *mut u8),
    ) {
        let end = upload.address + bytes.len();
        for (index, candidate) in upload.pages.iter().enumerate() {
            let base = index * PAGE;
            let start = upload.address.max(base);
            let stop = end.min(base + PAGE);
            if start >= stop && self.pages[index].is_some() {
                continue;
            }
            if start < stop && (start != base || stop != base + PAGE) {
                if let Some(old) = &self.pages[index] {
                    if !Arc::ptr_eq(old, candidate) {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                (*old.bytes.get()).as_ptr(),
                                (*candidate.bytes.get()).as_mut_ptr(),
                                PAGE,
                            );
                        }
                    }
                }
                unsafe {
                    (&mut *candidate.bytes.get())[start - base..stop - base]
                        .copy_from_slice(&bytes[start - upload.address..stop - upload.address]);
                }
            }
            install(index, candidate.bytes.get().cast::<u8>());
            self.pages[index] = Some(candidate.clone());
        }
        self.size = self.size.max(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn y8950_page_cow_preserves_runtime_ram_writes_and_partial_neighbors() {
        let budget = Arc::new(Budget::new(8 * 1024 * 1024));
        let arena = Arena::new(budget.clone());
        let mut applied = Applied::new();
        let upload = arena.prepare(0, &vec![17; PAGE * 3]).unwrap();
        applied.apply(&upload, &vec![17; PAGE * 3], |_, _| {});
        drop(upload);
        // Equivalent to an external CPU data-port write after preparation.
        let patch = arena.prepare(PAGE - 1, &[1, 2, 3]).unwrap();
        unsafe {
            (*applied.pages[0].as_ref().unwrap().bytes.get())[7] = 99;
            (*applied.pages[1].as_ref().unwrap().bytes.get())[500] = 88;
        }
        let untouched = Arc::as_ptr(applied.pages[2].as_ref().unwrap());
        let mut installed = Vec::new();
        applied.apply(&patch, &[1, 2, 3], |i, _| installed.push(i));
        assert_eq!(installed, [0, 1]);
        unsafe {
            assert_eq!((*applied.pages[0].as_ref().unwrap().bytes.get())[7], 99);
            assert_eq!(
                (*applied.pages[0].as_ref().unwrap().bytes.get())[PAGE - 1],
                1
            );
            assert_eq!((*applied.pages[1].as_ref().unwrap().bytes.get())[0], 2);
            assert_eq!((*applied.pages[1].as_ref().unwrap().bytes.get())[1], 3);
            assert_eq!((*applied.pages[1].as_ref().unwrap().bytes.get())[500], 88);
        }
        assert_eq!(Arc::as_ptr(applied.pages[2].as_ref().unwrap()), untouched);
        assert_eq!(applied.size(), (PAGE * 3) as u32);
        drop(patch);
        arena.collect();
        assert_eq!(budget.used(), PAGE_CHARGE * 3);
        let weak = Arc::downgrade(applied.pages[0].as_ref().unwrap());
        applied.clear(|_, _| {});
        assert!(
            weak.upgrade().is_some(),
            "callback page release is retained on control"
        );
        arena.collect();
        assert!(weak.upgrade().is_none());
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn y8950_candidate_gap_pages_survive_clear_and_reordered_uploads() {
        let arena = Arena::default();
        let mut applied = Applied::new();
        let late = arena.prepare(PAGE * 2 + 3, &[77]).unwrap();
        let early = arena.prepare(0, &[55]).unwrap();
        applied.apply(&early, &[55], |_, _| {});
        applied.clear(|_, _| {});
        applied.apply(&late, &[77], |_, _| {});
        assert_eq!(applied.size(), (PAGE * 2 + 4) as u32);
        unsafe {
            assert_eq!((*applied.pages[0].as_ref().unwrap().bytes.get())[0], 0);
            assert_eq!((*applied.pages[2].as_ref().unwrap().bytes.get())[3], 77);
        }
        // Bytes outside the new payload do not shrink a larger allocation.
        applied.apply(&early, &[55], |_, _| {});
        assert_eq!(applied.size(), (PAGE * 2 + 4) as u32);
    }
    #[test]
    fn y8950_preparation_reserves_entire_candidate_range_before_allocating_pages() {
        let budget = Arc::new(Budget::new(PAGE_CHARGE * 2));
        let arena = Arena::new(budget.clone());
        assert!(arena.prepare(PAGE * 2, &[1]).is_err());
        assert_eq!(budget.used(), 0);
        assert!(arena.prepare(LIMIT, &[1]).is_err());
        assert_eq!(budget.used(), 0);
        let upload = arena.prepare(PAGE, &[2]).unwrap();
        assert_eq!(budget.used(), PAGE_CHARGE * 2);
        drop(upload);
        arena.collect();
        assert_eq!(budget.used(), 0);
    }
}

#[cfg(test)]
pub(crate) mod allocation_probe {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };
    thread_local! { static COUNTS: Cell<Option<(usize,usize)>> = const { Cell::new(None) }; }
    struct Allocator;
    #[global_allocator]
    static ALLOCATOR: Allocator = Allocator;
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            COUNTS.with(|c| {
                if let Some((a, d)) = c.get() {
                    c.set(Some((a + 1, d)));
                }
            });
            System.alloc(layout)
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            COUNTS.with(|c| {
                if let Some((a, d)) = c.get() {
                    c.set(Some((a + 1, d)));
                }
            });
            System.alloc_zeroed(layout)
        }
        unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
            COUNTS.with(|c| {
                if let Some((a, d)) = c.get() {
                    c.set(Some((a, d + 1)));
                }
            });
            System.dealloc(p, layout)
        }
        unsafe fn realloc(&self, p: *mut u8, layout: Layout, n: usize) -> *mut u8 {
            COUNTS.with(|c| {
                if let Some((a, d)) = c.get() {
                    c.set(Some((a + 1, d + 1)));
                }
            });
            System.realloc(p, layout, n)
        }
    }
    pub fn measure(f: impl FnOnce()) -> (usize, usize) {
        COUNTS.with(|c| c.set(Some((0, 0))));
        f();
        COUNTS.with(|c| c.replace(None).unwrap())
    }
}
