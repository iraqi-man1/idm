//! Dynamic segmentation for multi-connection downloads.
//!
//! The table tracks disjoint byte ranges `[start, end)` that together cover
//! the file. Every segment has two cursors:
//!
//! * `received` – bytes handed to the disk writer (claimed by a connection),
//! * `written`  – bytes the writer has actually written to the file.
//!
//! `start <= written <= received <= end` always holds. Only `written` is
//! persisted, so after a crash the engine resumes from data known to be on
//! disk.
//!
//! Work distribution follows the IDM-style "split the largest remainder"
//! strategy: a new connection first takes over an unassigned segment; if all
//! segments are busy it splits the segment with the largest unreceived
//! remainder in half and takes the upper half. The connection that owned the
//! split segment simply stops when it reaches the new, smaller `end`, so no
//! request has to be re-issued and no byte is ever claimed twice.
//!
//! The table performs no I/O; callers wrap it in a mutex.

use serde::{Deserialize, Serialize};

/// Identifier of a connection (worker) inside one download.
pub type WorkerId = u64;

/// Marker used for the end of an open-ended segment (unknown file size).
pub const OPEN_END: u64 = u64::MAX;

/// Alignment of split points; keeps writes page aligned.
const SPLIT_ALIGN: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start: u64,
    /// Exclusive end; [`OPEN_END`] while the file size is unknown.
    pub end: u64,
    pub received: u64,
    pub written: u64,
    pub worker: Option<WorkerId>,
}

impl Segment {
    pub fn is_open_ended(&self) -> bool {
        self.end == OPEN_END
    }

    /// Bytes not yet claimed by a connection.
    pub fn unreceived(&self) -> u64 {
        self.end.saturating_sub(self.received)
    }

    pub fn is_received(&self) -> bool {
        !self.is_open_ended() && self.received >= self.end
    }

    pub fn is_done(&self) -> bool {
        !self.is_open_ended() && self.written >= self.end
    }

    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Segment state that is persisted between sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedSegment {
    pub start: u64,
    /// `None` for an open-ended segment.
    pub end: Option<u64>,
    /// Absolute offset up to which data is durably written.
    pub written: u64,
}

/// Work handed to a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    pub index: usize,
    /// Absolute offset to request from.
    pub from: u64,
    /// Exclusive end at the time of assignment (may shrink later).
    pub end: u64,
    /// The assignment was created by splitting a busy segment.
    pub split: bool,
}

/// Result of [`SegmentTable::claim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claim {
    /// Absolute file offset for the accepted bytes.
    pub offset: u64,
    /// Number of leading bytes of the chunk that belong to the segment.
    pub accepted: u64,
    /// The segment has been fully received; the connection should stop.
    pub finished: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SegmentError {
    #[error("persisted segments do not form a contiguous, non-overlapping cover of the file")]
    InvalidLayout,
    #[error("segment {0} does not exist")]
    NoSuchSegment(usize),
    #[error("segment {0} is not owned by worker {1}")]
    NotOwner(usize, WorkerId),
    #[error("write at offset {offset} is outside segment {index}")]
    WriteOutOfRange { index: usize, offset: u64 },
}

#[derive(Debug, Clone)]
pub struct SegmentTable {
    total: Option<u64>,
    segments: Vec<Segment>,
    min_split: u64,
    /// The server honours range requests.
    ranges: bool,
}

impl SegmentTable {
    /// A fresh table: one segment covering the whole file (or open-ended).
    ///
    /// `splittable` must be false when the server does not honour range
    /// requests; the single segment is then never divided.
    pub fn new(total: Option<u64>, min_split: u64, splittable: bool) -> Self {
        let end = total.unwrap_or(OPEN_END);
        Self {
            total,
            segments: vec![Segment { start: 0, end, received: 0, written: 0, worker: None }],
            min_split: min_split.max(SPLIT_ALIGN),
            ranges: splittable,
        }
    }

    /// Rebuild a table from persisted state, validating its layout.
    pub fn restore(
        total: Option<u64>,
        parts: &[PersistedSegment],
        min_split: u64,
        splittable: bool,
    ) -> Result<Self, SegmentError> {
        if parts.is_empty() {
            return Ok(Self::new(total, min_split, splittable));
        }
        let mut sorted: Vec<PersistedSegment> = parts.to_vec();
        sorted.sort_by_key(|p| p.start);
        let mut expected_start = 0u64;
        let mut segments = Vec::with_capacity(sorted.len());
        for (i, p) in sorted.iter().enumerate() {
            let end = p.end.unwrap_or(OPEN_END);
            let last = i + 1 == sorted.len();
            if p.start != expected_start || end < p.start || p.written < p.start || p.written > end
            {
                return Err(SegmentError::InvalidLayout);
            }
            if end == OPEN_END && (!last || total.is_some()) {
                return Err(SegmentError::InvalidLayout);
            }
            segments.push(Segment {
                start: p.start,
                end,
                received: p.written,
                written: p.written,
                worker: None,
            });
            expected_start = end;
        }
        if let Some(t) = total {
            if expected_start != t {
                return Err(SegmentError::InvalidLayout);
            }
        } else if expected_start != OPEN_END {
            return Err(SegmentError::InvalidLayout);
        }
        Ok(Self {
            total,
            segments,
            min_split: min_split.max(SPLIT_ALIGN),
            ranges: splittable,
        })
    }

    pub fn total(&self) -> Option<u64> {
        self.total
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn segment(&self, index: usize) -> Option<&Segment> {
        self.segments.get(index)
    }

    pub fn is_splittable(&self) -> bool {
        self.ranges && self.total.is_some()
    }

    /// Disable splitting (e.g. the server stopped honouring ranges).
    pub fn set_splittable(&mut self, splittable: bool) {
        self.ranges = splittable;
    }

    pub fn min_split(&self) -> u64 {
        self.min_split
    }

    /// Bytes claimed by connections (includes data still queued for disk).
    pub fn received_bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.received - s.start).sum()
    }

    /// Bytes written to disk.
    pub fn written_bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.written - s.start).sum()
    }

    /// Number of segments that currently have a worker.
    pub fn active_workers(&self) -> usize {
        self.segments.iter().filter(|s| s.worker.is_some()).count()
    }

    /// Every byte of the file has been written.
    pub fn is_complete(&self) -> bool {
        self.total.is_some() && self.segments.iter().all(|s| s.is_done())
    }

    /// Every byte has been claimed (some may still be queued for disk).
    pub fn is_fully_received(&self) -> bool {
        self.total.is_some() && self.segments.iter().all(|s| s.is_received())
    }

    /// Bytes not yet claimed by any connection.
    pub fn unreceived_bytes(&self) -> Option<u64> {
        self.total?;
        Some(self.segments.iter().map(|s| s.unreceived()).sum())
    }

    /// Mark a specific segment as owned by `worker` (used for the probe
    /// connection, which always starts at offset 0).
    pub fn assign(&mut self, index: usize, worker: WorkerId) -> Result<Assignment, SegmentError> {
        let seg = self.segments.get_mut(index).ok_or(SegmentError::NoSuchSegment(index))?;
        seg.worker = Some(worker);
        Ok(Assignment { index, from: seg.received, end: seg.end, split: false })
    }

    /// Find work for a new connection.
    ///
    /// Unassigned unfinished segments are preferred (largest remainder
    /// first); otherwise the busiest segment is split in half if its
    /// remainder is at least twice the minimum split size.
    pub fn acquire(&mut self, worker: WorkerId) -> Option<Assignment> {
        // 1. Free segment with the most remaining data.
        let free = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.worker.is_none() && (s.is_open_ended() || !s.is_received()))
            .max_by_key(|(_, s)| s.unreceived())
            .map(|(i, _)| i);
        if let Some(index) = free {
            let seg = &mut self.segments[index];
            seg.worker = Some(worker);
            return Some(Assignment { index, from: seg.received, end: seg.end, split: false });
        }
        if !self.is_splittable() {
            return None;
        }
        // 2. Split the busy segment with the largest unreceived remainder.
        let (victim, remaining) = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.worker.is_some() && !s.is_open_ended())
            .map(|(i, s)| (i, s.unreceived()))
            .max_by_key(|&(_, r)| r)?;
        if remaining < self.min_split.saturating_mul(2) {
            return None;
        }
        let seg = &self.segments[victim];
        let mid = align_up(seg.received + remaining / 2, SPLIT_ALIGN);
        if mid <= seg.received || mid >= seg.end || seg.end - mid < self.min_split {
            return None;
        }
        let old_end = seg.end;
        self.segments[victim].end = mid;
        self.segments.push(Segment {
            start: mid,
            end: old_end,
            received: mid,
            written: mid,
            worker: Some(worker),
        });
        Some(Assignment { index: self.segments.len() - 1, from: mid, end: old_end, split: true })
    }

    /// Account for `len` bytes received by `worker` for segment `index`.
    ///
    /// Returns how many leading bytes belong to the segment. Bytes past the
    /// (possibly shrunk) end must be discarded by the caller.
    pub fn claim(&mut self, index: usize, worker: WorkerId, len: u64) -> Result<Claim, SegmentError> {
        let seg = self.segments.get_mut(index).ok_or(SegmentError::NoSuchSegment(index))?;
        if seg.worker != Some(worker) {
            return Err(SegmentError::NotOwner(index, worker));
        }
        let offset = seg.received;
        let accepted = len.min(seg.end - seg.received);
        seg.received += accepted;
        let finished = !seg.is_open_ended() && seg.received >= seg.end;
        Ok(Claim { offset, accepted, finished })
    }

    /// Record that `[offset, offset+len)` of segment `index` was written.
    ///
    /// Writes of a segment arrive in order (one connection per segment,
    /// one writer per file), so `written` simply advances.
    pub fn mark_written(&mut self, index: usize, offset: u64, len: u64) -> Result<(), SegmentError> {
        let seg = self.segments.get_mut(index).ok_or(SegmentError::NoSuchSegment(index))?;
        let end = offset.saturating_add(len);
        if offset < seg.start || end > seg.received {
            return Err(SegmentError::WriteOutOfRange { index, offset });
        }
        if end > seg.written {
            seg.written = end;
        }
        Ok(())
    }

    /// The worker stopped serving the segment (finished, failed or paused).
    /// The next connection resumes at `received`: data already claimed is
    /// guaranteed to reach the writer.
    pub fn release(&mut self, index: usize, worker: WorkerId) {
        if let Some(seg) = self.segments.get_mut(index) {
            if seg.worker == Some(worker) {
                seg.worker = None;
            }
        }
    }

    /// Release every segment (all connections stopped).
    pub fn release_all(&mut self) {
        for s in &mut self.segments {
            s.worker = None;
        }
    }

    /// The size became known while downloading an open-ended segment
    /// (single connection, EOF reached). Closes the table at `received`.
    pub fn close_at_eof(&mut self) -> u64 {
        let total: u64 = self
            .segments
            .iter()
            .map(|s| if s.is_open_ended() { s.received } else { s.end })
            .max()
            .unwrap_or(0);
        for s in &mut self.segments {
            if s.is_open_ended() {
                s.end = s.received;
            }
        }
        self.total = Some(total);
        total
    }

    /// Discard all progress (restart from zero).
    pub fn reset(&mut self, total: Option<u64>) {
        *self = Self::new(total, self.min_split, self.ranges);
    }

    /// Persistable view, ordered by offset. Adjacent finished segments are
    /// merged to keep the stored table small.
    pub fn snapshot(&self) -> Vec<PersistedSegment> {
        let mut parts: Vec<PersistedSegment> = self
            .segments
            .iter()
            .map(|s| PersistedSegment {
                start: s.start,
                end: if s.is_open_ended() { None } else { Some(s.end) },
                written: s.written,
            })
            .collect();
        parts.sort_by_key(|p| p.start);
        let mut merged: Vec<PersistedSegment> = Vec::with_capacity(parts.len());
        for p in parts {
            if let Some(last) = merged.last_mut() {
                let last_done = last.end.is_some_and(|e| last.written == e);
                if last_done && last.end == Some(p.start) {
                    // Extend the finished prefix: [last.start, p.end) with
                    // written progress continuing contiguously.
                    last.end = p.end;
                    last.written = p.written;
                    continue;
                }
            }
            merged.push(p);
        }
        merged
    }

    /// Offsets of finished-but-unwritten ranges are impossible by design;
    /// this checks the internal invariants (used by tests and debug builds).
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut sorted: Vec<&Segment> = self.segments.iter().collect();
        sorted.sort_by_key(|s| s.start);
        let mut expect = 0u64;
        for s in sorted {
            if s.start != expect {
                return Err(format!("gap/overlap at {} (expected {})", s.start, expect));
            }
            if !(s.start <= s.written && s.written <= s.received && s.received <= s.end) {
                return Err(format!("cursor order violated: {s:?}"));
            }
            expect = s.end;
        }
        match self.total {
            Some(t) if expect != t => Err(format!("cover ends at {expect}, total {t}")),
            None if expect != OPEN_END => Err("open-ended table not open".into()),
            _ => Ok(()),
        }
    }
}

fn align_up(v: u64, align: u64) -> u64 {
    v.div_ceil(align).saturating_mul(align)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn drain(table: &mut SegmentTable, a: Assignment, worker: WorkerId, chunk: u64) -> u64 {
        let mut total = 0;
        loop {
            let c = table.claim(a.index, worker, chunk).unwrap();
            if c.accepted > 0 {
                table.mark_written(a.index, c.offset, c.accepted).unwrap();
            }
            total += c.accepted;
            if c.finished || c.accepted == 0 {
                break;
            }
        }
        table.release(a.index, worker);
        total
    }

    #[test]
    fn single_segment_full_download() {
        let mut t = SegmentTable::new(Some(10 * MIB), MIB, true);
        let a = t.assign(0, 1).unwrap();
        assert_eq!((a.from, a.end), (0, 10 * MIB));
        assert_eq!(drain(&mut t, a, 1, 64 * 1024), 10 * MIB);
        assert!(t.is_complete());
        t.check_invariants().unwrap();
    }

    #[test]
    fn split_largest_remainder() {
        let mut t = SegmentTable::new(Some(16 * MIB), MIB, true);
        t.assign(0, 1).unwrap();
        let b = t.acquire(2).unwrap();
        assert!(b.split);
        assert_eq!(b.from, 8 * MIB);
        assert_eq!(t.segment(0).unwrap().end, 8 * MIB);
        let c = t.acquire(3).unwrap();
        // Two halves of 8 MiB: the first one found with max remainder is split.
        assert_eq!(c.end - c.from, 4 * MIB);
        t.check_invariants().unwrap();
    }

    #[test]
    fn claim_truncates_after_split() {
        let mut t = SegmentTable::new(Some(4 * MIB), MIB, true);
        t.assign(0, 1).unwrap();
        // Worker 1 has received 1 MiB.
        let c = t.claim(0, 1, MIB).unwrap();
        t.mark_written(0, c.offset, c.accepted).unwrap();
        let b = t.acquire(2).unwrap();
        // remaining 3 MiB -> split at 1 MiB + 1.5 MiB = 2.5 MiB
        assert_eq!(b.from, 2 * MIB + MIB / 2);
        // Worker 1 now receives a 2 MiB chunk; only 1.5 MiB belongs to it.
        let c = t.claim(0, 1, 2 * MIB).unwrap();
        assert_eq!(c.accepted, MIB + MIB / 2);
        assert!(c.finished);
        t.check_invariants().unwrap();
    }

    #[test]
    fn no_split_below_minimum() {
        let mut t = SegmentTable::new(Some(MIB + 100), MIB, true);
        t.assign(0, 1).unwrap();
        assert!(t.acquire(2).is_none());
    }

    #[test]
    fn unsplittable_table_never_splits() {
        let mut t = SegmentTable::new(Some(100 * MIB), MIB, false);
        t.assign(0, 1).unwrap();
        assert!(t.acquire(2).is_none());
        let mut open = SegmentTable::new(None, MIB, true);
        open.assign(0, 1).unwrap();
        assert!(open.acquire(2).is_none());
    }

    #[test]
    fn released_segment_is_reacquired_from_received() {
        let mut t = SegmentTable::new(Some(8 * MIB), MIB, true);
        t.assign(0, 1).unwrap();
        let c = t.claim(0, 1, 3 * MIB).unwrap();
        t.mark_written(0, c.offset, c.accepted).unwrap();
        t.release(0, 1);
        let a = t.acquire(9).unwrap();
        assert_eq!((a.index, a.from, a.split), (0, 3 * MIB, false));
        assert!(matches!(t.claim(0, 1, 1), Err(SegmentError::NotOwner(0, 1))));
    }

    #[test]
    fn snapshot_restore_roundtrip_and_merge() {
        let mut t = SegmentTable::new(Some(32 * MIB), MIB, true);
        t.assign(0, 1).unwrap();
        let others: Vec<_> = (2..6).map(|w| (w, t.acquire(w).unwrap())).collect();
        // Finish the first segment completely, the others partially.
        let first_end = t.segment(0).unwrap().end;
        let c = t.claim(0, 1, first_end).unwrap();
        t.mark_written(0, c.offset, c.accepted).unwrap();
        for (w, a) in &others {
            let c = t.claim(a.index, *w, 100_000).unwrap();
            t.mark_written(a.index, c.offset, c.accepted).unwrap();
        }
        let snap = t.snapshot();
        let restored = SegmentTable::restore(Some(32 * MIB), &snap, MIB, true).unwrap();
        restored.check_invariants().unwrap();
        assert_eq!(restored.written_bytes(), t.written_bytes());
        assert!(snap.len() <= t.segments().len());
    }

    #[test]
    fn restore_rejects_bad_layouts() {
        let bad = [
            PersistedSegment { start: 0, end: Some(10), written: 5 },
            PersistedSegment { start: 12, end: Some(20), written: 12 },
        ];
        assert!(SegmentTable::restore(Some(20), &bad, MIB, true).is_err());
        let overflow = [PersistedSegment { start: 0, end: Some(10), written: 11 }];
        assert!(SegmentTable::restore(Some(10), &overflow, MIB, true).is_err());
        let wrong_total = [PersistedSegment { start: 0, end: Some(10), written: 0 }];
        assert!(SegmentTable::restore(Some(11), &wrong_total, MIB, true).is_err());
    }

    #[test]
    fn open_ended_closes_at_eof() {
        let mut t = SegmentTable::new(None, MIB, true);
        t.assign(0, 1).unwrap();
        for _ in 0..10 {
            let c = t.claim(0, 1, 1000).unwrap();
            assert!(!c.finished);
            t.mark_written(0, c.offset, c.accepted).unwrap();
        }
        assert_eq!(t.close_at_eof(), 10_000);
        assert!(t.is_complete());
        t.check_invariants().unwrap();
    }

    /// Randomised simulation: many workers, random chunk sizes, random
    /// failures. Every byte must be claimed exactly once and the file must
    /// end fully covered.
    #[test]
    fn randomized_no_duplicate_bytes() {
        use rand::{Rng, SeedableRng};
        for seed in 0..40u64 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let total = rng.gen_range(1..64 * MIB);
            let mut t = SegmentTable::new(Some(total), 256 * 1024, true);
            let mut claimed: u64 = 0;
            let mut ranges: Vec<(u64, u64)> = Vec::new();
            let mut active: Vec<(WorkerId, Assignment)> = vec![(0, t.assign(0, 0).unwrap())];
            let mut next_worker = 1;
            let mut steps = 0;
            while !t.is_complete() {
                steps += 1;
                assert!(steps < 1_000_000, "no progress");
                if active.len() < 8 && rng.gen_bool(0.3) {
                    if let Some(a) = t.acquire(next_worker) {
                        active.push((next_worker, a));
                    }
                    next_worker += 1;
                }
                if active.is_empty() {
                    let a = t.acquire(next_worker).expect("work must remain");
                    active.push((next_worker, a));
                    next_worker += 1;
                }
                let i = rng.gen_range(0..active.len());
                let (w, a) = active[i];
                if rng.gen_bool(0.02) {
                    // connection failure
                    t.release(a.index, w);
                    active.swap_remove(i);
                    continue;
                }
                let chunk = rng.gen_range(1..300_000);
                let c = t.claim(a.index, w, chunk).unwrap();
                if c.accepted > 0 {
                    ranges.push((c.offset, c.offset + c.accepted));
                    claimed += c.accepted;
                    t.mark_written(a.index, c.offset, c.accepted).unwrap();
                }
                if c.finished {
                    t.release(a.index, w);
                    active.swap_remove(i);
                }
                t.check_invariants().unwrap();
            }
            assert_eq!(claimed, total, "seed {seed}");
            ranges.sort();
            let mut pos = 0;
            for (s, e) in ranges {
                assert_eq!(s, pos, "seed {seed}: gap or duplicate at {s}");
                pos = e;
            }
            assert_eq!(pos, total);
        }
    }
}
