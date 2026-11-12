//! ferrooxide runtime: compiled program representation, 4-state ops, scheduler and VM.
//!
//! Value layout: a value of width `w` occupies `2*nw(w)` consecutive words of the flat memory:
//! `nw` words of aval followed by `nw` words of bval (VPI encoding: 00=0, 10=1, 01=Z, 11=X).
//! Bits above `w` in the top word are always zero in both planes.

pub mod fast;
pub mod fmt;
pub mod fuse;
#[cfg(feature = "jit")]
pub mod jit;
pub mod ops;
pub mod vcd;
pub mod vm;

pub use vm::Sim;

pub const NONE: u32 = u32::MAX;

#[inline(always)]
pub fn nw(w: u32) -> usize {
    ((w as usize) + 63) >> 6
}

/// Mask for the top word of a `w`-bit value.
#[inline(always)]
pub fn tmask(w: u32) -> u64 {
    let r = w & 63;
    if r == 0 {
        !0
    } else {
        (1u64 << r) - 1
    }
}

/// Number of memory words for a `w`-bit value (both planes).
#[inline(always)]
pub fn words(w: u32) -> u32 {
    2 * nw(w) as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Any,
    Pos,
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigKind {
    Wire,
    Reg,
    Integer,
}

#[derive(Clone, Debug)]
pub struct SignalInfo {
    pub name: String,
    pub slot: u32,
    pub width: u32,
    pub signed: bool,
    /// number of array elements, 0 for a plain vector
    pub depth: u32,
    /// declared [msb:lsb] for VCD
    pub msb: i64,
    pub lsb: i64,
    /// declared index of the first array element and direction (for $readmem)
    pub arr_lo: i64,
    pub kind: SigKind,
    /// this signal is another name for element `e` of array `a` (a port connected to `a[e]`):
    /// it shares the element's memory and stands for it in sensitivity
    pub alias_of: Option<(u32, u32)>,
    /// a `real` variable: the value word holds f64 bits, the unknown word is 0
    pub real: bool,
}

#[derive(Clone, Debug)]
pub struct EventCtl {
    pub proc: u32,
    /// (signal, edge, array element or NONE for the whole signal)
    pub triggers: Vec<(u32, Edge, u32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcClass {
    /// started at time 0
    Normal,
    /// not started automatically, runs in the postponed region when woken ($monitor/$strobe)
    Postponed,
    /// final block: runs at the end of simulation
    Final,
    /// a fork branch: started by its parent's `Fork`, not at time 0
    Spawned,
}

#[derive(Clone, Debug)]
pub struct ProcDef {
    pub name: String,
    pub code: Vec<Instr>,
    pub class: ProcClass,
    /// for a fork branch: the process that forked it (NONE otherwise)
    pub parent: u32,
    /// a pure combinational process (assign, always @*, always_comb) whose sensitivity is its
    /// full read set: re-evaluating it is harmless, so it may be fused with others
    pub fusible: bool,
}

#[derive(Default, Clone, Debug)]
pub struct Program {
    pub mem: Vec<u64>,
    pub signals: Vec<SignalInfo>,
    pub procs: Vec<ProcDef>,
    pub ctls: Vec<EventCtl>,
    /// all hierarchical names (including port aliases) for VCD: (name, signal)
    pub names: Vec<(String, u32)>,
    /// memory words [0, scratch_words) are the shared scratch region for process temporaries
    pub scratch_words: u32,
    /// disable fusion of combinational processes (debugging)
    pub no_fuse: bool,
    /// simulation precision as a power of ten of seconds (one tick)
    pub precision: i32,
    /// named blocks and task bodies that `disable` can end
    pub blocks: Vec<BlockDef>,
    /// `Disable { target }`: the blocks each target names
    pub disables: Vec<Vec<u32>>,
}

/// A named block (or inlined task body): code range [start, end) of `proc`, and the fork
/// branches started inside it (ended too when the block is disabled).
#[derive(Clone, Debug, Default)]
pub struct BlockDef {
    pub proc: u32,
    pub start: u32,
    pub end: u32,
    pub children: Vec<u32>,
}

/// Every memory slot an instruction reads or writes (value base slots).
pub fn instr_slots(ins: &Instr) -> Vec<u32> {
    let ix = |i: &Index| i.slot;
    match ins {
        Instr::Copy { d, s, .. } | Instr::Resize { d, s, .. } | Instr::Un { d, s, .. } | Instr::Red { d, s, .. } => {
            vec![*d, *s]
        }
        Instr::Clog2 { d, s, .. } => vec![*d, *s],
        Instr::IToR { d, s, .. } | Instr::RToI { d, s, .. } | Instr::RNeg { d, s } | Instr::RToTicks { d, s, .. } => {
            vec![*d, *s]
        }
        Instr::RBin { d, a, b, .. } | Instr::RCmp { d, a, b, .. } => vec![*d, *a, *b],
        Instr::Bin { d, a, b, .. } | Instr::Cmp { d, a, b, .. } => vec![*d, *a, *b],
        Instr::Shift { d, a, b, .. } | Instr::Pow { d, a, b, .. } => vec![*d, *a, *b],
        Instr::Mux { d, c, t, f, .. } => vec![*d, *c, *t, *f],
        Instr::Concat { d, parts, .. } => std::iter::once(*d).chain(parts.iter().map(|p| p.0)).collect(),
        Instr::Insert { d, s, .. } => vec![*d, *s],
        Instr::Slice { d, s, .. } => vec![*d, *s],
        Instr::DynSlice { d, s, ix: i, .. } => vec![*d, *s, ix(i)],
        Instr::ArrRead { d, ix: i, .. } => vec![*d, ix(i)],
        Instr::Store { t, s, mode } => {
            let mut v = vec![*s];
            match t {
                Target::DynPart { ix: i, .. } | Target::Elem { ix: i, .. } | Target::ElemPart { ix: i, .. } => {
                    v.push(ix(i))
                }
                _ => {}
            }
            if let StoreMode::NbaDelay(d) = mode {
                v.push(*d);
            }
            v
        }
        Instr::Jz { c, .. } => vec![*c],
        Instr::Delay { s } => vec![*s],
        Instr::SysF { d, .. } => vec![*d],
        Instr::Jmp { .. } | Instr::Wait { .. } | Instr::Halt | Instr::Fork { .. } | Instr::Disable { .. } => vec![],
        Instr::Sys(t) => {
            let fa = |args: &Vec<FmtArg>| -> Vec<u32> {
                args.iter()
                    .filter_map(|a| match a {
                        FmtArg::Val { s, .. } | FmtArg::Time { s } | FmtArg::Real { s } => Some(*s),
                        _ => None,
                    })
                    .collect()
            };
            match &**t {
                SysTask::Display { args, fd, .. } => {
                    let mut v = fa(args);
                    if let Some((s, _)) = fd {
                        v.push(*s);
                    }
                    v
                }
                SysTask::MonitorPrint { args, .. } => fa(args),
                SysTask::ReadMem { start, end, file, .. } => {
                    let mut v: Vec<u32> = start.iter().chain(end.iter()).map(|x| x.0).collect();
                    if let FileName::Val { s, .. } = file {
                        v.push(*s);
                    }
                    v
                }
                SysTask::FOpen { d, file, .. } => {
                    let mut v = vec![*d];
                    if let FileName::Val { s, .. } = file {
                        v.push(*s);
                    }
                    v
                }
                SysTask::TestPlusargs { d, .. } => vec![*d],
                SysTask::ValuePlusargs { d, val, .. } => vec![*d, *val],
                SysTask::SFormat { d, args, .. } => {
                    let mut v = fa(args);
                    v.push(*d);
                    v
                }
                SysTask::FClose { s } => vec![*s],
                _ => vec![],
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Instructions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedOp {
    And,
    Or,
    Xor,
    Nand,
    Nor,
    Xnor,
    /// truthiness: same as Or
    Bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    And,
    Or,
    Xor,
    Xnor,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    DivS,
    ModS,
    /// wire resolution of two drivers (z yields, conflicts give x)
    Resolve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    CaseEq,
    CaseNe,
    /// casez match (z / ? in either operand is a wildcard)
    ZEq,
    /// casex match (x or z in either operand is a wildcard)
    XEq,
    /// ==? (x/z in rhs are wildcards)
    WildEq,
    WildNe,
    Lt,
    Le,
    Gt,
    Ge,
    LtS,
    LeS,
    GtS,
    GeS,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShOp {
    Shl,
    Shr,
    AShr,
}

/// A runtime index: `offset = (neg ? -v : v) + k` where `v` is the (possibly signed) value in `slot`.
#[derive(Clone, Copy, Debug)]
pub struct Index {
    pub slot: u32,
    pub w: u32,
    pub signed: bool,
    pub neg: bool,
    pub k: i64,
}

#[derive(Clone, Copy, Debug)]
pub enum Target {
    Whole { sig: u32 },
    Part { sig: u32, lsb: u32, w: u32 },
    DynPart { sig: u32, ix: Index, w: u32 },
    Elem { sig: u32, ix: Index },
    /// array element with a constant (in-range) element offset
    ElemK { sig: u32, e: u32 },
    ElemPart { sig: u32, ix: Index, lsb: u32, w: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreMode {
    Blocking,
    Nba,
    /// NBA with intra-assignment delay; delay value is in the slot
    NbaDelay(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SysFn {
    /// $time in units of the calling module: simulation ticks / 10^exp, rounded
    Time(u8),
    /// $realtime: ticks / 10^exp as a real
    RealTime(u8),
    Random,
    /// $random(seed) where the seed variable is a signal
    RandomSeed(u32),
    Urandom,
}

#[derive(Clone, Debug)]
pub enum FmtArg {
    /// string literal: used as a format string if it appears where a format is expected
    Lit(String),
    Val { s: u32, w: u32, signed: bool },
    /// value produced by $time
    Time { s: u32 },
    /// a real value (f64 bits)
    Real { s: u32 },
    Empty,
}

/// A file name: a string literal, or a variable holding the characters.
#[derive(Clone, Debug)]
pub enum FileName {
    Lit(String),
    Val { s: u32, w: u32 },
}

#[derive(Clone, Debug)]
pub enum SysTask {
    /// $display/$write/$strobe/$monitor body printing; `nl` adds newline. `fd` = None → stdout
    /// `texp`: %t scales a time in module units by 10^texp to simulation ticks
    Display { nl: bool, args: Vec<FmtArg>, scope: String, fd: Option<(u32, u32)>, prefix: &'static str, texp: u8 },
    /// mark the given postponed process as the active monitor and wake it
    Monitor { proc: u32 },
    /// only print if this proc is the active monitor
    MonitorPrint { proc: u32, args: Vec<FmtArg>, scope: String, texp: u8 },
    MonitorOn(bool),
    Strobe { proc: u32 },
    /// $finish / $stop; `loc` is "file:line" for the message (empty: no message)
    Finish { loc: String, stop: bool },
    DumpFile(String),
    DumpVars,
    DumpOn(bool),
    ReadMem { hex: bool, file: FileName, sig: u32, start: Option<(u32, u32)>, end: Option<(u32, u32)>, loc: String },
    /// `mode`: "r", "w", "a" (+ "b"/"+"); without a mode the file is opened for writing
    FOpen { d: u32, dw: u32, file: FileName, mode: String },
    FClose { s: u32 },
    /// d (1 bit) = some plusarg starts with `prefix`
    TestPlusargs { d: u32, prefix: String },
    /// d (1 bit) = a plusarg `<prefix><value>` exists; its value, converted by `conv`
    /// (d h o b s f e g), goes to `val` (`w` bits, or f64 bits if `real`)
    ValuePlusargs { d: u32, prefix: String, conv: char, val: u32, w: u32, real: bool },
    /// format into the `w`-bit value at `d` (characters right-aligned)
    SFormat { d: u32, w: u32, args: Vec<FmtArg>, scope: String, texp: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Clone, Debug)]
pub enum Instr {
    Copy { d: u32, s: u32, w: u32 },
    Resize { d: u32, s: u32, sw: u32, dw: u32, sign: bool },
    Un { op: UnOp, d: u32, s: u32, w: u32 },
    Red { op: RedOp, d: u32, s: u32, w: u32 },
    Bin { op: BinOp, d: u32, a: u32, b: u32, w: u32 },
    Cmp { op: CmpOp, d: u32, a: u32, b: u32, w: u32 },
    Shift { op: ShOp, d: u32, a: u32, w: u32, b: u32, bw: u32 },
    Pow { d: u32, a: u32, b: u32, w: u32, bw: u32, signed: bool },
    Mux { d: u32, c: u32, cw: u32, t: u32, f: u32, w: u32 },
    Concat { d: u32, w: u32, parts: Box<[(u32, u32)]> },
    /// deposit the `sw`-bit value `s` at bit `off` of the `dw`-bit value `d` (other bits kept)
    Insert { d: u32, dw: u32, s: u32, sw: u32, off: u32 },
    Slice { d: u32, s: u32, sw: u32, lsb: u32, w: u32 },
    DynSlice { d: u32, s: u32, sw: u32, ix: Index, w: u32 },
    ArrRead { d: u32, sig: u32, ix: Index },
    Store { t: Target, s: u32, mode: StoreMode },
    Jmp { to: u32 },
    /// jump if the value is not true (zero, x or z)
    Jz { c: u32, w: u32, to: u32 },
    Delay { s: u32 },
    Wait { ctl: u32 },
    /// start another (postponed) process
    Halt,
    SysF { f: SysFn, d: u32 },
    Clog2 { d: u32, s: u32, w: u32 },
    /// start the fork branches; join: 0 = join (all), 1 = join_any, 2 = join_none
    Fork { children: Box<[u32]>, join: u8 },
    /// disable the named blocks / tasks of `Program::disables[target]`
    Disable { target: u32 },
    /// integer (w bits) -> real; x/z bits count as 0
    IToR { d: u32, s: u32, w: u32, signed: bool },
    /// real -> integer of w bits (rounded half away from zero, or truncated toward zero)
    RToI { d: u32, s: u32, w: u32, trunc: bool },
    RBin { op: RealOp, d: u32, a: u32, b: u32 },
    RNeg { d: u32, s: u32 },
    /// real comparison (Eq, Ne, Lt, Le, Gt, Ge), 1-bit result
    RCmp { op: CmpOp, d: u32, a: u32, b: u32 },
    /// a delay in module units (real) -> simulation ticks (64-bit integer):
    /// round(v * 10^exp) (the module's precision) * 10^post
    RToTicks { d: u32, s: u32, exp: u8, post: u8 },
    Sys(Box<SysTask>),
}
