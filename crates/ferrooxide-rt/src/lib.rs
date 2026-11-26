//! ferrooxide runtime: compiled program representation, 4-state ops, scheduler and VM.
//!
//! Value layout: a value of width `w` occupies `2*nw(w)` consecutive words of the flat memory:
//! `nw` words of aval followed by `nw` words of bval (VPI encoding: 00=0, 10=1, 01=Z, 11=X).
//! Bits above `w` in the top word are always zero in both planes.

pub mod assert;
pub mod fast;
pub mod fmt;
pub mod fuse;
pub mod history;
#[cfg(feature = "jit")]
pub mod jit;
pub mod ops;
pub mod vcd;
pub mod vm;

pub use vm::{RunState, Sim};

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
    /// the target of a force or procedural assign: stores take the generic path (force mask)
    pub forced: bool,
    /// a `string` variable: 64 bits holding a string handle (`Program::strings`)
    pub string: bool,
    /// a named event (a 1-bit signal toggled by each trigger)
    pub event: bool,
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
    /// woken once at time 0 after every process has started (always_comb, always_latch)
    pub kick0: bool,
    /// for a fork branch: its private memory (slot, words): temporaries that live across
    /// waits and the frames of automatic subroutines it calls. Several instances of the
    /// branch running at once take turns holding it.
    pub frame: Vec<(u32, u32)>,
    /// the frame holds automatic variables of the fork block: their values when the branch
    /// is started (set by the forking process) are its instance's own
    pub capture: bool,
    /// fused combinational processes (`fuse.rs`): a time-0 evaluation, then the loop; both
    /// end in a wait on the same control
    pub fused: bool,
}

#[derive(Default, Clone, Debug)]
pub struct Program {
    pub mem: Vec<u64>,
    pub signals: Vec<SignalInfo>,
    pub procs: Vec<ProcDef>,
    pub ctls: Vec<EventCtl>,
    /// all hierarchical names (including port aliases) for VCD: (name, signal)
    pub names: Vec<(String, u32)>,
    /// the order in which the processes start at time 0 (empty: by index)
    pub start_order: Vec<u32>,
    /// strings known at compile time; a handle is an index (0: the empty string)
    pub strings: Vec<Vec<u8>>,
    /// source locations ("file:line") of the instructions that can warn at run time
    pub locs: Vec<String>,
    /// packages with variables (not dumped; `$dumpvars` warns about them)
    pub packages: Vec<String>,
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
    /// user-defined primitive tables (Instr::Udp)
    pub udps: Vec<UdpTable>,
    /// classes (`Instr::Obj`): the words of a new object (its properties' start values)
    pub classes: Vec<ClassDef>,
    /// a signal every write of an object property touches (for `wait` on conditions that
    /// read objects), if some code waits on it
    pub obj_sig: Option<u32>,
    /// concurrent assertions (`Instr::Assert`)
    pub asserts: Vec<assert::AssertDef>,
    /// sampled values (IEEE 1800-2017 16.5.1): (from, to, words) copied before each time
    /// step, so that `to` holds the values of `from` from before the step (Preponed region)
    pub samples: Vec<(u32, u32, u32)>,
}

/// A class: an object holds its properties flat, `nw(width)` value words then as many
/// unknown words (as a vector of `width` bits); reals and strings take 64 bits each.
#[derive(Clone, Debug, Default)]
pub struct ClassDef {
    pub name: String,
    pub width: u32,
    /// the words of a new object
    pub init: Vec<u64>,
}

/// Value masks of UDP table symbols: bit 0 = 0, bit 1 = 1, bit 2 = x (z reads as x).
pub const UDP_0: u8 = 1;
pub const UDP_1: u8 = 2;
pub const UDP_X: u8 = 4;

/// One input entry of a UDP row: a level (`prev` unused) or an edge (prev -> cur, a change).
#[derive(Clone, Copy, Debug)]
pub struct UdpSym {
    pub edge: bool,
    pub prev: u8,
    pub cur: u8,
}

#[derive(Clone, Debug)]
pub struct UdpRow {
    pub ins: Vec<UdpSym>,
    /// current-state mask (sequential), 7 = any
    pub state: u8,
    /// 0, 1, 2 = x, 3 = no change ('-')
    pub out: u8,
    pub has_edge: bool,
}

#[derive(Clone, Debug, Default)]
pub struct UdpTable {
    pub sequential: bool,
    pub rows: Vec<UdpRow>,
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

/// Calls `f` on every signal id an instruction refers to.
pub fn instr_sigs_mut(ins: &mut Instr, f: &mut impl FnMut(&mut u32)) {
    fn target(t: &mut Target, f: &mut impl FnMut(&mut u32)) {
        match t {
            Target::Whole { sig }
            | Target::Part { sig, .. }
            | Target::DynPart { sig, .. }
            | Target::Elem { sig, .. }
            | Target::ElemK { sig, .. }
            | Target::ElemPart { sig, .. }
            | Target::ElemDynPart { sig, .. } => f(sig),
        }
    }
    match ins {
        Instr::Store { t, .. } | Instr::Unforce { t } => target(t, f),
        Instr::ArrRead { sig, .. } | Instr::Kick { sig } | Instr::Touch { sig } => f(sig),
        Instr::SysF { f: SysFn::RandomSeed(sig) | SysFn::UrandomSeed(sig) | SysFn::Dist { seed: sig, .. }, .. } => f(sig),
        Instr::Sys(t) => match &mut **t {
            SysTask::ReadMem { sig, .. } | SysTask::WriteMem { sig, .. } => f(sig),
            _ => {}
        },
        _ => {}
    }
}

/// Every memory slot an instruction reads or writes (value base slots).
pub fn instr_slots(ins: &Instr) -> Vec<u32> {
    let ix = |i: &Index| i.slot;
    match ins {
        Instr::Copy { d, s, .. } | Instr::Resize { d, s, .. } | Instr::Un { d, s, .. } | Instr::Red { d, s, .. } => {
            vec![*d, *s]
        }
        Instr::Clog2 { d, s, .. } | Instr::Ones { d, s, .. } => vec![*d, *s],
        Instr::CountBits { d, s, ctl, .. } => [*d, *s].into_iter().chain(ctl.iter().copied()).collect(),
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
                Target::ElemDynPart { ix: i, bix, .. } => {
                    v.push(ix(i));
                    v.push(ix(bix));
                }
                _ => {}
            }
            if let StoreMode::NbaDelay(d) | StoreMode::Inertial(d) = mode {
                v.push(*d);
            }
            v
        }
        Instr::Jz { c, .. } => vec![*c],
        Instr::Delay { s } => vec![*s],
        Instr::SysF { d, f: SysFn::UrandomRange(a, b) } => vec![*d, *a, *b],
        Instr::SysF { d, f: SysFn::RandRange { lo, hi, .. } } => vec![*d, *lo, *hi],
        Instr::SysF { d, f: SysFn::Dist { a, b, .. } } => [*d, *a, *b].into_iter().filter(|&x| x != NONE).collect(),
        Instr::CallSub { dsts, .. } => dsts.to_vec(),
        Instr::RetSub { srcs } => srcs.iter().map(|x| x.0).collect(),
        Instr::SaveFrame { .. } => vec![],
        Instr::ResolveStr { d, drv, .. } => std::iter::once(*d).chain(drv.iter().map(|x| x.0)).collect(),
        Instr::SysF { d, .. } => vec![*d],
        Instr::Jmp { .. }
        | Instr::Wait { .. }
        | Instr::Halt
        | Instr::Fork { .. }
        | Instr::WaitFork
        | Instr::Evict { .. }
        | Instr::Disable { .. }
        | Instr::Kick { .. }
        | Instr::Touch { .. } => vec![],
        Instr::Udp { ins, prev, q, .. } => ins.iter().chain(prev.iter()).copied().chain(std::iter::once(*q)).collect(),
        Instr::Str { d, a, b, c, .. } | Instr::Heap { d, a, b, c, .. } | Instr::Assoc { d, a, b, c, .. } => {
            [*d, *a, *b, *c].into_iter().filter(|&x| x != NONE).collect()
        }
        Instr::Obj { d, a, b, .. } => [*d, *a, *b].into_iter().filter(|&x| x != NONE).collect(),
        Instr::Proc { d, a, .. } => [*d, *a].into_iter().filter(|&x| x != NONE).collect(),
        Instr::Assert(AssertOp::Tick { leaves, pass, fail, .. }) => vec![*leaves, *pass, *fail],
        Instr::Assert(_) => vec![],
        Instr::Unforce { t } => match t {
            Target::DynPart { ix, .. } | Target::Elem { ix, .. } | Target::ElemPart { ix, .. } => vec![ix.slot],
            Target::ElemDynPart { ix, bix, .. } => vec![ix.slot, bix.slot],
            _ => vec![],
        },
        Instr::Sys(t) => {
            let fa = |args: &Vec<FmtArg>| -> Vec<u32> {
                args.iter()
                    .filter_map(|a| match a {
                        FmtArg::Val { s, .. } | FmtArg::Str { s, .. } | FmtArg::Time { s } | FmtArg::Real { s, .. } | FmtArg::SVar { s } | FmtArg::Agg { s, .. } => Some(*s),
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
                SysTask::ReadMem { start, end, file, .. } | SysTask::WriteMem { start, end, file, .. } => {
                    let mut v: Vec<u32> = start.iter().chain(end.iter()).map(|x| x.0).collect();
                    if let FileName::Val { s, .. } | FileName::SVar { s } = file {
                        v.push(*s);
                    }
                    v
                }
                SysTask::FOpen { d, file, .. } => {
                    let mut v = vec![*d];
                    if let FileName::Val { s, .. } | FileName::SVar { s } = file {
                        v.push(*s);
                    }
                    v
                }
                SysTask::TestPlusargs { d, prefix: f } | SysTask::ValuePlusargs { d, fmt: f, .. } => {
                    let mut v = vec![*d];
                    if let SysTask::ValuePlusargs { val, .. } = &**t {
                        v.push(*val);
                    }
                    if let FileName::Val { s, .. } | FileName::SVar { s } = f {
                        v.push(*s);
                    }
                    v
                }
                SysTask::SFormat { d, args, .. } => {
                    let mut v = fa(args);
                    v.push(*d);
                    v
                }
                SysTask::FClose { s, .. } => vec![*s],
                SysTask::FileRead { kind, d, src, .. } => {
                    let mut v = vec![*d, src.0];
                    match kind {
                        FileRead::Ungetc { c } => v.push(*c),
                        FileRead::Gets { val, .. } => v.push(*val),
                        FileRead::Seek { off, op } => v.extend([*off, *op]),
                        FileRead::Scan { outs, fmt, .. } => {
                            for o in outs {
                                v.extend([o.val, o.ok]);
                            }
                            if let FileName::Val { s, .. } | FileName::SVar { s } = fmt {
                                v.push(*s);
                            }
                        }
                        _ => {}
                    }
                    v
                }
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
    /// to a 2-state value: x and z bits become 0 (stores into bit/int/byte/... variables)
    To2,
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
    /// wired-AND / wired-OR resolution (wand, wor): z yields, then 0 (AND) or 1 (OR) dominates
    ResolveAnd,
    ResolveOr,
    /// z bits of a take the value of b (tri0/tri1/pullup/pulldown nets)
    PullZ,
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
    /// `mem[i][b +: w]`: bits at a run-time offset `bix` of element `ix`
    ElemDynPart { sig: u32, ix: Index, bix: Index, w: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreMode {
    Blocking,
    Nba,
    /// NBA with intra-assignment delay; delay value is in the slot
    NbaDelay(u32),
    /// force / procedural assign: writes the bits and marks them forced (other stores keep them)
    Force,
    /// a force's process keeping the forced value up to date: writes only the bits that are
    /// still forced (a part may have been released)
    Reforce,
    /// continuous assignment / gate output with an (inertial) delay: the store happens after the
    /// delay unless a newer value for the same target replaces it first. The slot holds a
    /// 192-bit value whose words are the rise, fall and turn-off delays in ticks.
    Inertial(u32),
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
    /// $urandom(seed) where the seed variable is a signal
    UrandomSeed(u32),
    /// $urandom_range(max, min): slots of the two 32-bit bounds
    UrandomRange(u32, u32),
    /// a random `w`-bit value between the 64-bit bounds in slots lo and hi (`signed`
    /// bounds; lo > hi gives lo; more than 64 bits: all bits random) for `randomize`
    RandRange { lo: u32, hi: u32, w: u32, signed: bool },
    /// `$dist_*(seed, a[, b])` (IEEE 1800-2017 20.15): the seed variable is a signal, `a` and
    /// `b` slots of 32-bit signed arguments (`b` NONE when the function has one)
    Dist { kind: Dist, seed: u32, a: u32, b: u32 },
}

/// The distribution of a `SysFn::Dist`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dist {
    Uniform,
    Normal,
    Exponential,
    Poisson,
    ChiSquare,
    T,
    Erlang,
}

#[derive(Clone, Debug)]
pub enum FmtArg {
    /// string literal: used as a format string if it appears where a format is expected
    Lit(String),
    Val { s: u32, w: u32, signed: bool },
    /// a string value (characters, the first in the high byte; NUL bytes are not
    /// characters): an enum's `.name()`
    Str { s: u32, w: u32 },
    /// value produced by $time
    Time { s: u32 },
    /// a `string` value (a handle): printed, or used as a format, like a literal
    SVar { s: u32 },
    /// a real value (f64 bits); without a format `%#g`, or with `fixed` decimals (a bare
    /// `$realtime`: the digits of the module's time precision below its unit, as the reference)
    Real { s: u32, fixed: Option<u8> },
    /// a queue or dynamic array (its handle in slot s, `w`-bit elements) printed as an
    /// assignment pattern (`%p`, IEEE 1800-2017 21.2.1.7)
    Agg { s: u32, w: u32, t: HeapTy, signed: bool },
    /// a one-dimensional fixed-size array printed as an assignment pattern, its elements
    /// from left to right (`desc`: the highest index first)
    ArrAgg { sig: u32, desc: bool, signed: bool, elem: ElemKind },
    /// a value formatted already (`'{1, 2}`: what the simulator makes of `Agg`, `ArrAgg`)
    Pattern(String),
    Empty,
}

/// A file name: a string literal, or a variable holding the characters.
#[derive(Clone, Debug)]
pub enum FileName {
    Lit(String),
    Val { s: u32, w: u32 },
    /// a string value (handle)
    SVar { s: u32 },
}

#[derive(Clone, Debug)]
pub enum SysTask {
    /// $display/$write/$strobe/$monitor body printing; `nl` adds newline. `fd` = None → stdout
    /// `texp`: %t scales a time in module units by 10^texp to simulation ticks
    /// `radix`: default for arguments without a format ('d', 'h', 'b', 'o': $displayh ...)
    Display { nl: bool, args: Vec<FmtArg>, scope: String, fd: Option<(u32, u32)>, prefix: &'static str, texp: u8, radix: char },
    /// mark the given postponed process as the active monitor and wake it
    Monitor { proc: u32 },
    /// only print if this proc is the active monitor
    MonitorPrint { proc: u32, args: Vec<FmtArg>, scope: String, texp: u8, radix: char },
    MonitorOn(bool),
    /// `$fflush`: write out buffered output (of all files)
    Flush,
    Strobe { proc: u32 },
    /// $finish / $stop; `loc` is "file:line" for the message (empty: no message)
    /// `$finish` / `$stop` (`loc`: print the message), `$fatal` (`fatal`: exit status 1)
    Finish { loc: String, stop: bool, fatal: bool },
    DumpFile(String),
    DumpVars,
    DumpOn(bool),
    /// $dumpall: a block with every traced value
    DumpAll,
    /// $dumpflush: write out what the VCD buffer holds
    DumpFlush,
    ReadMem { hex: bool, file: FileName, sig: u32, start: Option<(u32, u32)>, end: Option<(u32, u32)>, loc: String },
    /// $writememh / $writememb
    WriteMem { hex: bool, file: FileName, sig: u32, start: Option<(u32, u32)>, end: Option<(u32, u32)> },
    /// `mode`: "r", "w", "a" (+ "b"/"+"); without a mode the file is opened for writing
    FOpen { d: u32, dw: u32, file: FileName, mode: String },
    FClose { s: u32, loc: String },
    /// `$timeformat(units, precision, suffix, min_width)`; None resets the default
    TimeFormat(Option<fmt::TimeFormat>),
    /// d (1 bit) = some plusarg starts with `prefix`
    TestPlusargs { d: u32, prefix: FileName },
    /// d (1 bit) = a plusarg `<prefix><value>` exists; its value, converted by `conv`
    /// (d h o b s f e g), goes to `val` (`w` bits, or f64 bits if `real`)
    /// `fmt`: the prefix and a conversion (`"name=%d"`), read when the call runs
    ValuePlusargs { d: u32, fmt: FileName, val: u32, w: u32, real: bool },
    /// format into the `w`-bit value at `d` (characters right-aligned)
    /// `nul_warn`: `file:line: $sformat` of the task forms, which warn about a result with a
    /// NUL byte (empty for $sformatf)
    SFormat { d: u32, w: u32, args: Vec<FmtArg>, scope: String, texp: u8, radix: char, nul_warn: String },
    /// file input: the 32-bit result goes to `d`; `src` is the descriptor (or, for $sscanf,
    /// the string) as (slot, width)
    /// `loc`: file:line of the call (for warnings)
    FileRead { kind: FileRead, d: u32, src: (u32, u32), scope: String, loc: String },
}

#[derive(Clone, Debug)]
pub enum FileRead {
    Getc,
    Ungetc { c: u32 },
    /// a line into the `w`-bit value at `val`
    Gets { val: u32, w: u32 },
    Eof,
    Tell,
    Seek { off: u32, op: u32 },
    Rewind,
    /// $fscanf / $sscanf (`string`): conversions go to `outs`
    Scan { string: bool, fmt: FileName, outs: Vec<ScanOut> },
}

/// One $scanf output: the converted value (`w` bits, or f64 if `real`) and a 1-bit flag set
/// when the conversion happened.
#[derive(Clone, Debug)]
pub struct ScanOut {
    pub val: u32,
    pub w: u32,
    pub real: bool,
    pub ok: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealOp {
    /// the bits of a as a shortreal (IEEE 754 single precision) in the low 32 bits
    ToShortBits,
    /// the shortreal whose bits are the low 32 bits of a
    FromShortBits,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    // math system functions (one argument: `b` is ignored)
    Sqrt, Ln, Log10, Exp, Floor, Ceil, Sin, Cos, Tan, Asin, Acos, Atan, Sinh, Cosh, Tanh, Asinh, Acosh, Atanh, Atan2, Hypot,
}

/// Operations on queues and dynamic arrays in `Instr::Heap`. Such an array is an object on the
/// simulator's heap; a variable holds its 64-bit handle, 0 being the empty array. `a` is the
/// slot of a handle: the operations that change the array change the object in place (an
/// empty array gets a new object, whose handle is written to `a`). Indexes are 32-bit signed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapOp {
    /// d (32 bits) = the number of elements of a
    Size,
    /// d = a[b]; outside the array or for an unknown index the default value
    Get,
    /// a[b] = c; b == size appends; outside: no change
    Set,
    /// d = a new array of b elements: those of array c (NONE: none) as far as it has them,
    /// the rest default values (`new[b](c)`), or copies of element a (not NONE)
    New,
    /// d = a copy of array a
    Copy,
    /// the array of handle a is no longer used
    Free,
    /// append element c / put it in front
    PushBack,
    PushFront,
    /// d = the last / first element of a, removed (the default value when a is empty)
    PopBack,
    PopFront,
    /// insert element c before index b (b == size appends)
    Insert,
    /// remove element b (NONE: all elements)
    Delete,
    /// d = a new array of the elements b..=c of a (those inside it)
    Slice,
    /// append the elements of array c
    Append,
    /// d (1 bit) = arrays a and b have the same size and elements
    Eq,
    /// keep at most `bound` elements of the array at a (an array assigned to a bounded queue)
    Truncate,
    /// reorder the elements of a in place (`sort`, `rsort`, `reverse`, `shuffle`)
    Order(OrderOp),
    /// d (1 bit) = array a has an element equal to c (all bits, x and z too)
    Contains,
    /// d (1 bit) = array a has as many elements as the 32-bit value in c (else an error
    /// message: a fixed-size array of that many elements cannot take it)
    SizeIs,
}

/// The ordering methods of `HeapOp::Order` (IEEE 1800-2017 7.12.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderOp {
    /// ascending (`desc`: descending) by value; `signed` vector elements
    Sort { desc: bool, signed: bool },
    /// `sort with (e)` / `rsort with (e)`: by the keys in array b (`kw`-bit elements of
    /// kind `key`, one per element of a)
    SortBy { desc: bool, signed: bool, key: ElemKind, kw: u32 },
    Reverse,
    Shuffle,
}

/// Operations on strings (handles, `Program::strings`) in `Instr::Str`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrOp {
    /// d = the string of the `w`-bit vector at a (its characters, NUL bytes dropped)
    FromVec,
    /// d (`w` bits) = the characters of a, the last one in the low byte
    ToVec,
    /// d = a b
    Concat,
    /// d = a repeated b (32-bit) times
    Repl,
    /// d (1 bit) = a op b, lexicographic
    Cmp(CmpOp),
    /// d (32 bits) = a.len()
    Len,
    /// d (8 bits) = a[b] (b: 32-bit signed), 0 outside
    Getc,
    /// d = a with character b (32-bit signed) set to c (8 bits); unchanged outside or for 0
    Putc,
    Upper,
    Lower,
    /// d = a.substr(b, c) (32-bit signed bounds)
    Substr,
    /// d (32 bits, signed) = a.compare(b) / a.icompare(b)
    Compare,
    Icompare,
    /// d (`w` bits) = a.atoi() and co.
    Atoi,
    Atohex,
    Atooct,
    Atobin,
    /// d (real) = a.atoreal()
    Atoreal,
    /// d = the digits of the `w`-bit value a (c = 1: signed)
    Itoa,
    Hextoa,
    Octtoa,
    Bintoa,
    /// d = a (real) as text
    Realtoa,
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
    /// number of 1 bits (x/z bits do not count): `$countones` (32 bits), `$onehot` (mode 1:
    /// exactly one), `$onehot0` (mode 2: at most one)
    Ones { d: u32, s: u32, w: u32, mode: u8 },
    /// `$countbits`: the number of bits of `s` equal to one of the (1-bit, 4-state) control
    /// values in `ctl` (32 bits)
    CountBits { d: u32, s: u32, w: u32, ctl: Box<[u32]> },
    /// start the fork branches; join: 0 = join (all), 1 = join_any, 2 = join_none, 3 = as
    /// join_none, and a branch that starts with a `Wait` is waiting at once (the thread of an
    /// event-controlled nonblocking assignment: the event control counts from the statement)
    Fork { children: Box<[u32]>, join: u8 },
    /// `wait fork`: suspend until the fork branches this process started have all ended
    WaitFork,
    /// before a fork block sets its automatic variables: the branches' frames (holding them)
    /// are saved, so that the new values do not overwrite a running instance's
    Evict { defs: Box<[u32]> },
    /// disable the named blocks / tasks of `Program::disables[target]`
    Disable { target: u32 },
    /// the `std::process` class (IEEE 1800-2017 9.7), see `ProcOp`
    Proc { op: ProcOp, d: u32, a: u32 },
    /// re-evaluate the waiting processes that write `sig` (a released net takes its drivers'
    /// value again)
    Kick { sig: u32 },
    /// net resolution with drive strengths: per bit the strongest non-z drivers decide (x if
    /// they disagree); `drv`: (value slot, strength0, strength1), levels 0 (highz) .. 7 (supply)
    ResolveStr { d: u32, w: u32, drv: Box<[(u32, u8, u8)]> },
    /// recursion: save memory ranges (lo, words) on the process's frame stack
    SaveFrame { ranges: Box<[(u32, u32)]> },
    /// recursion: call a subroutine at `entry`; its results go to `dsts` on return
    CallSub { entry: u32, dsts: Box<[u32]> },
    /// recursion: return; the values (slot, words) in `srcs` go to the caller's `dsts` after the
    /// saved frame is restored
    RetSub { srcs: Box<[(u32, u32)]> },
    /// a change event on `sig` without an edge (time-0 events of initial values, like Icarus)
    Touch { sig: u32 },
    /// release / deassign: the bits of the target are no longer forced
    Unforce { t: Target },
    /// evaluate UDP table `table` for the 1-bit inputs `ins` (previous values in `prev`, updated)
    /// and the state / output in `q` (updated)
    Udp { table: u32, ins: Box<[u32]>, prev: Box<[u32]>, q: u32 },
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
    /// a string operation; unused operands are NONE
    Str { op: StrOp, d: u32, a: u32, b: u32, c: u32, w: u32 },
    /// an operation on a queue or dynamic array of `w`-bit elements; unused operands are NONE;
    /// `at`: the source location for warnings (`Program::locs`)
    Heap { op: HeapOp, d: u32, a: u32, b: u32, c: u32, w: u32, t: HeapTy, at: u32 },
    /// an operation on an associative array of `w`-bit elements with keys of type `key`
    /// (slot `b`); `x` as for `Heap`
    Assoc { op: AssocOp, d: u32, a: u32, b: u32, c: u32, w: u32, key: KeyKind, x: bool },
    /// an operation on class objects; unused operands are NONE
    Obj { op: ObjOp, d: u32, a: u32, b: u32, off: u32, w: u32 },
    /// concurrent assertions
    Assert(AssertOp),
}

/// Operations of concurrent assertions (`Instr::Assert`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertOp {
    /// a tick of assertion `id` (`Program::asserts`): its leaves are the bits of the `n`-bit
    /// value at `leaves`; the numbers of passes and failures go to the 32-bit `pass`, `fail`
    Tick { id: u32, leaves: u32, n: u32, pass: u32, fail: u32 },
    /// `disable iff`: the running attempts of assertion `id` end
    Disable { id: u32 },
    /// the process waits for the Reactive region of this time step (action blocks)
    Reactive,
    /// action process `proc` of a deferred assertion runs (from its start) in the Reactive
    /// region, or the Postponed one: once, also when scheduled again before it runs
    Defer { proc: u32, postponed: bool },
}

/// Operations of `Instr::Proc`. A process is named by a 64-bit id, (incarnation << 32) |
/// (process + 1): a fork branch started again is another process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcOp {
    /// d (64 bits) = the id of the running process
    Self_,
    /// d (32 bits) = the state of process a: 0 finished, 1 running, 2 waiting, 4 killed
    Status,
    /// end process a and the fork branches it started
    Kill,
    /// suspend until process a has ended
    Await,
}

/// Operations on class objects in `Instr::Obj`. An object is on the simulator's heap; a
/// handle (64 bits) refers to it, 0 being `null`. Objects are shared, never freed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjOp {
    /// d = a new object of class `w` (`Program::classes`)
    New,
    /// d (`w` bits) = bits `off..off + w` of object a (x when a is null)
    Get,
    /// bits `off..off + w` of object a = b (no change when a is null)
    Set,
    /// d = a new object with the class and properties of object a (null: null)
    Copy,
    /// d (32 bits) = the class of object a (all ones for null)
    Class,
    /// d (`w` bits) = element b (a 32-bit index; x/z or outside 0..n: x) of the array of n
    /// `w`-bit elements at bit `off` of object a
    GetAt(u32),
    /// element d (a 32-bit index; x/z or outside 0..n: no change) of the array of n `w`-bit
    /// elements at bit `off` of object a = b
    SetAt(u32),
}

/// The array of an `Instr::Heap`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapTy {
    pub elem: ElemKind,
    /// a queue (its operations warn about indexes outside it, as the reference does)
    pub queue: bool,
    /// the most elements a bounded queue (`[$:N]`, N + 1) holds; 0: unbounded
    pub bound: u32,
    /// an element outside the array reads as x (4-state vectors), else as 0
    pub x: bool,
}

/// The element type of an array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElemKind {
    Vec,
    Real,
    Str,
}

/// Operations on associative arrays in `Instr::Assoc` (objects on the heap as for
/// `HeapOp`; `HeapOp::Copy` and `Free` work on them too). `a` is the slot of a handle, `b`
/// of a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssocOp {
    /// d (32 bits) = the number of entries
    Num,
    /// d = a[b]; the default value for a key without an entry
    Get,
    /// a[b] = c
    Set,
    /// d (32 bits) = 1 if key b has an entry, else 0
    Exists,
    /// remove the entry of key b (NONE: all entries)
    Delete,
    /// c = the first / last key (d (32 bits) = 1), or c = b when there is none (d = 0)
    First,
    Last,
    /// c = the key after / before b (d = 1), or c = b when there is none (d = 0)
    Next,
    Prev,
}

/// The key type of an associative array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// a string (handle)
    Str,
    /// an integer of `w` (at most 64) bits
    Int { w: u32, signed: bool },
}
