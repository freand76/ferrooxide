//! ferrooxide runtime: compiled program representation, 4-state ops, scheduler and VM.
//!
//! Value layout: a value of width `w` occupies `2*nw(w)` consecutive words of the flat memory:
//! `nw` words of aval followed by `nw` words of bval (VPI encoding: 00=0, 10=1, 01=Z, 11=X).
//! Bits above `w` in the top word are always zero in both planes.

pub mod fmt;
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
}

#[derive(Clone, Debug)]
pub struct EventCtl {
    pub proc: u32,
    pub triggers: Vec<(u32, Edge)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcClass {
    /// started at time 0
    Normal,
    /// not started automatically, runs in the postponed region when woken ($monitor/$strobe)
    Postponed,
    /// final block: runs at the end of simulation
    Final,
}

#[derive(Clone, Debug)]
pub struct ProcDef {
    pub name: String,
    pub code: Vec<Instr>,
    pub class: ProcClass,
}

#[derive(Default, Clone, Debug)]
pub struct Program {
    pub mem: Vec<u64>,
    pub signals: Vec<SignalInfo>,
    pub procs: Vec<ProcDef>,
    pub ctls: Vec<EventCtl>,
    /// all hierarchical names (including port aliases) for VCD: (name, signal)
    pub names: Vec<(String, u32)>,
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
    Time,
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
    Empty,
}

#[derive(Clone, Debug)]
pub enum SysTask {
    /// $display/$write/$strobe/$monitor body printing; `nl` adds newline. `fd` = None → stdout
    Display { nl: bool, args: Vec<FmtArg>, scope: String, fd: Option<(u32, u32)>, prefix: &'static str },
    /// mark the given postponed process as the active monitor and wake it
    Monitor { proc: u32 },
    /// only print if this proc is the active monitor
    MonitorPrint { proc: u32, args: Vec<FmtArg>, scope: String },
    MonitorOn(bool),
    Strobe { proc: u32 },
    Finish,
    DumpFile(String),
    DumpVars,
    DumpOn(bool),
    ReadMem { hex: bool, file: String, sig: u32, start: Option<(u32, u32)>, end: Option<(u32, u32)> },
    FOpen { d: u32, dw: u32, file: String },
    FClose { s: u32 },
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
    Sys(Box<SysTask>),
}
