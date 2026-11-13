//! Fusion of combinational processes.
//!
//! A pure combinational process A (assign / always @* / always_comb, see `ProcDef::fusible`) is
//! merged into process B when B is the only combinational reader of everything A writes. Chains
//! merge transitively. The fused process runs its members' bodies in rank order and waits on the
//! union of their inputs, so one activation replaces several. Because B is A's only
//! combinational consumer, merging cannot create a cycle in the rank graph. Stores inside the
//! fused body still notify other (non-combinational) readers.
//!
//! Only processes with blocking writes are fused: with non-blocking writes a member's result would
//! only appear in the NBA region and re-trigger the fused process (`x <= 0; x <= v;` would
//! even loop forever).
//!
//! Continuous assignments are evaluated at time 0 but `always @*` blocks are not, so the fused
//! code starts with a time-0 section holding only the assign-style members:
//!
//! ```text
//!   <bodies of members that run at time 0>   ; rank order
//!   Wait ctl
//! L:<bodies of all members>                  ; rank order
//!   Wait ctl
//!   Jmp L
//! ```

use crate::vm::comb_graph;
use crate::*;

const END: u32 = u32::MAX;

/// The body of a fusible process: instructions with jump targets relative to the body start
/// (END = end of the body), and whether the process runs at time 0 (wait at the end).
fn body(code: &[Instr]) -> Option<(Vec<Instr>, bool)> {
    let n = code.len();
    if n < 2 || !matches!(code[n - 1], Instr::Jmp { to: 0 }) {
        return None;
    }
    let (start, end, at_time0) = if matches!(code[n - 2], Instr::Wait { .. }) {
        (0, n - 2, true)
    } else if matches!(code[0], Instr::Wait { .. }) {
        (1, n - 1, false)
    } else {
        return None;
    };
    let rel = |t: u32| -> Option<u32> {
        let t = t as usize;
        if t == end || (!at_time0 && t == 0) {
            Some(END)
        } else if t >= start && t < end {
            Some((t - start) as u32)
        } else {
            None
        }
    };
    let mut out = Vec::with_capacity(end - start);
    for ins in &code[start..end] {
        out.push(match ins {
            Instr::Wait { .. }
            | Instr::Delay { .. }
            | Instr::Halt
            | Instr::Sys(_)
            | Instr::SysF { .. }
            | Instr::Fork { .. }
            | Instr::Disable { .. }
            | Instr::Kick { .. }
            | Instr::Unforce { .. }
            | Instr::Udp { .. } => return None,
            // fused members must see each other's results at once: only blocking writes
            Instr::Store { mode: StoreMode::Nba | StoreMode::NbaDelay(_), .. } => return None,
            Instr::Jmp { to } => Instr::Jmp { to: rel(*to)? },
            Instr::Jz { c, w, to } => Instr::Jz { c: *c, w: *w, to: rel(*to)? },
            other => other.clone(),
        });
    }
    Some((out, at_time0))
}

fn append(code: &mut Vec<Instr>, body: &[Instr]) {
    let base = code.len() as u32;
    let end = base + body.len() as u32;
    let fix = |t: u32| if t == END { end } else { base + t };
    for ins in body {
        code.push(match ins {
            Instr::Jmp { to } => Instr::Jmp { to: fix(*to) },
            Instr::Jz { c, w, to } => Instr::Jz { c: *c, w: *w, to: fix(*to) },
            other => other.clone(),
        });
    }
}

fn find(uf: &mut [u32], x: u32) -> u32 {
    let mut r = x;
    while uf[r as usize] != r {
        r = uf[r as usize];
    }
    let mut x = x;
    while uf[x as usize] != r {
        let nx = uf[x as usize];
        uf[x as usize] = r;
        x = nx;
    }
    r
}

/// Fuse combinational process chains in place. Returns the number of processes removed.
pub fn fuse(prog: &mut Program) -> usize {
    let n = prog.procs.len();
    let (rank, succ, eligible) = comb_graph(prog);
    // the event control each process waits on
    let mut ctl_of = vec![NONE; n];
    for p in 0..n {
        for ins in &prog.procs[p].code {
            if let Instr::Wait { ctl } = ins {
                ctl_of[p] = *ctl;
            }
        }
    }
    let bodies: Vec<Option<(Vec<Instr>, bool)>> = (0..n)
        .map(|p| {
            let ok = eligible[p] && rank[p] != NONE && prog.procs[p].fusible && ctl_of[p] != NONE;
            if ok {
                body(&prog.procs[p].code)
            } else {
                None
            }
        })
        .collect();
    // signals and their writers: A may only be fused if nothing but A writes what A drives
    // (another writer, e.g. a force, would re-trigger the fused process and A would undo it)
    let mut writers: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let mut writes: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (p, pd) in prog.procs.iter().enumerate() {
        for ins in &pd.code {
            if let Instr::Store { t, .. } = ins {
                let sig = match *t {
                    Target::Whole { sig }
                    | Target::Part { sig, .. }
                    | Target::DynPart { sig, .. }
                    | Target::Elem { sig, .. }
                    | Target::ElemK { sig, .. }
                    | Target::ElemPart { sig, .. } => sig,
                };
                let sig = prog.signals[sig as usize].alias_of.map(|a| a.0).unwrap_or(sig);
                let w = writers.entry(sig).or_default();
                if !w.contains(&(p as u32)) {
                    w.push(p as u32);
                }
                writes[p].push(sig);
            }
        }
    }
    let sole_writer = |a: usize| writes[a].iter().all(|s| writers[s].len() == 1);
    let mut uf: Vec<u32> = (0..n as u32).collect();
    for a in 0..n {
        if bodies[a].is_none() || succ[a].len() != 1 || !sole_writer(a) {
            continue;
        }
        let b = succ[a][0] as usize;
        if bodies[b].is_some() {
            let (ra, rb) = (find(&mut uf, a as u32), find(&mut uf, b as u32));
            if ra != rb {
                uf[ra as usize] = rb;
            }
        }
    }
    let mut groups: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for p in 0..n as u32 {
        if bodies[p as usize].is_some() {
            groups.entry(find(&mut uf, p)).or_default().push(p);
        }
    }
    let mut removed = 0;
    for (_, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        members.sort_by_key(|&p| (rank[p as usize], p));
        let rep = members[0] as usize;
        let ctl = ctl_of[rep];
        let mut triggers: Vec<(u32, Edge, u32)> = Vec::new();
        for &m in &members {
            triggers.extend(prog.ctls[ctl_of[m as usize] as usize].triggers.iter().copied());
        }
        triggers.sort_by_key(|t| (t.0, t.2, t.1 as u8));
        triggers.dedup();
        let mut code = Vec::new();
        for &m in &members {
            let (b, at0) = bodies[m as usize].as_ref().unwrap();
            if *at0 {
                append(&mut code, b);
            }
        }
        code.push(Instr::Wait { ctl });
        let top = code.len() as u32;
        for &m in &members {
            append(&mut code, &bodies[m as usize].as_ref().unwrap().0);
        }
        code.push(Instr::Wait { ctl });
        code.push(Instr::Jmp { to: top });
        for &m in &members[1..] {
            let m = m as usize;
            prog.ctls[ctl_of[m] as usize].triggers.clear();
            prog.procs[m].code = vec![Instr::Halt];
            prog.procs[m].fusible = false;
            removed += 1;
        }
        prog.ctls[ctl as usize].triggers = triggers;
        prog.procs[rep].code = code;
    }
    removed
}
