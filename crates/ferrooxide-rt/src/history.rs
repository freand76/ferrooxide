//! Value history of all non-array signals (for the debug server's interval queries).
//! Fed from the same per-time-step dirty list as the VCD writer: at the end of each time step
//! the changed signals' values are appended to one sequential log (cheap while simulating:
//! no per-signal structures are touched).
//!
//! Log entry: signal id (u32) and the value and unknown planes packed to the signal's width
//! (1, 2 or 4 bytes per plane up to 32 bits, else 8 bytes per word); a 1-bit change takes 6
//! bytes. The times of the time steps and where each starts in the log are stored once.
//! Per-signal indexes are built on demand (`prepare`) for the signals a query reads, scanning
//! only the part of the log not indexed yet.

pub struct History {
    /// times of the time steps in which something changed (ascending)
    steps: Vec<u64>,
    /// per step: offset of its first entry in `log`
    starts: Vec<usize>,
    log: Vec<u8>,
    /// per signal: value words (0: not recorded)
    nw: Vec<u32>,
    /// per signal: bytes per plane
    bpp: Vec<u32>,
    /// per signal: index of its changes, built on demand
    index: Vec<SigIndex>,
    /// bytes used by the log and the step table
    pub bytes: usize,
    /// recording stops when `bytes` reaches this
    pub budget: usize,
    /// the budget was reached at this time (later changes are not recorded)
    pub full_at: Option<u64>,
}

#[derive(Clone, Default)]
struct SigIndex {
    /// the log is indexed for this signal up to this offset
    scanned: usize,
    /// per change: step index and offset of the packed planes in the log
    step: Vec<u32>,
    off: Vec<usize>,
}

fn bytes_per_plane(width: u32, nw: u32) -> u32 {
    match width {
        0..=8 => 1,
        9..=16 => 2,
        17..=32 => 4,
        _ => 8 * nw,
    }
}

impl History {
    /// `widths[sig]`: the signal's width, 0 for signals not recorded.
    pub fn new(widths: &[u32], budget: usize) -> History {
        let nw: Vec<u32> = widths.iter().map(|&w| if w == 0 { 0 } else { w.div_ceil(64) }).collect();
        let bpp = widths.iter().zip(&nw).map(|(&w, &n)| bytes_per_plane(w, n)).collect();
        History {
            steps: Vec::new(),
            starts: Vec::new(),
            log: Vec::new(),
            nw,
            bpp,
            index: vec![SigIndex::default(); widths.len()],
            bytes: 0,
            budget,
            full_at: None,
        }
    }

    /// Record `sig`'s planes at time `t`. At most one call per signal and time step (the
    /// dirty list has each signal once).
    #[inline]
    pub fn record(&mut self, sig: u32, t: u64, planes: &[u64]) {
        let s = sig as usize;
        let (nw, bpp) = (self.nw[s] as usize, self.bpp[s] as usize);
        if nw == 0 || self.full_at.is_some() {
            return;
        }
        if self.bytes >= self.budget {
            self.full_at = Some(t);
            return;
        }
        if self.steps.last() != Some(&t) {
            self.steps.push(t);
            self.starts.push(self.log.len());
            self.bytes += 16;
        }
        let n0 = self.log.len();
        self.log.extend_from_slice(&sig.to_le_bytes());
        if bpp < 8 {
            self.log.extend_from_slice(&planes[0].to_le_bytes()[..bpp]);
            self.log.extend_from_slice(&planes[1].to_le_bytes()[..bpp]);
        } else {
            for w in &planes[..2 * nw] {
                self.log.extend_from_slice(&w.to_le_bytes());
            }
        }
        self.bytes += self.log.len() - n0;
    }

    /// Index the changes of `sigs` recorded so far (one scan of the unindexed part of the log).
    pub fn prepare(&mut self, sigs: &[u32]) {
        let end = self.log.len();
        let mut want = vec![false; self.nw.len()];
        let mut from = end;
        for &s in sigs {
            if self.nw[s as usize] != 0 && self.index[s as usize].scanned < end {
                want[s as usize] = true;
                from = from.min(self.index[s as usize].scanned);
            }
        }
        if from == end {
            return;
        }
        // the step that contains offset `from` (entries never straddle steps)
        let mut k = self.starts.partition_point(|&o| o <= from).saturating_sub(1);
        let mut pos = from;
        let log = &self.log[..end];
        // entry size per signal, looked up once per entry
        let size: Vec<u32> = self.bpp.iter().map(|&b| 4 + 2 * b).collect();
        while pos < end {
            let s = u32::from_le_bytes([log[pos], log[pos + 1], log[pos + 2], log[pos + 3]]) as usize;
            if want[s] && self.index[s].scanned <= pos {
                while k + 1 < self.starts.len() && self.starts[k + 1] <= pos {
                    k += 1;
                }
                self.index[s].step.push(k as u32);
                self.index[s].off.push(pos + 4);
            }
            pos += size[s] as usize;
        }
        for &s in sigs {
            if want[s as usize] {
                self.index[s as usize].scanned = end;
            }
        }
    }

    /// Number of indexed changes of `sig` (call `prepare` first).
    pub fn count(&self, sig: u32) -> usize {
        self.index[sig as usize].step.len()
    }

    /// Time of change `i` of `sig`.
    pub fn time(&self, sig: u32, i: usize) -> u64 {
        self.steps[self.index[sig as usize].step[i] as usize]
    }

    /// Number of changes of `sig` at or before `t`.
    pub fn count_upto(&self, sig: u32, t: u64) -> usize {
        self.index[sig as usize].step.partition_point(|&k| self.steps[k as usize] <= t)
    }

    /// Index of the last change of `sig` at or before `t`.
    pub fn index_at(&self, sig: u32, t: u64) -> Option<usize> {
        self.count_upto(sig, t).checked_sub(1)
    }

    /// The value plane (`nw` words) of change `i` of `sig`, written to `out`.
    pub fn value_plane(&self, sig: u32, i: usize, out: &mut [u64]) {
        let s = sig as usize;
        let (nw, bpp) = (self.nw[s] as usize, self.bpp[s] as usize);
        let d = &self.log[self.index[s].off[i]..];
        if bpp < 8 {
            let mut b = [0u8; 8];
            b[..bpp].copy_from_slice(&d[..bpp]);
            out[0] = u64::from_le_bytes(b);
        } else {
            for (k, o) in out.iter_mut().enumerate().take(nw) {
                *o = u64::from_le_bytes(d[8 * k..8 * k + 8].try_into().unwrap());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::History;

    #[test]
    fn record_and_read() {
        let mut h = History::new(&[1, 40, 100, 0], 1 << 20);
        h.record(0, 0, &[0, 0]);
        h.record(1, 0, &[0x12_3456_789a, 0]);
        h.record(2, 0, &[1, 2, 0, 0]);
        h.record(0, 5, &[0, 1]); // z
        h.prepare(&[0, 2]);
        h.record(2, 7, &[3, 4, 0, 0]);
        h.record(3, 7, &[1, 0]); // not recorded
        assert_eq!(h.steps, vec![0, 5, 7]);
        // indexed up to the prepare call only
        assert_eq!((h.count(0), h.count(1), h.count(2)), (2, 0, 1));
        h.prepare(&[0, 1, 2, 3]);
        assert_eq!((h.count(0), h.count(1), h.count(2), h.count(3)), (2, 1, 2, 0));
        assert_eq!(h.time(2, 1), 7);
        assert_eq!(h.index_at(0, 4), Some(0));
        assert_eq!(h.index_at(0, 5), Some(1));
        assert_eq!(h.index_at(2, 6), Some(0));
        let mut v = [0u64; 2];
        h.value_plane(0, 1, &mut v);
        assert_eq!(v[0], 0); // z: value plane 0
        h.value_plane(1, 0, &mut v);
        assert_eq!(v[0], 0x12_3456_789a);
        h.value_plane(2, 1, &mut v);
        assert_eq!(v, [3, 4]);
        // 6 bytes per 1-bit change, 16 per step
        assert_eq!(h.bytes, 3 * 16 + 2 * 6 + (4 + 16) + 2 * (4 + 32));
    }
}
