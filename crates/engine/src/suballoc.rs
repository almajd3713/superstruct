//! Offset sub-allocator with a coalescing free list, for the shared vertex and index buffers.
//!
//! Offsets and sizes are in elements (vertices or indices), so an offset is directly usable as
//! `base_vertex` or `first_index`.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub offset: u32,
    pub len: u32,
}

pub struct SubAllocator {
    capacity: u32,
    /// Free spans keyed by offset; neighbours are always merged.
    free: BTreeMap<u32, u32>,
    used: u32,
}

impl SubAllocator {
    pub fn new(capacity: u32) -> Self {
        let mut free = BTreeMap::new();
        if capacity > 0 {
            free.insert(0, capacity);
        }
        Self {
            capacity,
            free,
            used: 0,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    /// Best fit, so large spans survive for large requests. `None` when no single span fits.
    pub fn alloc(&mut self, len: u32) -> Option<Range> {
        if len == 0 {
            return None;
        }
        let (&offset, &span) = self
            .free
            .iter()
            .filter(|&(_, &s)| s >= len)
            .min_by_key(|&(&o, &s)| (s, o))?;
        self.free.remove(&offset);
        if span > len {
            self.free.insert(offset + len, span - len);
        }
        self.used += len;
        Some(Range { offset, len })
    }

    /// Returns `r` to the free list. `r` must come from [`alloc`](Self::alloc) and be freed once.
    pub fn free(&mut self, r: Range) {
        debug_assert!(r.offset + r.len <= self.capacity);
        let mut offset = r.offset;
        let mut len = r.len;
        if let Some((&po, &pl)) = self.free.range(..offset).next_back()
            && po + pl == offset
        {
            self.free.remove(&po);
            offset = po;
            len += pl;
        }
        if let Some(&nl) = self.free.get(&(offset + len)) {
            self.free.remove(&(offset + len));
            len += nl;
        }
        self.free.insert(offset, len);
        self.used -= r.len;
    }

    pub fn largest_free(&self) -> u32 {
        self.free.values().copied().max().unwrap_or(0)
    }

    /// Share of free space that cannot hold a request of `typical` elements, because it sits
    /// in spans shorter than that. 0 means every free element is usable for such a request.
    pub fn fragmentation(&self, typical: u32) -> f32 {
        let total = self.capacity - self.used;
        if total == 0 {
            return 0.0;
        }
        let usable: u32 = self.free.values().filter(|&&s| s >= typical).sum();
        1.0 - usable as f32 / total as f32
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    const TYPICAL: u32 = 20_000;

    #[test]
    fn allocs_are_disjoint_and_free_coalesces() {
        let mut a = SubAllocator::new(100);
        let x = a.alloc(30).unwrap();
        let y = a.alloc(30).unwrap();
        let z = a.alloc(40).unwrap();
        assert!(a.alloc(1).is_none());
        a.free(y);
        a.free(x);
        assert_eq!(a.largest_free(), 60);
        a.free(z);
        assert_eq!(a.largest_free(), 100);
        assert_eq!(a.used(), 0);
        assert_eq!(
            a.alloc(100),
            Some(Range {
                offset: 0,
                len: 100
            })
        );
    }

    #[test]
    fn best_fit_prefers_the_tightest_hole() {
        let mut a = SubAllocator::new(100);
        let r: Vec<_> = [10, 10, 30, 10, 40]
            .iter()
            .map(|&n| a.alloc(n).unwrap())
            .collect();
        a.free(r[0]);
        a.free(r[2]);
        assert_eq!(a.alloc(8).unwrap().offset, 0);
    }

    #[test]
    fn zero_and_oversize_requests_fail() {
        let mut a = SubAllocator::new(10);
        assert!(a.alloc(0).is_none());
        assert!(a.alloc(11).is_none());
    }

    /// Chunk-like churn: sizes spread over two orders of magnitude, about 60% full, the
    /// oldest resident chunks evicted first with a little jitter, as the streamer does when the
    /// player moves.
    #[test]
    fn churn_keeps_fragmentation_under_twenty_percent() {
        let capacity = 4_000_000;
        let mut a = SubAllocator::new(capacity);
        let mut s = 0x9e37_79b9_7f4a_7c15_u64;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let size = |r: u64| 500 + (r % 20_000) as u32 * (1 + (r >> 40) as u32 % 5);
        let mut live: VecDeque<Range> = VecDeque::default();
        while a.used() < capacity / 100 * 60 {
            live.push_back(a.alloc(size(rnd())).unwrap());
        }
        let mut worst = 0.0_f32;
        for _ in 0..50_000 {
            let i = (rnd() % 8) as usize;
            a.free(live.remove(i).unwrap());
            while a.used() < capacity / 100 * 60 {
                match a.alloc(size(rnd())) {
                    Some(r) => live.push_back(r),
                    None => break,
                }
            }
            worst = worst.max(a.fragmentation(TYPICAL));
        }
        assert!(worst < 0.20, "worst fragmentation {worst}");
        let mut spans: Vec<Range> = live.iter().copied().collect();
        spans.sort_by_key(|r| r.offset);
        assert!(
            spans
                .windows(2)
                .all(|w| w[0].offset + w[0].len <= w[1].offset)
        );
    }
}
