//! Lowering of bytecode to specialised single-word ops.
//!
//! Every `Instr` lowers to exactly one `Op` (so jump targets are unchanged). Instructions whose
//! operands all fit in one word (≤64 bits) get a specialised op that the VM executes with the
//! 2-state fast path inline; everything else becomes `Op::Gen`, which executes the original
//! instruction through the generic path.

use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// execute the original instruction at this pc
    Gen,
    Copy1 { d: u32, s: u32 },
    /// d = (s >> sh) & mask
    Slice1 { d: u32, s: u32, sh: u8, w: u8 },
    /// zero-extend / truncate
    Trunc1 { d: u32, s: u32, w: u8 },
    /// sign-extend from `sw` bits, then mask
    Sext1 { d: u32, s: u32, sw: u8, w: u8 },
    Not1 { d: u32, s: u32, w: u8 },
    And1 { d: u32, a: u32, b: u32 },
    Or1 { d: u32, a: u32, b: u32 },
    Xor1 { d: u32, a: u32, b: u32 },
    Xnor1 { d: u32, a: u32, b: u32, w: u8 },
    Add1 { d: u32, a: u32, b: u32, w: u8 },
    Sub1 { d: u32, a: u32, b: u32, w: u8 },
    Mul1 { d: u32, a: u32, b: u32, w: u8 },
    Eq1 { d: u32, a: u32, b: u32 },
    Ne1 { d: u32, a: u32, b: u32 },
    CaseEq1 { d: u32, a: u32, b: u32 },
    CaseNe1 { d: u32, a: u32, b: u32 },
    Lt1 { d: u32, a: u32, b: u32 },
    Le1 { d: u32, a: u32, b: u32 },
    Shl1 { d: u32, a: u32, b: u32, w: u8 },
    Shr1 { d: u32, a: u32, b: u32, w: u8 },
    /// two-part concatenation: d = hi << sh | lo
    Cat2 { d: u32, hi: u32, lo: u32, sh: u8 },
    Mux1 { d: u32, c: u32, t: u32, f: u32 },
    Bool1 { d: u32, s: u32 },
    RedAnd1 { d: u32, s: u32, w: u8 },
    RedXor1 { d: u32, s: u32 },
    Jmp { to: u32 },
    Jz1 { c: u32, to: u32 },
    /// blocking whole-signal store of a ≤64-bit value; `slot` is the signal's slot
    Store1 { sig: u32, s: u32, slot: u32 },
    /// non-blocking whole-signal store of a ≤64-bit value
    Nba1 { sig: u32, s: u32, slot: u32 },
    Wait { ctl: u32 },
    /// delay by the (≤64-bit) value in slot s
    Delay { s: u32 },
    Halt,
}

/// mask of the low `w` bits, 1 ≤ w ≤ 64
#[inline(always)]
pub fn mk(w: u8) -> u64 {
    u64::MAX >> (64 - w as u32)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Slot,
    Sig,
    Ctl,
}

/// Rewrite every slot / signal / event-control field of an op through `f`.
pub fn map_op(op: Op, f: &mut dyn FnMut(Field, u32) -> u32) -> Op {
    use Field::*;
    let mut s = |x: u32| f(Slot, x);
    match op {
        Op::Gen | Op::Halt | Op::Jmp { .. } => op,
        Op::Copy1 { d, s: x } => Op::Copy1 { d: s(d), s: s(x) },
        Op::Slice1 { d, s: x, sh, w } => Op::Slice1 { d: s(d), s: s(x), sh, w },
        Op::Trunc1 { d, s: x, w } => Op::Trunc1 { d: s(d), s: s(x), w },
        Op::Sext1 { d, s: x, sw, w } => Op::Sext1 { d: s(d), s: s(x), sw, w },
        Op::Not1 { d, s: x, w } => Op::Not1 { d: s(d), s: s(x), w },
        Op::And1 { d, a, b } => Op::And1 { d: s(d), a: s(a), b: s(b) },
        Op::Or1 { d, a, b } => Op::Or1 { d: s(d), a: s(a), b: s(b) },
        Op::Xor1 { d, a, b } => Op::Xor1 { d: s(d), a: s(a), b: s(b) },
        Op::Xnor1 { d, a, b, w } => Op::Xnor1 { d: s(d), a: s(a), b: s(b), w },
        Op::Add1 { d, a, b, w } => Op::Add1 { d: s(d), a: s(a), b: s(b), w },
        Op::Sub1 { d, a, b, w } => Op::Sub1 { d: s(d), a: s(a), b: s(b), w },
        Op::Mul1 { d, a, b, w } => Op::Mul1 { d: s(d), a: s(a), b: s(b), w },
        Op::Eq1 { d, a, b } => Op::Eq1 { d: s(d), a: s(a), b: s(b) },
        Op::Ne1 { d, a, b } => Op::Ne1 { d: s(d), a: s(a), b: s(b) },
        Op::CaseEq1 { d, a, b } => Op::CaseEq1 { d: s(d), a: s(a), b: s(b) },
        Op::CaseNe1 { d, a, b } => Op::CaseNe1 { d: s(d), a: s(a), b: s(b) },
        Op::Lt1 { d, a, b } => Op::Lt1 { d: s(d), a: s(a), b: s(b) },
        Op::Le1 { d, a, b } => Op::Le1 { d: s(d), a: s(a), b: s(b) },
        Op::Shl1 { d, a, b, w } => Op::Shl1 { d: s(d), a: s(a), b: s(b), w },
        Op::Shr1 { d, a, b, w } => Op::Shr1 { d: s(d), a: s(a), b: s(b), w },
        Op::Cat2 { d, hi, lo, sh } => Op::Cat2 { d: s(d), hi: s(hi), lo: s(lo), sh },
        Op::Mux1 { d, c, t, f: x } => Op::Mux1 { d: s(d), c: s(c), t: s(t), f: s(x) },
        Op::Bool1 { d, s: x } => Op::Bool1 { d: s(d), s: s(x) },
        Op::RedAnd1 { d, s: x, w } => Op::RedAnd1 { d: s(d), s: s(x), w },
        Op::RedXor1 { d, s: x } => Op::RedXor1 { d: s(d), s: s(x) },
        Op::Jz1 { c, to } => Op::Jz1 { c: s(c), to },
        Op::Delay { s: x } => Op::Delay { s: s(x) },
        Op::Store1 { sig, s: x, slot } => {
            let (x, slot) = (s(x), s(slot));
            Op::Store1 { sig: f(Sig, sig), s: x, slot }
        }
        Op::Nba1 { sig, s: x, slot } => {
            let (x, slot) = (s(x), s(slot));
            Op::Nba1 { sig: f(Sig, sig), s: x, slot }
        }
        Op::Wait { ctl } => Op::Wait { ctl: f(Ctl, ctl) },
    }
}

pub fn lower(code: &[Instr], sigs: &[SignalInfo]) -> Vec<Op> {
    code.iter().map(|i| lower1(i, sigs)).collect()
}

fn lower1(ins: &Instr, sigs: &[SignalInfo]) -> Op {
    use crate::Instr as I;
    match *ins {
        I::Copy { d, s, w } if w <= 64 => Op::Copy1 { d, s },
        I::Slice { d, s, sw, lsb, w } if sw <= 64 && lsb + w <= sw => Op::Slice1 { d, s, sh: lsb as u8, w: w as u8 },
        I::Resize { d, s, sw, dw, sign } if sw <= 64 && dw <= 64 => {
            if sign && sw < dw {
                Op::Sext1 { d, s, sw: sw as u8, w: dw as u8 }
            } else {
                Op::Trunc1 { d, s, w: dw as u8 }
            }
        }
        I::Un { op: UnOp::Not, d, s, w } if w <= 64 => Op::Not1 { d, s, w: w as u8 },
        I::Bin { op, d, a, b, w } if w <= 64 => match op {
            BinOp::And => Op::And1 { d, a, b },
            BinOp::Or => Op::Or1 { d, a, b },
            BinOp::Xor => Op::Xor1 { d, a, b },
            BinOp::Xnor => Op::Xnor1 { d, a, b, w: w as u8 },
            BinOp::Add => Op::Add1 { d, a, b, w: w as u8 },
            BinOp::Sub => Op::Sub1 { d, a, b, w: w as u8 },
            BinOp::Mul => Op::Mul1 { d, a, b, w: w as u8 },
            _ => Op::Gen,
        },
        I::Cmp { op, d, a, b, w } if w <= 64 => match op {
            CmpOp::Eq => Op::Eq1 { d, a, b },
            CmpOp::Ne => Op::Ne1 { d, a, b },
            CmpOp::CaseEq => Op::CaseEq1 { d, a, b },
            CmpOp::CaseNe => Op::CaseNe1 { d, a, b },
            CmpOp::Lt => Op::Lt1 { d, a, b },
            CmpOp::Le => Op::Le1 { d, a, b },
            CmpOp::Gt => Op::Lt1 { d, a: b, b: a },
            CmpOp::Ge => Op::Le1 { d, a: b, b: a },
            _ => Op::Gen,
        },
        I::Shift { op, d, a, w, b, bw } if w <= 64 && bw <= 64 => match op {
            ShOp::Shl => Op::Shl1 { d, a, b, w: w as u8 },
            ShOp::Shr => Op::Shr1 { d, a, b, w: w as u8 },
            ShOp::AShr => Op::Gen,
        },
        I::Concat { d, w, ref parts } if w <= 64 && parts.len() == 2 => {
            Op::Cat2 { d, hi: parts[0].0, lo: parts[1].0, sh: parts[1].1 as u8 }
        }
        I::Mux { d, c, cw, t, f, w } if cw <= 64 && w <= 64 => Op::Mux1 { d, c, t, f },
        I::Red { op: RedOp::Bool | RedOp::Or, d, s, w } if w <= 64 => Op::Bool1 { d, s },
        I::Red { op: RedOp::And, d, s, w } if w <= 64 => Op::RedAnd1 { d, s, w: w as u8 },
        I::Red { op: RedOp::Xor, d, s, w } if w <= 64 => Op::RedXor1 { d, s },
        I::Jmp { to } => Op::Jmp { to },
        I::Wait { ctl } => Op::Wait { ctl },
        I::Delay { s } => Op::Delay { s },
        I::Halt => Op::Halt,
        I::Jz { c, w, to } if w <= 64 => Op::Jz1 { c, to },
        I::Store { t: Target::Whole { sig }, s, mode } => {
            let info = &sigs[sig as usize];
            if info.width > 64 || info.depth > 0 {
                return Op::Gen;
            }
            match mode {
                StoreMode::Blocking => Op::Store1 { sig, s, slot: info.slot },
                StoreMode::Nba => Op::Nba1 { sig, s, slot: info.slot },
                StoreMode::NbaDelay(_) => Op::Gen,
            }
        }
        I::Store { t: Target::ElemK { sig, e }, s, mode } => {
            let info = &sigs[sig as usize];
            if info.width > 64 {
                return Op::Gen;
            }
            let slot = info.slot + 2 * e;
            match mode {
                StoreMode::Blocking => Op::Store1 { sig, s, slot },
                StoreMode::Nba => Op::Nba1 { sig, s, slot },
                StoreMode::NbaDelay(_) => Op::Gen,
            }
        }
        _ => Op::Gen,
    }
}

// ---------------------------------------------------------------------------------------------
// single-word 4-state kernels: (value, unknown) pairs

#[inline(always)]
pub fn and1(va: u64, xa: u64, vb: u64, xb: u64) -> (u64, u64) {
    if (xa | xb) == 0 {
        return (va & vb, 0);
    }
    let k0 = (!va & !xa) | (!vb & !xb);
    let k1 = (va & !xa) & (vb & !xb);
    // bits that are neither known 0 nor known 1 are x (only possible where an input is unknown)
    let rx = (xa | xb) & !k0 & !k1;
    (k1 | rx, rx)
}

#[inline(always)]
pub fn or1(va: u64, xa: u64, vb: u64, xb: u64) -> (u64, u64) {
    if (xa | xb) == 0 {
        return (va | vb, 0);
    }
    let k1 = (va & !xa) | (vb & !xb);
    let k0 = (!va & !xa) & (!vb & !xb);
    let rx = (xa | xb) & !k0 & !k1;
    (k1 | rx, rx)
}

#[inline(always)]
pub fn sext(v: u64, w: u32) -> u64 {
    if w >= 64 {
        v
    } else {
        let sh = 64 - w;
        (((v << sh) as i64) >> sh) as u64
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        eprintln!("Op {} Instr {}", std::mem::size_of::<super::Op>(), std::mem::size_of::<crate::Instr>());
    }
}
