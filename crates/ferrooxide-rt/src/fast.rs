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
    /// a generic jump (wide condition): continues at the pc the generic path returns
    GenJ,
    /// narrow insert: bits [off, off+w) of d = s
    Ins1 { d: u32, s: u32, off: u8, w: u8 },
    // ---- wide ops (65..=MAX_WIDE bits); `w` widths, operands laid out as nw words per plane
    WCopy { d: u32, s: u32, w: u16 },
    WAnd { d: u32, a: u32, b: u32, w: u16 },
    WOr { d: u32, a: u32, b: u32, w: u16 },
    WXor { d: u32, a: u32, b: u32, w: u16 },
    WNot { d: u32, s: u32, w: u16 },
    WAdd { d: u32, a: u32, b: u32, w: u16 },
    WSub { d: u32, a: u32, b: u32, w: u16 },
    WEq { d: u32, a: u32, b: u32, w: u16 },
    WNe { d: u32, a: u32, b: u32, w: u16 },
    WCaseEq { d: u32, a: u32, b: u32, w: u16 },
    WCaseNe { d: u32, a: u32, b: u32, w: u16 },
    WLt { d: u32, a: u32, b: u32, w: u16 },
    WLe { d: u32, a: u32, b: u32, w: u16 },
    /// extract `w` bits at `lsb` of the `sw`-bit value s (in range; either width may be wide)
    WSlice { d: u32, s: u32, lsb: u32, sw: u16, w: u16 },
    WResize { d: u32, s: u32, sw: u16, dw: u16, sign: bool },
    /// insert the `sw`-bit value s at bit `off` of the `dw`-bit value d
    WIns { d: u32, s: u32, off: u32, dw: u16, sw: u16 },
    /// mux with a ≤64-bit condition
    WMux { d: u32, c: u32, t: u32, f: u32, w: u16 },
    WBool { d: u32, s: u32, w: u16 },
    /// blocking store of a wide value into a whole signal (or constant-index array element)
    WStore { sig: u32, s: u32, slot: u32, w: u16 },
    /// non-blocking store of a wide value; the value is read from memory by the runtime
    WNba { sig: u32, s: u32, w: u16 },
    // ---- memories, selects and stores with fast in-range paths (otherwise: generic path)
    /// d = array element ix (element ≤64 bits, elements 2 words apart from `base`)
    ArrRead1 { d: u32, base: u32, ix: u32, depth: u32 },
    /// d = (s >> ix) & mask(w) for a ≤64-bit s when ix + w <= sw
    DynSlice1 { d: u32, s: u32, ix: u32, w: u8, sw: u8 },
    /// logical not of a ≤64-bit value
    LNot1 { d: u32, s: u32 },
    LtS1 { d: u32, a: u32, b: u32, w: u8 },
    LeS1 { d: u32, a: u32, b: u32, w: u8 },
    /// wire resolution of two ≤64-bit drivers
    Resolve1 { d: u32, a: u32, b: u32, w: u8 },
    WResolve { d: u32, a: u32, b: u32, w: u16 },
    /// blocking store of bits [lsb, lsb+w) of a ≤64-bit signal at `slot`
    StorePart1 { sig: u32, s: u32, slot: u32, lsb: u8, w: u8 },
    /// blocking store of a ≤64-bit array element (runtime call)
    StoreElem1 { sig: u32, s: u32, ix: u32 },
    /// blocking store of bits [lsb, lsb+w) (w ≤ 64) of an array element (runtime call)
    StoreElemPart1 { sig: u32, s: u32, ix: u32, lsb: u8, w: u8 },
    /// non-blocking store of a ≤64-bit array element (runtime call)
    NbaElem1 { sig: u32, s: u32, ix: u32 },
    /// blocking store of bits [lsb, lsb+w) (w ≤ 64) of a wide signal of `n` words per plane
    StorePartW { sig: u32, s: u32, slot: u32, lsb: u32, w: u8, n: u8 },
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
        Op::Gen | Op::GenJ | Op::Halt | Op::Jmp { .. } => op,
        Op::Ins1 { d, s: x, off, w } => Op::Ins1 { d: s(d), s: s(x), off, w },
        Op::WCopy { d, s: x, w } => Op::WCopy { d: s(d), s: s(x), w },
        Op::WAnd { d, a, b, w } => Op::WAnd { d: s(d), a: s(a), b: s(b), w },
        Op::WOr { d, a, b, w } => Op::WOr { d: s(d), a: s(a), b: s(b), w },
        Op::WXor { d, a, b, w } => Op::WXor { d: s(d), a: s(a), b: s(b), w },
        Op::WNot { d, s: x, w } => Op::WNot { d: s(d), s: s(x), w },
        Op::WAdd { d, a, b, w } => Op::WAdd { d: s(d), a: s(a), b: s(b), w },
        Op::WSub { d, a, b, w } => Op::WSub { d: s(d), a: s(a), b: s(b), w },
        Op::WEq { d, a, b, w } => Op::WEq { d: s(d), a: s(a), b: s(b), w },
        Op::WNe { d, a, b, w } => Op::WNe { d: s(d), a: s(a), b: s(b), w },
        Op::WCaseEq { d, a, b, w } => Op::WCaseEq { d: s(d), a: s(a), b: s(b), w },
        Op::WCaseNe { d, a, b, w } => Op::WCaseNe { d: s(d), a: s(a), b: s(b), w },
        Op::WLt { d, a, b, w } => Op::WLt { d: s(d), a: s(a), b: s(b), w },
        Op::WLe { d, a, b, w } => Op::WLe { d: s(d), a: s(a), b: s(b), w },
        Op::WSlice { d, s: x, lsb, sw, w } => Op::WSlice { d: s(d), s: s(x), lsb, sw, w },
        Op::WResize { d, s: x, sw, dw, sign } => Op::WResize { d: s(d), s: s(x), sw, dw, sign },
        Op::WIns { d, s: x, off, dw, sw } => Op::WIns { d: s(d), s: s(x), off, dw, sw },
        Op::WMux { d, c, t, f: x, w } => Op::WMux { d: s(d), c: s(c), t: s(t), f: s(x), w },
        Op::WBool { d, s: x, w } => Op::WBool { d: s(d), s: s(x), w },
        Op::WStore { sig, s: x, slot, w } => {
            let (x, slot) = (s(x), s(slot));
            Op::WStore { sig: f(Sig, sig), s: x, slot, w }
        }
        Op::WNba { sig, s: x, w } => {
            let x = s(x);
            Op::WNba { sig: f(Sig, sig), s: x, w }
        }
        Op::ArrRead1 { d, base, ix, depth } => Op::ArrRead1 { d: s(d), base: s(base), ix: s(ix), depth },
        Op::DynSlice1 { d, s: x, ix, w, sw } => Op::DynSlice1 { d: s(d), s: s(x), ix: s(ix), w, sw },
        Op::LNot1 { d, s: x } => Op::LNot1 { d: s(d), s: s(x) },
        Op::LtS1 { d, a, b, w } => Op::LtS1 { d: s(d), a: s(a), b: s(b), w },
        Op::LeS1 { d, a, b, w } => Op::LeS1 { d: s(d), a: s(a), b: s(b), w },
        Op::Resolve1 { d, a, b, w } => Op::Resolve1 { d: s(d), a: s(a), b: s(b), w },
        Op::WResolve { d, a, b, w } => Op::WResolve { d: s(d), a: s(a), b: s(b), w },
        Op::StorePart1 { sig, s: x, slot, lsb, w } => {
            let (x, slot) = (s(x), s(slot));
            Op::StorePart1 { sig: f(Sig, sig), s: x, slot, lsb, w }
        }
        Op::StoreElem1 { sig, s: x, ix } => {
            let (x, ix) = (s(x), s(ix));
            Op::StoreElem1 { sig: f(Sig, sig), s: x, ix }
        }
        Op::StoreElemPart1 { sig, s: x, ix, lsb, w } => {
            let (x, ix) = (s(x), s(ix));
            Op::StoreElemPart1 { sig: f(Sig, sig), s: x, ix, lsb, w }
        }
        Op::NbaElem1 { sig, s: x, ix } => {
            let (x, ix) = (s(x), s(ix));
            Op::NbaElem1 { sig: f(Sig, sig), s: x, ix }
        }
        Op::StorePartW { sig, s: x, slot, lsb, w, n } => {
            let (x, slot) = (s(x), s(slot));
            Op::StorePartW { sig: f(Sig, sig), s: x, slot, lsb, w, n }
        }
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

/// `elem_sig`: (array, element) -> virtual signal notified for writes to that element
pub fn lower(code: &[Instr], sigs: &[SignalInfo], elem_sig: &std::collections::HashMap<(u32, u32), u32>) -> Vec<Op> {
    code.iter().map(|i| lower1(i, sigs, elem_sig)).collect()
}

fn lower1(ins: &Instr, sigs: &[SignalInfo], elem_sig: &std::collections::HashMap<(u32, u32), u32>) -> Op {
    if let Instr::Store { t, .. } = ins {
        if sigs[t_sig(t) as usize].forced {
            return Op::Gen; // the generic store applies the force mask
        }
    }
    let op = lower_narrow(ins, sigs, elem_sig);
    if !matches!(op, Op::Gen) {
        return op;
    }
    lower_wide(ins, sigs, elem_sig)
}

/// widest value the wide ops handle (the JIT unrolls word loops)
pub const MAX_WIDE: u32 = 1024;

fn lower_wide(ins: &Instr, sigs: &[SignalInfo], elem_sig: &std::collections::HashMap<(u32, u32), u32>) -> Op {
    use crate::Instr as I;
    let ok = |w: u32| w >= 1 && w <= MAX_WIDE;
    match *ins {
        I::Jmp { .. } | I::Jz { .. } | I::Disable { .. } | I::CallSub { .. } | I::RetSub { .. } => Op::GenJ,
        I::Insert { d, dw, s, sw, off } if dw <= 64 && off + sw <= dw => Op::Ins1 { d, s, off: off as u8, w: sw as u8 },
        I::Insert { d, dw, s, sw, off } if ok(dw) && ok(sw) && off + sw <= dw => {
            Op::WIns { d, s, off, dw: dw as u16, sw: sw as u16 }
        }
        I::Copy { d, s, w } if ok(w) => Op::WCopy { d, s, w: w as u16 },
        I::Un { op: UnOp::Not, d, s, w } if ok(w) => Op::WNot { d, s, w: w as u16 },
        I::Bin { op, d, a, b, w } if ok(w) => {
            let w = w as u16;
            match op {
                BinOp::And => Op::WAnd { d, a, b, w },
                BinOp::Or => Op::WOr { d, a, b, w },
                BinOp::Xor => Op::WXor { d, a, b, w },
                BinOp::Add => Op::WAdd { d, a, b, w },
                BinOp::Sub => Op::WSub { d, a, b, w },
                _ => Op::Gen,
            }
        }
        I::Cmp { op, d, a, b, w } if ok(w) => {
            let w = w as u16;
            match op {
                CmpOp::Eq => Op::WEq { d, a, b, w },
                CmpOp::Ne => Op::WNe { d, a, b, w },
                CmpOp::CaseEq => Op::WCaseEq { d, a, b, w },
                CmpOp::CaseNe => Op::WCaseNe { d, a, b, w },
                CmpOp::Lt => Op::WLt { d, a, b, w },
                CmpOp::Le => Op::WLe { d, a, b, w },
                CmpOp::Gt => Op::WLt { d, a: b, b: a, w },
                CmpOp::Ge => Op::WLe { d, a: b, b: a, w },
                _ => Op::Gen,
            }
        }
        I::Slice { d, s, sw, lsb, w } if ok(sw) && ok(w) && lsb + w <= sw => {
            Op::WSlice { d, s, lsb, sw: sw as u16, w: w as u16 }
        }
        I::Resize { d, s, sw, dw, sign } if ok(sw) && ok(dw) => Op::WResize { d, s, sw: sw as u16, dw: dw as u16, sign },
        I::Mux { d, c, cw, t, f, w } if cw <= 64 && ok(w) => Op::WMux { d, c, t, f, w: w as u16 },
        I::Red { op: RedOp::Bool | RedOp::Or, d, s, w } if ok(w) => Op::WBool { d, s, w: w as u16 },
        I::Store { t, s, mode } => {
            let (sig, slot, w) = match t {
                Target::Whole { sig } => {
                    let i = &sigs[sig as usize];
                    if i.depth > 0 {
                        return Op::Gen;
                    }
                    (sig, i.slot, i.width)
                }
                Target::ElemK { sig, e } => {
                    let i = &sigs[sig as usize];
                    (elem_sig.get(&(sig, e)).copied().unwrap_or(sig), i.slot + e * words(i.width), i.width)
                }
                _ => return Op::Gen,
            };
            if !ok(w) {
                return Op::Gen;
            }
            match mode {
                StoreMode::Blocking => Op::WStore { sig, s, slot, w: w as u16 },
                // generic NBAs of a constant-index element need the element's own signal
                StoreMode::Nba if matches!(t, Target::Whole { .. }) || sig != t_sig(&t) => {
                    Op::WNba { sig, s, w: w as u16 }
                }
                _ => Op::Gen,
            }
        }
        _ => Op::Gen,
    }
}

fn t_sig(t: &Target) -> u32 {
    match *t {
        Target::Whole { sig }
        | Target::Part { sig, .. }
        | Target::DynPart { sig, .. }
        | Target::Elem { sig, .. }
        | Target::ElemK { sig, .. }
        | Target::ElemPart { sig, .. }
        | Target::ElemDynPart { sig, .. } => sig,
    }
}

fn lower_narrow(ins: &Instr, sigs: &[SignalInfo], elem_sig: &std::collections::HashMap<(u32, u32), u32>) -> Op {
    use crate::Instr as I;
    // a plain index: offset = value (no negation, no offset), at most 64 bits
    let plain = |ix: &Index| !ix.neg && ix.k == 0 && ix.w <= 64;
    match *ins {
        I::ArrRead { d, sig, ix } if plain(&ix) && sigs[sig as usize].width <= 64 => {
            let i = &sigs[sig as usize];
            Op::ArrRead1 { d, base: i.slot, ix: ix.slot, depth: i.depth }
        }
        I::DynSlice { d, s, sw, ix, w } if plain(&ix) && sw <= 64 && w <= sw => {
            Op::DynSlice1 { d, s, ix: ix.slot, w: w as u8, sw: sw as u8 }
        }
        I::Red { op: RedOp::Nor, d, s, w } if w <= 64 => Op::LNot1 { d, s },
        I::Cmp { op: CmpOp::LtS, d, a, b, w } if w <= 64 => Op::LtS1 { d, a, b, w: w as u8 },
        I::Cmp { op: CmpOp::LeS, d, a, b, w } if w <= 64 => Op::LeS1 { d, a, b, w: w as u8 },
        I::Cmp { op: CmpOp::GtS, d, a, b, w } if w <= 64 => Op::LtS1 { d, a: b, b: a, w: w as u8 },
        I::Cmp { op: CmpOp::GeS, d, a, b, w } if w <= 64 => Op::LeS1 { d, a: b, b: a, w: w as u8 },
        I::Bin { op: BinOp::Resolve, d, a, b, w } if w <= 64 => Op::Resolve1 { d, a, b, w: w as u8 },
        I::Bin { op: BinOp::Resolve, d, a, b, w } if w <= MAX_WIDE => Op::WResolve { d, a, b, w: w as u16 },
        I::Store { t: Target::Part { sig, lsb, w }, s, mode: StoreMode::Blocking }
            if sigs[sig as usize].width <= 64 && sigs[sig as usize].depth == 0 =>
        {
            Op::StorePart1 { sig, s, slot: sigs[sig as usize].slot, lsb: lsb as u8, w: w as u8 }
        }
        I::Store { t: Target::Elem { sig, ix }, s, mode: StoreMode::Blocking }
            if plain(&ix) && sigs[sig as usize].width <= 64 =>
        {
            Op::StoreElem1 { sig, s, ix: ix.slot }
        }
        I::Store { t: Target::Elem { sig, ix }, s, mode: StoreMode::Nba } if plain(&ix) && sigs[sig as usize].width <= 64 => {
            Op::NbaElem1 { sig, s, ix: ix.slot }
        }
        I::Store { t: Target::ElemPart { sig, ix, lsb, w }, s, mode: StoreMode::Blocking }
            if plain(&ix) && sigs[sig as usize].width <= 64 =>
        {
            Op::StoreElemPart1 { sig, s, ix: ix.slot, lsb: lsb as u8, w: w as u8 }
        }
        I::Store { t: Target::Part { sig, lsb, w }, s, mode: StoreMode::Blocking }
            if w <= 64 && sigs[sig as usize].depth == 0 && nw(sigs[sig as usize].width) <= 255 =>
        {
            let i = &sigs[sig as usize];
            Op::StorePartW { sig, s, slot: i.slot, lsb, w: w as u8, n: nw(i.width) as u8 }
        }
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
                StoreMode::NbaDelay(_) | StoreMode::Force | StoreMode::Inertial(_) => Op::Gen,
            }
        }
        I::Store { t: Target::ElemK { sig, e }, s, mode } => {
            let info = &sigs[sig as usize];
            if info.width > 64 {
                return Op::Gen;
            }
            let slot = info.slot + 2 * e;
            let sig = elem_sig.get(&(sig, e)).copied().unwrap_or(sig);
            match mode {
                StoreMode::Blocking => Op::Store1 { sig, s, slot },
                StoreMode::Nba => Op::Nba1 { sig, s, slot },
                StoreMode::NbaDelay(_) | StoreMode::Force | StoreMode::Inertial(_) => Op::Gen,
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

/// wire resolution of two drivers (value, unknown): z yields, equal values stay, conflicts are x
#[inline(always)]
pub fn resolve1(va: u64, xa: u64, vb: u64, xb: u64, mask: u64) -> (u64, u64) {
    let za = !va & xa;
    let zb = !vb & xb;
    let take_a = zb & !za;
    let take_b = za & !zb;
    let both = !za & !zb;
    let eq = !(va ^ vb) & !(xa ^ xb);
    let rv = (va & take_a) | (vb & take_b) | (both & ((va & eq) | !eq));
    let rx = (xa & take_a) | (xb & take_b) | (za & zb) | (both & ((xa & eq) | !eq));
    (rv & mask, rx & mask)
}
