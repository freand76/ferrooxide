//! Value history of all non-array signals (for the debug server's interval queries).
//! Fed from the same per-time-step dirty list as the VCD writer: at the end of each time step
//! the changed signals' values are appended to per-signal change logs.

pub struct History {
    /// per signal: the times of its changes (ascending, one entry per time step at most)
    pub times: Vec<Vec<u64>>,
    /// per signal: value and unknown planes after each change (`words[sig]` words each)
    pub vals: Vec<Vec<u64>>,
    /// per signal: words per entry (2 * value words; 0: not recorded)
    pub words: Vec<u32>,
    /// bytes used by the logs
    pub bytes: usize,
    /// recording stops when `bytes` reaches this
    pub budget: usize,
    /// the budget was reached at this time (later changes are not recorded)
    pub full_at: Option<u64>,
}

impl History {
    pub fn new(words: Vec<u32>, budget: usize) -> History {
        let n = words.len();
        History { times: vec![Vec::new(); n], vals: vec![Vec::new(); n], words, bytes: 0, budget, full_at: None }
    }

    /// Record `sig`'s planes at time `t` (no entry when the value did not change; a second
    /// change in the same time step replaces the first).
    pub fn record(&mut self, sig: u32, t: u64, planes: &[u64]) {
        let s = sig as usize;
        let w = self.words[s] as usize;
        if w == 0 || self.full_at.is_some() {
            return;
        }
        let (times, vals) = (&mut self.times[s], &mut self.vals[s]);
        if let Some(&lt) = times.last() {
            let last = &vals[vals.len() - w..];
            if last == &planes[..w] {
                return;
            }
            if lt == t {
                let n = vals.len();
                vals[n - w..].copy_from_slice(&planes[..w]);
                return;
            }
        }
        if self.bytes >= self.budget {
            self.full_at = Some(t);
            return;
        }
        times.push(t);
        vals.extend_from_slice(&planes[..w]);
        self.bytes += 8 + 8 * w;
    }

    /// Index of the last change of `sig` at or before `t`.
    pub fn index_at(&self, sig: u32, t: u64) -> Option<usize> {
        let n = self.times[sig as usize].partition_point(|&x| x <= t);
        n.checked_sub(1)
    }

    /// The planes of change `i` of `sig`.
    pub fn value(&self, sig: u32, i: usize) -> &[u64] {
        let w = self.words[sig as usize] as usize;
        &self.vals[sig as usize][i * w..(i + 1) * w]
    }
}
