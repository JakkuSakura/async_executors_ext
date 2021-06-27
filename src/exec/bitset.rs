use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
const SIZE: usize = 128;
pub struct Bitset {
    dense: [AtomicBool; SIZE],
    count: AtomicUsize,
}
#[allow(dead_code)]
impl Bitset {
    pub const fn new() -> Self {
        Self {
            dense: arr_macro::arr![AtomicBool::new(false); 128],
            count: AtomicUsize::new(0),
        }
    }
    pub fn set(&self, i: u32) {
        if self.dense[i as usize]
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            self.count.fetch_add(1, Ordering::AcqRel);
        }
    }
    pub fn unset(&self, i: u32) -> bool {
        let prev = self.dense[i as usize].compare_exchange(
            true,
            false,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        if prev.is_ok() {
            self.count.fetch_sub(1, Ordering::AcqRel);
            prev.unwrap()
        } else {
            prev.unwrap_err()
        }
    }
    pub fn is_set(&self, i: u32) -> bool {
        assert!(i <= SIZE as u32);
        self.dense[i as usize].load(Ordering::Acquire)
    }

    pub fn clear(&self) {
        self.count.store(0, Ordering::Release);
        for i in 0..SIZE {
            self.dense[i].store(true, Ordering::Relaxed)
        }
    }
    pub fn get_count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }
}
