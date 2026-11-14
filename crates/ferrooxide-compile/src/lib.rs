//! Elaboration (parameters, hierarchy, generate) and code generation to ferrooxide-rt bytecode.

mod expr;
mod konst;
mod lint;
mod stmt;

pub use konst::Const;

use ferrooxide_front::ast::*;
use ferrooxide_front::Error;
use ferrooxide_rt::{self as rt, Instr, Program, ProcClass, ProcDef, SigKind, SignalInfo, NONE};
use ferrooxide_front::svtypes::SvType;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub(crate) type R<T> = Result<T, Error>;

/// Size of the shared scratch region for process temporaries, at the start of memory.
const SCRATCH_WORDS: u32 = 1 << 16;

#[derive(Clone, Debug)]
pub(crate) enum Sym {
    Sig(u32),
    Const(Const),
    /// a real parameter
    Real(f64),
    /// a type parameter's type, its dimensions evaluated (constants)
    Type(Rc<SvType>),
}

/// A bidirectional connection between bits of two nets (`tran`, an inout port connected to a
/// part of a net): each side gets the other's drivers as an extra driver. `ctl`: tranif
/// control (expression, scope, conducts when 1).
#[derive(Clone, Debug)]
pub(crate) struct Link {
    a: u32,
    a_lsb: u32,
    b: u32,
    b_lsb: u32,
    w: u32,
    ctl: Option<(Expr, usize, bool)>,
}

/// A parameter value given by an override.
#[derive(Clone, Debug)]
pub(crate) enum PVal {
    Int(Const),
    Real(f64),
    Type(Rc<SvType>),
}

pub(crate) struct Scope<'a> {
    pub name: String,
    pub parent: Option<usize>,
    pub syms: HashMap<String, Sym>,
    pub children: HashMap<String, usize>,
    pub funcs: HashMap<String, &'a SubroutineDecl>,
    /// declared [msb:lsb] of parameters with a range (for bit and part selects)
    pub param_ranges: HashMap<String, (i64, i64)>,
    pub file: String,
    /// time unit and precision of the enclosing module (powers of ten of seconds)
    pub tunit: i32,
    pub tprec: i32,
    /// `default_nettype of the module (None: wire)
    pub nettype: Option<String>,
    /// `unconnected_drive of the module
    pub uncdrive: Option<bool>,
    /// package scopes whose names a module sees after its own (module scopes only)
    pub imports: Vec<usize>,
}

enum Job<'a> {
    Proc { kind: ProcKind, body: &'a Stmt, scope: usize, line: u32 },
    Assign { lhs: Expr, rhs: Expr, delay: Option<Expr>, scope: usize, line: u32 },
    /// child input port driven by a parent expression
    PortIn { child_sig: u32, expr: Expr, scope: usize, line: u32 },
    /// parent lvalue driven by a child output port
    PortOut { lhs: Expr, child_sig: u32, scope: usize, line: u32 },
    /// an instance of a user-defined primitive: output, inputs (in the instantiating scope)
    Udp { module: &'a Module, terms: Vec<Expr>, delay: Option<Expr>, scope: usize, line: u32 },
}

#[derive(Default)]
pub(crate) struct PState {
    pub proc: u32,
    pub code: Vec<Instr>,
    /// loop stack: (break patches, continue patches)
    pub loops: Vec<(Vec<usize>, Vec<usize>)>,
    /// function/task bodies being inlined: (return variable, patches to the end)
    pub rets: Vec<(Option<u32>, Vec<usize>)>,
}

pub(crate) struct FuncInst {
    pub scope: usize,
    pub ret: Option<u32>,
    /// (dir, signal) in declaration order
    pub ports: Vec<(Dir, u32)>,
    /// the memory words of its variables (lo, hi)
    pub mem: (u32, u32),
}

/// A recursive function compiled as a subroutine of a process: its body is placed after the
/// process's code; calls jump to it with a saved frame.
pub(crate) struct SubReq<'a> {
    /// the process nesting level (pb.len()) it belongs to
    pub level: usize,
    pub f: &'a SubroutineDecl,
    pub key: (usize, String),
    /// CallSub instructions to point at the entry
    pub calls: Vec<usize>,
    /// SaveFrame instructions to complete with the ranges
    pub saves: Vec<usize>,
    pub entry: Option<usize>,
    /// memory allocated while compiling the body (block variables, temporaries)
    pub alloc: (u32, u32),
}

pub struct Compiler<'a> {
    mods: HashMap<&'a str, &'a Module>,
    pub prog: Program,
    pub(crate) scopes: Vec<Scope<'a>>,
    tops: HashMap<String, usize>,
    jobs: Vec<Job<'a>>,
    consts: HashMap<Const, u32>,
    pub(crate) pb: Vec<PState>,
    pub(crate) func_inst: HashMap<(usize, String), FuncInst>,
    pub(crate) file: String,
    pub(crate) line: u32,
    pub warnings: Vec<String>,
    /// lint findings (only collected with `lint_enabled`)
    pub lint: Vec<String>,
    lint_seen: HashSet<String>,
    pub lint_enabled: bool,
    /// 2-state variables (bit, int, byte, ...): stores turn x/z into 0, they start at 0
    pub(crate) two_state: HashSet<u32>,
    pub(crate) call_depth: u32,
    /// functions being inlined: (definition scope, name, instance key)
    pub(crate) inline_stack: Vec<(usize, String, (usize, String))>,
    /// recursive functions compiled as subroutines (of the process at `level`)
    pub(crate) subs: Vec<SubReq<'a>>,
    warned: HashSet<String>,
    scratch_top: u32,
    scratch_saved: Vec<u32>,
    /// simulation precision (power of ten of seconds): the smallest of all modules
    pub(crate) gprec: i32,
    /// (array, element) -> alias signal created for port connections
    elem_alias: HashMap<(u32, u32), u32>,
    /// views: another name for a signal with its own declared range and sign (a port of a
    /// different range or sign collapsed onto its connection, a net aliased to the source of
    /// a plain `assign`) -> the signal it stands for. Instructions refer to the view while
    /// compiling (selects and sign use its declaration) and to the signal afterwards.
    pub(crate) view_of: HashMap<u32, u32>,
    /// (view, driver signal) -> view of the driver (see `redir`)
    drv_views: HashMap<(u32, u32), u32>,
    /// variables made aliases by `alias_assigns` (a procedural store to one is an error)
    alias_vars: HashSet<u32>,
    /// named blocks and task bodies for `disable`: (path, definition)
    pub(crate) block_reg: Vec<(String, rt::BlockDef)>,
    /// registry entries of the blocks being compiled (fork branches started inside them are
    /// stopped when they are disabled)
    pub(crate) open_blocks: Vec<usize>,
    /// disable targets to resolve after code generation: (target, candidate paths, file, line)
    pub(crate) pending_disables: Vec<(u32, Vec<String>, String, u32)>,
    /// memory limit (words) and the error raised when a declaration exceeds it
    pub(crate) mem_limit_words: u64,
    pub(crate) mem_error: Option<Error>,
    /// signals given an initial value before time 0 (declaration initializers, constant
    /// continuous assignments): they get a change event at time 0, as with Icarus
    init_touch: Vec<u32>,
    /// the same for nets with constant drivers (their events come first)
    init_touch_net: Vec<u32>,
    /// defparam overrides by full parameter path
    defparams: HashMap<String, PVal>,
    /// parameters already evaluated (full paths), a defparam for one of them came too late,
    /// and whether this is the second elaboration
    params_done: HashSet<String>,
    late_defparam: bool,
    restarted: bool,
    top_names: HashSet<String>,
    /// the scope of the job being compiled, and per process the scope it was created in
    job_scope: usize,
    proc_scope: Vec<usize>,
    /// package (and `$unit`) scopes by name
    pub(crate) pkg_scopes: HashMap<String, usize>,
    /// UDP name -> index of its runtime table
    pub(crate) udp_index: HashMap<String, u32>,
    /// counter for per-call storage of automatic functions and tasks
    pub(crate) auto_count: u32,
    /// nets of a special type (tri0, wand, ...): resolved even with one or no driver
    pub(crate) net_types: HashMap<u32, NetType>,
    /// nets with a pullup (true) or pulldown (false) gate
    pub(crate) pulls: HashMap<u32, Vec<(u32, u32, bool)>>,
    /// nets with several continuous drivers -> their driver signals
    drivers: HashMap<u32, Vec<u32>>,
    /// while compiling a driver of a multi-driven net: net -> this driver's signal
    redirect: HashMap<u32, u32>,
    dialect: Dialect,
    links: Vec<Link>,
    /// drive strength of the continuous assignments being created, and per job index
    cur_strength: Option<(u8, u8)>,
    /// multi-dimensional arrays (stored flat): per dimension (lowest index, size)
    pub(crate) md_dims: HashMap<u32, Vec<(i64, u32)>>,
    /// vectors with several packed dimensions (stored [W-1:0]): per dimension (left, right)
    pub(crate) packed_dims: HashMap<u32, Vec<(i64, i64)>>,
    /// delay of the gate being instantiated
    cur_delay: Option<Expr>,
    job_strength: HashMap<usize, (u8, u8)>,
    /// read sets include signals read inside called functions (lint only)
    pub(crate) func_reads: std::cell::Cell<bool>,
    /// functions being followed by `func_reads` (recursion guard)
    pub(crate) func_reads_seen: std::cell::RefCell<Vec<(usize, String)>>,
}

/// Compile parsed modules into a runnable program. `top`: explicit top-level module(s).
pub fn compile(modules: &[Module], tops: &[String]) -> Result<(Program, Vec<String>), Error> {
    compile_lint(modules, tops, false).map(|(p, w, _)| (p, w))
}

/// Language rules where Verilog-2005 and SystemVerilog differ.
#[derive(Clone, Copy, Debug, Default)]
pub struct Dialect {
    /// Verilog-2005 (`-g2005`): a variable declaration initializer is an assignment at time 0,
    /// with a change event; in SystemVerilog (the default) it sets the value before time 0
    pub v2005: bool,
    /// `-gspecify`: specify path delays apply (otherwise they are ignored)
    pub specify: bool,
}

/// Compile and, with `lint`, also collect lint findings: (program, warnings, lint findings).
pub fn compile_lint(modules: &[Module], tops: &[String], lint: bool) -> Result<(Program, Vec<String>, Vec<String>), Error> {
    compile_dialect(modules, tops, lint, Dialect::default())
}

pub fn compile_dialect(
    modules: &[Module],
    tops: &[String],
    lint: bool,
    dialect: Dialect,
) -> Result<(Program, Vec<String>, Vec<String>), Error> {
    let top_mods: Vec<&Module> = if tops.is_empty() {
        let mut used = HashSet::new();
        fn scan<'b>(items: &'b [Item], used: &mut HashSet<&'b str>) {
            for it in items {
                match it {
                    Item::Instances { module, .. } => {
                        used.insert(module.as_str());
                    }
                    Item::GenFor { items, .. } | Item::GenBlock { items, .. } => scan(items, used),
                    Item::GenIf { then_items, else_items, .. } => {
                        scan(then_items, used);
                        scan(else_items, used);
                    }
                    Item::GenCase { arms, .. } => {
                        for (_, _, i) in arms {
                            scan(i, used);
                        }
                    }
                    _ => {}
                }
            }
        }
        for m in modules {
            scan(&m.items, &mut used);
        }
        modules.iter().filter(|m| !used.contains(m.name.as_str()) && m.udp.is_none() && !m.interface && !m.package).collect()
    } else {
        let mut v = Vec::new();
        for t in tops {
            match modules.iter().find(|m| &m.name == t) {
                Some(m) => v.push(m),
                None => {
                    return Err(Error { msg: format!("top module '{t}' not found"), file: String::new(), line: 0 })
                }
            }
        }
        v
    };
    if top_mods.is_empty() {
        return Err(Error { msg: "no top-level module found".into(), file: String::new(), line: 0 });
    }
    // elaboration; a defparam found after its target parameter was evaluated restarts it once,
    // with all defparams known from the start
    let mut seed: Option<HashMap<String, PVal>> = None;
    let mut c = loop {
    let mut c = Compiler::new(modules);
    c.lint_enabled = lint;
    c.dialect = dialect;
    c.gprec = modules.iter().map(|m| m.timescale.map(|t| t.1).unwrap_or(0)).min().unwrap_or(0);
    if let Some(sd) = &seed {
        c.defparams = sd.clone();
        c.restarted = true;
    }
    c.top_names = top_mods.iter().map(|m| m.name.clone()).collect();
    // defparams in top modules that reach into other top modules must be known before those
    // are elaborated: evaluate them in a scratch copy of their module's parameters
    for m in &top_mods {
        if m.items.iter().any(|i| matches!(i, Item::Defparam { .. })) {
            let saved = std::mem::replace(&mut c.file, m.file.clone());
            let sid = c.new_scope(m.name.clone(), None, m.file.clone());
            c.eval_params(m, &[], sid)?;
            c.register_defparams(&m.items, sid)?;
            c.file = saved;
        }
    }
    // packages with variables first: one instance each, seen by the modules that import them
    for m in modules.iter().filter(|m| m.package) {
        let sid = c.instantiate(m, m.name.clone(), &[], None)?;
        c.pkg_scopes.insert(m.name.clone(), sid);
    }
    for m in &top_mods {
        let sid = c.instantiate(m, m.name.clone(), &[], None)?;
        c.tops.insert(m.name.clone(), sid);
    }
    if c.late_defparam && seed.is_none() {
        seed = Some(c.defparams.clone());
        continue;
    }
    break c;
    };
    if let Some(e) = c.mem_error.take() {
        return Err(e);
    }
    c.codegen_jobs()?;
    c.init_events();
    c.resolve_disables();
    c.resolve_views();
    c.start_order();
    // package variables are not dumped (as in the reference)
    if !c.pkg_scopes.is_empty() {
        let pre: Vec<String> = c.pkg_scopes.keys().map(|n| format!("{n}.")).collect();
        c.prog.names.retain(|(n, _)| !pre.iter().any(|p| n.starts_with(p.as_str())));
        let mut pk: Vec<String> = c.pkg_scopes.keys().cloned().collect();
        pk.sort();
        c.prog.packages = pk;
    }
    if let Some(e) = c.mem_error.take() {
        return Err(e);
    }
    c.prog.scratch_words = SCRATCH_WORDS;
    c.prog.precision = c.gprec;
    if lint {
        // names that are SystemVerilog keywords: the code does not compile as SystemVerilog
        for m in modules {
            // once per name and module, where it first appears (usually its declaration)
            let mut seen = HashSet::new();
            for (n, l) in &m.keyword_names {
                if !seen.insert(n) {
                    continue;
                }
                let msg = format!(
                    "{}:{l}: lint: '{n}' is a SystemVerilog keyword (IEEE 1800) used as a name; \
                     SystemVerilog tools reject it, rename it",
                    m.file
                );
                if c.lint_seen.insert(msg.clone()) {
                    c.lint.push(msg);
                }
            }
        }
        // in source order
        let key = |s: &String| -> (String, u64) {
            let mut it = s.splitn(3, ':');
            let f = it.next().unwrap_or("").to_string();
            (f, it.next().and_then(|l| l.parse().ok()).unwrap_or(0))
        };
        c.lint.sort_by_key(key);
    }
    Ok((c.prog, c.warnings, c.lint))
}

impl<'a> Compiler<'a> {
    fn new(modules: &'a [Module]) -> Self {
        let mut mods = HashMap::new();
        for m in modules {
            mods.insert(m.name.as_str(), m);
        }
        Compiler {
            mods,
            prog: Program { mem: vec![0; SCRATCH_WORDS as usize], ..Program::default() },
            scopes: Vec::new(),
            tops: HashMap::new(),
            jobs: Vec::new(),
            consts: HashMap::new(),
            pb: Vec::new(),
            func_inst: HashMap::new(),
            file: String::new(),
            line: 0,
            warnings: Vec::new(),
            lint: Vec::new(),
            lint_seen: HashSet::new(),
            lint_enabled: false,
            two_state: HashSet::new(),
            call_depth: 0,
            inline_stack: Vec::new(),
            subs: Vec::new(),
            warned: HashSet::new(),
            scratch_top: 0,
            scratch_saved: Vec::new(),
            gprec: 0,
            elem_alias: HashMap::new(),
            view_of: HashMap::new(),
            drv_views: HashMap::new(),
            alias_vars: HashSet::new(),
            drivers: HashMap::new(),
            net_types: HashMap::new(),
            pulls: HashMap::new(),
            block_reg: Vec::new(),
            open_blocks: Vec::new(),
            pending_disables: Vec::new(),
            auto_count: 0,
            udp_index: HashMap::new(),
            mem_limit_words: std::env::var("FX_MAX_MEM_MB")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2048)
                .min(32 * 1024)
                * (1 << 20)
                / 8,
            mem_error: None,
            defparams: HashMap::new(),
            params_done: HashSet::new(),
            late_defparam: false,
            restarted: false,
            init_touch: Vec::new(),
            init_touch_net: Vec::new(),
            dialect: Dialect::default(),
            links: Vec::new(),
            cur_strength: None,
            md_dims: HashMap::new(),
            packed_dims: HashMap::new(),
            cur_delay: None,
            job_strength: HashMap::new(),
            func_reads: std::cell::Cell::new(false),
            func_reads_seen: std::cell::RefCell::new(Vec::new()),
            top_names: HashSet::new(),
            pkg_scopes: HashMap::new(),
            job_scope: usize::MAX,
            proc_scope: Vec::new(),
            redirect: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------ utilities
    pub(crate) fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(Error { msg: msg.into(), file: self.file.clone(), line: self.line })
    }

    pub(crate) fn warn(&mut self, msg: impl Into<String>) {
        let m = format!("{}:{}: warning: {}", self.file, self.line, msg.into());
        if self.warned.insert(m.clone()) {
            self.warnings.push(m);
        }
    }

    /// allocate raw memory for a value (zero-initialised)
    pub(crate) fn alloc(&mut self, w: u32) -> u32 {
        let s = self.prog.mem.len() as u32;
        self.prog.mem.resize(self.prog.mem.len() + rt::words(w) as usize, 0);
        s
    }

    /// A temporary that is dead at every suspension point. Process temporaries share one
    /// scratch region (only one process runs at a time), which keeps them hot in the cache.
    pub(crate) fn tmp(&mut self, w: u32) -> u32 {
        let in_proc = self.pb.last().map(|p| p.proc != NONE).unwrap_or(false);
        let n = rt::words(w);
        if in_proc && self.scratch_top + n <= SCRATCH_WORDS {
            let s = self.scratch_top;
            self.scratch_top += n;
            return s;
        }
        self.alloc(w)
    }

    /// A temporary that stays live across suspension points (private to the process).
    pub(crate) fn ptmp(&mut self, w: u32) -> u32 {
        self.alloc(w)
    }

    pub(crate) fn const_slot(&mut self, c: &Const) -> u32 {
        let key = Const { signed: false, ..c.clone() };
        if let Some(&s) = self.consts.get(&key) {
            return s;
        }
        let s = self.alloc(c.w);
        let n = rt::nw(c.w);
        for i in 0..n {
            self.prog.mem[s as usize + i] = c.a[i];
            self.prog.mem[s as usize + n + i] = c.b[i];
        }
        self.consts.insert(key, s);
        s
    }

    pub(crate) fn read_const(&self, slot: u32, w: u32, signed: bool) -> Const {
        let n = rt::nw(w);
        let s = slot as usize;
        Const { w, signed, a: self.prog.mem[s..s + n].to_vec(), b: self.prog.mem[s + n..s + 2 * n].to_vec() }
    }

    pub(crate) fn new_signal(
        &mut self,
        name: String,
        width: u32,
        signed: bool,
        depth: u32,
        msb: i64,
        lsb: i64,
        arr_lo: i64,
        kind: SigKind,
    ) -> u32 {
        let total_w = width;
        let mut depth = depth;
        let mut elems = depth.max(1);
        // refuse designs that would not fit (instead of running the machine out of memory)
        let need = elems as u64 * 2 * rt::nw(total_w) as u64 + self.prog.mem.len() as u64;
        if need > self.mem_limit_words {
            if self.mem_error.is_none() {
                self.mem_error = Some(Error {
                    msg: format!(
                        "'{}' needs {} MB of simulation memory, more than the limit of {} MB (set FX_MAX_MEM_MB to raise it)",
                        name,
                        need * 8 >> 20,
                        self.mem_limit_words * 8 >> 20
                    ),
                    file: self.file.clone(),
                    line: self.line,
                });
            }
            depth = depth.min(1);
            elems = 1;
        }
        let slot = self.prog.mem.len() as u32;
        let n = rt::nw(total_w);
        for _ in 0..elems {
            for plane in 0..2 {
                for i in 0..n {
                    let full = if i == n - 1 { rt::tmask(width) } else { !0 };
                    // wires start as z (a=0,b=1), variables as x (a=1,b=1)
                    let v = match (kind, plane) {
                        (SigKind::Wire, 0) => 0,
                        _ => full,
                    };
                    self.prog.mem.push(v);
                }
            }
        }
        let id = self.prog.signals.len() as u32;
        self.prog.signals.push(SignalInfo {
            name: name.clone(),
            slot,
            width,
            signed,
            depth,
            msb,
            lsb,
            arr_lo,
            kind,
            alias_of: None,
            real: false,
            forced: false,
        });
        self.prog.names.push((name, id));
        id
    }

    pub(crate) fn emit(&mut self, mut i: Instr) -> usize {
        // a store into a 2-state variable: x/z bits of the value become 0 first
        if let Instr::Store { t, s, mode } = i {
            use rt::Target;
            let sig = match t {
                Target::Whole { sig }
                | Target::Part { sig, .. }
                | Target::DynPart { sig, .. }
                | Target::Elem { sig, .. }
                | Target::ElemK { sig, .. }
                | Target::ElemPart { sig, .. }
                | Target::ElemDynPart { sig, .. } => sig,
            };
            if !self.alias_vars.is_empty() {
                let mut v = sig;
                loop {
                    if self.alias_vars.contains(&v) {
                        if self.mem_error.is_none() {
                            let name = self.sig(v).name.rsplit('.').next().unwrap_or("").to_string();
                            self.mem_error = self
                                .err::<()>(format!(
                                    "Cannot perform procedural assignment to variable '{name}' because it is also continuously assigned."
                                ))
                                .err();
                        }
                        break;
                    }
                    match self.view_of.get(&v) {
                        Some(&b) => v = b,
                        None => break,
                    }
                }
            }
            // (a value converted from a real keeps its x from inf/nan, as in the reference
            // simulator: ICARUS_NOTES.md)
            let from_real = {
                let code = &self.pb.last().unwrap().code;
                let mut v = s;
                let mut found = false;
                for ins in code.iter().rev().take(4) {
                    match *ins {
                        Instr::Resize { d, s: x, .. } | Instr::Copy { d, s: x, .. } if d == v => v = x,
                        Instr::RToI { d, .. } if d == v => {
                            found = true;
                            break;
                        }
                        _ => break,
                    }
                }
                found
            };
            if self.two_state.contains(&self.canon(sig)) && !from_real {
                let w = match t {
                    Target::Part { w, .. } | Target::DynPart { w, .. } | Target::ElemPart { w, .. } | Target::ElemDynPart { w, .. } => w,
                    _ => self.sig(sig).width,
                };
                let d = self.tmp(w);
                self.pb.last_mut().unwrap().code.push(Instr::Un { op: rt::UnOp::To2, d, s, w });
                i = Instr::Store { t, s: d, mode };
            }
        }
        let c = &mut self.pb.last_mut().unwrap().code;
        c.push(i);
        c.len() - 1
    }

    pub(crate) fn here(&self) -> u32 {
        self.pb.last().unwrap().code.len() as u32
    }

    pub(crate) fn patch(&mut self, at: usize, to: u32) {
        match &mut self.pb.last_mut().unwrap().code[at] {
            Instr::Jmp { to: t } | Instr::Jz { to: t, .. } => *t = to,
            _ => panic!("patching a non-jump"),
        }
    }

    pub(crate) fn begin_proc(&mut self, name: String, class: ProcClass) -> u32 {
        let id = self.prog.procs.len() as u32;
        self.proc_scope.push(self.job_scope);
        self.prog.procs.push(ProcDef { name, code: Vec::new(), class, parent: NONE, fusible: false });
        self.pb.push(PState { proc: id, ..Default::default() });
        self.scratch_saved.push(self.scratch_top);
        self.scratch_top = 0;
        id
    }

    pub(crate) fn end_proc(&mut self) {
        if let Err(e) = self.finish_subs() {
            if self.mem_error.is_none() {
                self.mem_error = Some(e);
            }
        }
        // nested processes ($monitor/$strobe bodies) restart at 0: the enclosing process's
        // temporaries are dead whenever the nested one runs (it runs in the postponed region)
        self.scratch_top = self.scratch_saved.pop().unwrap().max(self.scratch_top);
        let st = self.pb.pop().unwrap();
        self.prog.procs[st.proc as usize].code = st.code;
    }

    /// An event control: (signal, edge, array element or NONE) triggers.
    pub(crate) fn make_ctl(&mut self, mut triggers: Vec<(u32, rt::Edge, u32)>) -> u32 {
        triggers.sort_by_key(|t| (t.0, t.2, t.1 as u8));
        triggers.dedup();
        let proc = self.pb.last().unwrap().proc;
        let id = self.prog.ctls.len() as u32;
        self.prog.ctls.push(rt::EventCtl { proc, triggers });
        id
    }

    pub(crate) fn new_scope(&mut self, name: String, parent: Option<usize>, file: String) -> usize {
        self.scopes.push(Scope {
            name,
            parent,
            syms: HashMap::new(),
            children: HashMap::new(),
            funcs: HashMap::new(),
            param_ranges: HashMap::new(),
            file,
            tunit: parent.map(|p| self.scopes[p].tunit).unwrap_or(0),
            tprec: parent.map(|p| self.scopes[p].tprec).unwrap_or(0),
            nettype: parent.and_then(|p| self.scopes[p].nettype.clone()),
            uncdrive: parent.and_then(|p| self.scopes[p].uncdrive),
            imports: Vec::new(),
        });
        self.scopes.len() - 1
    }

    /// %t / delay scaling of a scope: its time unit in simulation ticks is 10^texp
    pub(crate) fn texp(&self, sid: usize) -> u8 {
        (self.scopes[sid].tunit - self.gprec).clamp(0, 18) as u8
    }

    /// Named events start at 0: a trigger toggles them, so every trigger is a change.
    pub(crate) fn zero_init(&mut self, s: u32) {
        let info = &self.prog.signals[s as usize];
        let (slot, n) = (info.slot as usize, rt::words(info.width) as usize * info.depth.max(1) as usize);
        for w in &mut self.prog.mem[slot..slot + n] {
            *w = 0;
        }
    }

    /// A variable of a 2-state type (bit, byte, shortint, int, longint): starts at 0, stores
    /// turn x/z into 0.
    pub(crate) fn mark_two_state(&mut self, s: u32, kind: NetKind) {
        if matches!(kind, NetKind::Bit) || matches!(kind, NetKind::Int(n) if n > 0) {
            self.two_state.insert(s);
            self.zero_init(s);
        }
    }

    pub(crate) fn mark_real(&mut self, s: u32) {
        let info = &mut self.prog.signals[s as usize];
        info.real = true;
        let (slot, n) = (info.slot as usize, rt::words(info.width) as usize * info.depth.max(1) as usize);
        for w in &mut self.prog.mem[slot..slot + n] {
            *w = 0;
        }
    }

    /// Declared range of a parameter named `name` visible from `sid`.
    pub(crate) fn param_range(&self, name: &str, mut sid: usize) -> Option<(i64, i64)> {
        loop {
            if self.scopes[sid].syms.contains_key(name) {
                return self.scopes[sid].param_ranges.get(name).copied();
            }
            sid = self.scopes[sid].parent?;
        }
    }

    pub(crate) fn lookup(&self, name: &str, mut sid: usize) -> Option<Sym> {
        loop {
            if let Some(s) = self.scopes[sid].syms.get(name) {
                return Some(s.clone());
            }
            for &p in &self.scopes[sid].imports {
                if let Some(s) = self.scopes[p].syms.get(name) {
                    return Some(s.clone());
                }
            }
            sid = self.scopes[sid].parent?;
        }
    }

    pub(crate) fn lookup_func(&self, name: &str, mut sid: usize) -> Option<(&'a SubroutineDecl, usize)> {
        if name.contains('.') {
            // hierarchical: inst.inst.task
            let parts: Vec<String> = name.split('.').map(String::from).collect();
            let sc = self.hier_scope(&parts[..parts.len() - 1], sid)?;
            return self.scopes[sc].funcs.get(parts.last().unwrap()).map(|f| (*f, sc));
        }
        loop {
            if let Some(f) = self.scopes[sid].funcs.get(name) {
                return Some((f, sid));
            }
            // functions and tasks of packages run in their package's scope
            for &p in &self.scopes[sid].imports {
                if let Some(f) = self.scopes[p].funcs.get(name) {
                    return Some((f, p));
                }
            }
            sid = self.scopes[sid].parent?;
        }
    }

    /// Resolve a hierarchical path a.b.c to a symbol.
    pub(crate) fn lookup_hier(&self, parts: &[String], sid: usize) -> Option<Sym> {
        let parts = &self.hier_parts(parts, sid)?;
        let cur = self.hier_scope(&parts[..parts.len() - 1], sid)?;
        self.scopes[cur].syms.get(parts.last().unwrap()).cloned()
    }

    /// The scope named by a hierarchical path (instance names), searched from `sid` upwards and
    /// then from the top-level modules.
    /// Hierarchical-name segments with computed indices (`blk[@N]`) evaluated in scope `sid`.
    pub(crate) fn hier_parts(&self, parts: &[String], sid: usize) -> Option<Vec<String>> {
        let mut out = Vec::with_capacity(parts.len());
        for p in parts {
            match p.find("[@") {
                Some(i) if p.ends_with(']') => {
                    let n: usize = p[i + 2..p.len() - 1].parse().ok()?;
                    let e = ferrooxide_front::parser::hier_index_expr(n)?;
                    let v = self.const_value(&e, sid)?;
                    out.push(format!("{}[{v}]", &p[..i]));
                }
                _ => out.push(p.clone()),
            }
        }
        Some(out)
    }

    pub(crate) fn hier_scope(&self, parts: &[String], sid: usize) -> Option<usize> {
        if parts.is_empty() {
            return Some(sid);
        }
        let normalized;
        let parts = if parts.iter().any(|p| p.contains("[@")) {
            normalized = self.hier_parts(parts, sid)?;
            &normalized[..]
        } else {
            parts
        };
        // find the scope for parts[0]
        let mut s = Some(sid);
        let mut start = None;
        while let Some(x) = s {
            if let Some(&c) = self.scopes[x].children.get(&parts[0]) {
                start = Some(c);
                break;
            }
            if self.scopes[x].name.rsplit('.').next() == Some(parts[0].as_str()) {
                start = Some(x);
                break;
            }
            s = self.scopes[x].parent;
        }
        let mut cur = match start {
            Some(c) => c,
            None => *self.tops.get(&parts[0]).or_else(|| self.pkg_scopes.get(&parts[0]))?,
        };
        for p in &parts[1..] {
            cur = *self.scopes[cur].children.get(p)?;
        }
        Some(cur)
    }

    /// A port connected to a constant-index array element of the same width becomes an alias of
    /// the element (no copy process). Returns the alias signal.
    fn elem_port_alias(&mut self, conn: &Expr, ps: usize, width: u32, name: &str) -> Option<u32> {
        let Expr::Index(a, i) = conn else { return None };
        let arr = self.array_of(a, ps)?;
        let e = self.const_value(i, ps)? - self.sig(arr).arr_lo;
        let info = self.sig(arr).clone();
        if info.width != width || e < 0 || e >= info.depth as i64 {
            return None;
        }
        let e = e as u32;
        if let Some(&s) = self.elem_alias.get(&(arr, e)) {
            return Some(s);
        }
        let id = self.prog.signals.len() as u32;
        self.prog.signals.push(SignalInfo {
            name: name.to_string(),
            slot: info.slot + e * rt::words(width),
            width,
            signed: info.signed,
            depth: 0,
            msb: info.msb,
            lsb: info.lsb,
            arr_lo: 0,
            kind: info.kind,
            alias_of: Some((arr, e)),
            real: info.real,
            forced: false,
        });
        self.elem_alias.insert((arr, e), id);
        Some(id)
    }

    /// The order in which processes start at time 0, as in the reference: top-level modules
    /// by name; in a scope first its generate scopes, then its instances (in source order,
    /// each with everything below it), then its own processes in source order.
    fn start_order(&mut self) {
        let n = self.scopes.len();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (s, sc) in self.scopes.iter().enumerate() {
            let mut v: Vec<usize> = sc.children.values().copied().filter(|&c| c != s).collect();
            v.sort_unstable();
            v.dedup();
            // generate scopes (their parent is this scope) before instances
            v.sort_by_key(|&c| (self.scopes[c].parent != Some(s), c));
            children[s] = v;
        }
        let mut rank = vec![usize::MAX; n];
        let mut next = 0;
        let mut tops: Vec<(&String, &usize)> = self.tops.iter().collect();
        tops.sort();
        fn visit(s: usize, children: &[Vec<usize>], rank: &mut [usize], next: &mut usize) {
            if rank[s] != usize::MAX {
                return;
            }
            rank[s] = usize::MAX - 1;
            for &c in &children[s] {
                visit(c, children, rank, next);
            }
            rank[s] = *next;
            *next += 1;
        }
        for (_, &t) in tops {
            visit(t, &children, &mut rank, &mut next);
        }
        let key = |p: usize| -> usize {
            match self.proc_scope.get(p) {
                Some(&s) if s < n => rank[s].saturating_add(1),
                // continuous assignments, port connections: first, in creation order
                Some(&s) if s == usize::MAX - 1 => 0,
                _ => usize::MAX,
            }
        };
        let mut order: Vec<u32> = (0..self.prog.procs.len() as u32).collect();
        order.sort_by_key(|&p| (key(p as usize), p));
        self.prog.start_order = order;
    }

    /// A package (or `$unit`) scope.
    pub(crate) fn is_pkg_scope(&self, sid: usize) -> bool {
        self.pkg_scopes.values().any(|&p| p == sid)
    }

    /// The signal a view stands for (the signal itself if it is not a view).
    pub(crate) fn canon(&self, mut s: u32) -> u32 {
        while let Some(&b) = self.view_of.get(&s) {
            s = b;
        }
        s
    }

    /// A new name for signal `base` with its own declared range and sign.
    fn new_view(&mut self, base: u32, name: String, msb: i64, lsb: i64, signed: bool) -> u32 {
        let base = self.canon(base);
        let mut info = self.sig(base).clone();
        info.name = name.clone();
        info.msb = msb;
        info.lsb = lsb;
        info.signed = signed;
        let id = self.prog.signals.len() as u32;
        self.prog.signals.push(info);
        self.prog.names.push((name, id));
        self.view_of.insert(id, base);
        id
    }

    /// After code generation: instructions, event controls and names refer to the signals the
    /// views stand for.
    fn resolve_views(&mut self) {
        if self.view_of.is_empty() {
            return;
        }
        let canon: Vec<u32> = (0..self.prog.signals.len() as u32).map(|s| self.canon(s)).collect();
        for p in &mut self.prog.procs {
            for ins in &mut p.code {
                rt::instr_sigs_mut(ins, &mut |s| *s = canon[*s as usize]);
            }
        }
        for c in &mut self.prog.ctls {
            for t in &mut c.triggers {
                t.0 = canon[t.0 as usize];
            }
        }
        for n in &mut self.prog.names {
            n.1 = canon[n.1 as usize];
        }
        for (&v, _) in &self.view_of {
            if self.prog.signals[v as usize].forced {
                self.prog.signals[canon[v as usize] as usize].forced = true;
            }
        }
    }

    pub(crate) fn sig(&self, id: u32) -> &SignalInfo {
        &self.prog.signals[id as usize]
    }

    // ------------------------------------------------------------------ constant evaluation
    pub(crate) fn const_eval(&mut self, e: &Expr, sid: usize) -> R<Const> {
        if let Expr::Num(n) = e {
            return Const::from_number(n).or_else(|m| self.err(m));
        }
        if let Expr::Real(r) = e {
            return Ok(Const::from_i64(r.round() as i64, 64, false));
        }
        self.pb.push(PState { proc: NONE, ..Default::default() });
        let r = self.self_expr(e, sid);
        let r = r.and_then(|x| self.finish_subs().map(|_| x));
        let st = self.pb.pop().unwrap();
        let (slot, w, s) = r?;
        if !st.code.is_empty() {
            if let Err(m) = rt::vm::eval_const(&st.code, &mut self.prog.mem, &self.prog.signals) {
                return self.err(m);
            }
        }
        Ok(self.read_const(slot, w, s))
    }

    /// Value of a constant expression without emitting code or errors (None if not evaluable).
    pub(crate) fn const_value(&self, e: &Expr, sid: usize) -> Option<i64> {
        match e {
            Expr::Num(n) => Const::from_number(n).ok()?.to_i64(),
            Expr::Ident(n) => match self.lookup(n, sid)? {
                Sym::Const(c) => c.to_i64(),
                _ => None,
            },
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.const_value(a, sid)?, self.const_value(b, sid)?);
                match op {
                    BinOp::Add => Some(a + b),
                    BinOp::Sub => Some(a - b),
                    BinOp::Mul => Some(a * b),
                    BinOp::Div if b != 0 => Some(a / b),
                    BinOp::Mod if b != 0 => Some(a % b),
                    BinOp::And => Some(a & b),
                    BinOp::Or => Some(a | b),
                    BinOp::Xor => Some(a ^ b),
                    BinOp::Shl if (0..63).contains(&b) => Some(a << b),
                    BinOp::Shr if (0..63).contains(&b) => Some(a >> b),
                    _ => None,
                }
            }
            Expr::Unary(UnOp::Neg, a) => Some(-self.const_value(a, sid)?),
            _ => None,
        }
    }

    /// A constant index: None when it has x/z bits (such a select reads x, writes nothing).
    pub(crate) fn const_index(&mut self, e: &Expr, sid: usize) -> R<Option<i64>> {
        Ok(self.const_eval(e, sid)?.to_i64())
    }

    pub(crate) fn const_int(&mut self, e: &Expr, sid: usize) -> R<i64> {
        let c = self.const_eval(e, sid)?;
        match c.to_i64() {
            Some(v) => Ok(v),
            None => self.err("constant expression has x/z bits"),
        }
    }

    pub(crate) fn range_vals(&mut self, r: &Option<Range>, sid: usize) -> R<(i64, i64)> {
        match r {
            None => Ok((0, 0)),
            Some((a, b)) => Ok((self.const_int(a, sid)?, self.const_int(b, sid)?)),
        }
    }

    // ------------------------------------------------------------------ elaboration
    fn instantiate(
        &mut self,
        m: &'a Module,
        path: String,
        overrides: &[(Option<String>, PVal)],
        conns: Option<(&'a [Conn], usize)>,
    ) -> R<usize> {
        let saved_file = std::mem::replace(&mut self.file, m.file.clone());
        let sid = self.new_scope(path.clone(), None, m.file.clone());
        self.scopes[sid].imports = m.imports.iter().filter_map(|n| self.pkg_scopes.get(n).copied()).collect();
        self.scopes[sid].tunit = m.timescale.map(|t| t.0).unwrap_or(0);
        self.scopes[sid].tprec = m.timescale.map(|t| t.1).unwrap_or(0);
        self.scopes[sid].nettype = m.default_nettype.clone();
        self.scopes[sid].uncdrive = m.unconnected_drive;
        for it in &m.items {
            if let Item::Subroutine(f) = it {
                self.scopes[sid].funcs.insert(f.name.clone(), f);
            }
        }
        self.eval_params(m, overrides, sid)?;
        // port connections by name
        let mut port_conns: HashMap<String, Option<Expr>> = HashMap::new();
        if let Some((cs, _)) = conns {
            let mut idx = 0;
            for c in cs {
                match c {
                    Conn::Positional(e) => {
                        if let Some(pn) = m.ports.get(idx) {
                            port_conns.insert(pn.clone(), e.clone());
                        } else if e.is_some() {
                            return self.err(format!("too many port connections for module '{}'", m.name));
                        }
                        idx += 1;
                    }
                    Conn::Named(n, e) => {
                        if !m.ports.contains(n) {
                            return self.err(format!("module '{}' has no port '{n}'", m.name));
                        }
                        port_conns.insert(n.clone(), e.clone());
                    }
                    Conn::Wildcard => {
                        for p in &m.ports {
                            port_conns.entry(p.clone()).or_insert_with(|| Some(Expr::Ident(p.clone())));
                        }
                    }
                }
            }
        }
        let parent_scope = conns.map(|c| c.1);
        let inst_line = self.line;
        // interface ports: the port's name stands for the connected interface instance
        for (pn, iface) in &m.iface_ports {
            let target = match (port_conns.get(pn), parent_scope) {
                (Some(Some(e)), Some(ps)) => {
                    let parts: Vec<String> = match e {
                        Expr::Ident(n) => vec![n.clone()],
                        Expr::Hier(p) => p.clone(),
                        _ => vec![],
                    };
                    // `bus.master`: a modport of the instance
                    self.hier_scope(&parts, ps).or_else(|| {
                        (parts.len() > 1).then(|| self.hier_scope(&parts[..parts.len() - 1], ps)).flatten()
                    })
                }
                _ => None,
            };
            let Some(target) = target else {
                return self.err(format!("interface port '{pn}' of '{}' must be connected to an instance of '{iface}'", m.name));
            };
            self.scopes[sid].children.insert(pn.clone(), target);
        }
        self.elab_items(&m.items, sid, true, &port_conns, parent_scope)?;
        if let Some(ps) = parent_scope {
            // after the instance's own children, as Icarus orders them
            self.port_width_warnings(m, sid, ps, &port_conns, &saved_file, inst_line)?;
        }
        self.file = saved_file;
        self.line = inst_line;
        Ok(sid)
    }

    fn push_job(&mut self, j: Job<'a>) {
        if let (Some(st), Job::Assign { .. }) = (self.cur_strength, &j) {
            self.job_strength.insert(self.jobs.len(), st);
        }
        self.jobs.push(j);
    }

    /// Icarus's warnings for port connections whose width differs from the port's.
    fn port_width_warnings(
        &mut self,
        m: &'a Module,
        sid: usize,
        ps: usize,
        port_conns: &HashMap<String, Option<Expr>>,
        pfile: &str,
        line: u32,
    ) -> R<()> {
        let mut dirs: HashMap<&str, Dir> = HashMap::new();
        for it in &m.items {
            if let Item::Decl(d) = it {
                if let Some(dir) = d.dir {
                    for dl in &d.names {
                        dirs.insert(dl.name.as_str(), dir);
                    }
                }
            }
        }
        for (k, pn) in m.ports.iter().enumerate() {
            let Some(Some(e)) = port_conns.get(pn) else { continue };
            let Some(Sym::Sig(s)) = self.scopes[sid].syms.get(pn).cloned() else { continue };
            let info = self.sig(s);
            if info.real || info.depth != 0 || self.is_real(e, ps) {
                continue;
            }
            let pw = info.width;
            let ew = self.ty(e, ps)?.0;
            if pw == ew {
                continue;
            }
            let n = pw.abs_diff(ew);
            let detail = match (dirs.get(pn.as_str()), ew < pw) {
                (Some(Dir::Inout), true) => format!("Leaving {n} high bits of the port unconnected."),
                (Some(Dir::Inout), false) => format!("Leaving {n} high bits of the expression dangling."),
                (Some(Dir::Input), false) => format!("Pruning {n} high bits of the expression."),
                (_, true) => format!("Padding {n} high bits of the port."),
                (_, false) => format!("Padding {n} high bits of the expression."),
            };
            let w = format!(
                "{pfile}:{line}: warning: Port {} ({pn}) of module {} expects {pw} bit(s), given {ew}.\n{pfile}:{line}:        : {detail}",
                k + 1,
                m.name
            );
            if self.warned.insert(w.clone()) {
                self.warnings.push(w);
            }
        }
        Ok(())
    }

    /// Evaluate the parameters of module `m` (with overrides) into scope `sid`, in order.
    fn eval_params(&mut self, m: &'a Module, overrides: &[(Option<String>, PVal)], sid: usize) -> R<()> {
        let mut pos = 0usize;
        for it in &m.items {
            if let Item::TypeParam { local, name, default, line } = it {
                self.line = *line;
                let mut ov = None;
                if !local {
                    ov = overrides.iter().find(|(n, _)| n.as_deref() == Some(name.as_str())).map(|x| x.1.clone());
                    if ov.is_none() {
                        let positional: Vec<&PVal> = overrides.iter().filter(|(n, _)| n.is_none()).map(|(_, c)| c).collect();
                        if pos < positional.len() {
                            ov = Some(positional[pos].clone());
                        }
                    }
                    pos += 1;
                }
                let t = match (ov, default) {
                    (Some(PVal::Type(t)), _) => t,
                    (Some(_), _) => return self.err(format!("type parameter '{name}' needs a type")),
                    (None, Some(d)) => Rc::new(self.resolve_type(d, sid)?),
                    (None, None) => return self.err(format!("type parameter '{name}' has no type")),
                };
                self.scopes[sid].syms.insert(name.clone(), Sym::Type(t));
                continue;
            }
            if let Item::Param(p) = it {
                self.line = p.line;
                let mut ov = None;
                if !p.local {
                    ov = overrides.iter().find(|(n, _)| n.as_deref() == Some(p.name.as_str())).map(|x| x.1.clone());
                    if ov.is_none() {
                        let positional: Vec<&PVal> =
                            overrides.iter().filter(|(n, _)| n.is_none()).map(|(_, c)| c).collect();
                        if pos < positional.len() {
                            ov = Some(positional[pos].clone());
                        }
                    }
                    pos += 1;
                }
                if !p.local {
                    let key = format!("{}.{}", self.scopes[sid].name, p.name);
                    if let Some(d) = self.defparams.get(&key) {
                        ov = Some(d.clone());
                    }
                    self.params_done.insert(key);
                }
                let v = self.param_value(p, ov, sid)?;
                self.scopes[sid].syms.insert(p.name.clone(), v);
                if p.range.is_some() && !p.real {
                    let r = self.range_vals(&p.range, sid)?;
                    self.scopes[sid].param_ranges.insert(p.name.clone(), r);
                }
            }
        }
        Ok(())
    }

    /// Record the defparams among `items` (in scope `sid`) by the full path of their target.
    fn register_defparams(&mut self, items: &'a [Item], sid: usize) -> R<()> {
        for it in items {
            let Item::Defparam { target, value, line } = it else { continue };
            self.line = *line;
            // generate-loop indices in the path (`Loop[i].m.p`)
            let target = &match self.hier_parts(target, sid) {
                Some(t) => t,
                None => return self.err(format!("defparam: cannot evaluate '{}'", target.join("."))),
            };
            let v = self.pval(value, sid)?;
            let rel = target.join(".");
            // upwards: the first component names this scope or an enclosing one
            let mut base = None;
            let mut s = Some(sid);
            while let Some(x) = s {
                let name = &self.scopes[x].name;
                if name.rsplit('.').next() == Some(target[0].as_str()) {
                    base = Some(match name.rfind('.') {
                        Some(i) => format!("{}.{rel}", &name[..i]),
                        None => rel.clone(),
                    });
                    break;
                }
                s = self.scopes[x].parent;
            }
            // relative: to this scope if it declares the first name (an instance or generate
            // block), else to the nearest enclosing scope that has it, else to the module
            let first = target[0].split('[').next().unwrap_or("");
            let declares = |its: &[Item]| {
                its.iter().any(|i| match i {
                    Item::Instances { insts, .. } => insts.iter().any(|x| x.name == first),
                    Item::GenFor { name: Some(name), .. } | Item::GenBlock { name: Some(name), .. } => name == first,
                    Item::GenIf { .. } | Item::GenCase { .. } => true,
                    _ => false,
                })
            };
            let rel_base = if declares(items) {
                sid
            } else {
                let mut x = sid;
                loop {
                    if self.scopes[x].children.contains_key(&target[0]) {
                        break x;
                    }
                    match self.scopes[x].parent {
                        Some(p) => x = p,
                        None => break x,
                    }
                }
            };
            let key = match base {
                Some(k) => k,
                None if self.top_names.contains(&target[0]) && target.len() > 1 => rel,
                None => format!("{}.{rel}", self.scopes[rel_base].name),
            };
            if !self.restarted && self.params_done.contains(&key) && !self.defparams.contains_key(&key) {
                self.late_defparam = true;
            }
            self.defparams.insert(key, v);
        }
        Ok(())
    }

    /// Port widths of module `m` with these parameter overrides (evaluated in a scratch scope).
    fn port_widths(&mut self, m: &'a Module, overrides: &[(Option<String>, PVal)]) -> R<HashMap<String, u32>> {
        let saved = std::mem::replace(&mut self.file, m.file.clone());
        let sid = self.new_scope(format!("$ports.{}", m.name), None, m.file.clone());
        for it in &m.items {
            if let Item::Subroutine(f) = it {
                self.scopes[sid].funcs.insert(f.name.clone(), f);
            }
        }
        self.eval_params(m, overrides, sid)?;
        let mut w = HashMap::new();
        for it in &m.items {
            if let Item::Decl(d) = it {
                let pw = match d.kind {
                    NetKind::Integer => Some(32),
                    NetKind::Int(n) if d.range.is_none() => Some(n),
                    NetKind::Real | NetKind::Time => Some(64),
                    _ => match &d.range {
                        Some(r) => {
                            let (a, b) = self.range_vals(&Some(r.clone()), sid)?;
                            Some(((a - b).abs() + 1) as u32)
                        }
                        None => None,
                    },
                };
                for dl in &d.names {
                    if m.ports.contains(&dl.name) {
                        match pw {
                            Some(p) => {
                                w.insert(dl.name.clone(), p);
                            }
                            None => {
                                w.entry(dl.name.clone()).or_insert(1);
                            }
                        }
                    }
                }
            }
        }
        self.file = saved;
        Ok(w)
    }

    /// The part of terminal `e` that instance `k` (from the left) of an `n`-instance array gets:
    /// slices of `pw` bits, the leftmost instance taking the most significant one.
    fn array_slice(&mut self, e: &Expr, k: u32, n: u32, pw: u32, sid: usize) -> R<Expr> {
        self.slice_bits(e, ((n - 1 - k) * pw) as i64, pw as i64, sid)
    }

    /// Bits `[off, off+w)` (counted from the LSB) of `e`, as an expression that is still an
    /// lvalue when `e` is one.
    fn slice_bits(&mut self, e: &Expr, off: i64, w: i64, sid: usize) -> R<Expr> {
        let num = |v: i64| -> Expr {
            let e = Expr::Num(ferrooxide_front::parser::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, Box::new(e))
            } else {
                e
            }
        };
        // declared index range of a plain signal
        let decl = |me: &Self, b: &Expr| -> Option<(i64, i64)> {
            let sym = match b {
                Expr::Ident(nm) => me.lookup(nm, sid),
                Expr::Hier(p) => me.lookup_hier(p, sid),
                _ => None,
            };
            match sym {
                Some(Sym::Sig(s)) if me.sig(s).depth == 0 => Some((me.sig(s).msb, me.sig(s).lsb)),
                _ => None,
            }
        };
        let off_of = |(msb, lsb): (i64, i64), i: i64| if msb >= lsb { i - lsb } else { lsb - i };
        match e {
            Expr::Range(b, m, l) if decl(self, b).is_some() => {
                if let (Some(m), Some(l)) = (self.const_value(m, sid), self.const_value(l, sid)) {
                    let d = decl(self, b).unwrap();
                    let lo = off_of(d, m).min(off_of(d, l));
                    return self.slice_bits(&b.clone(), lo + off, w, sid);
                }
            }
            Expr::Index(b, i) if decl(self, b).is_some() && off == 0 && w == 1 => return Ok(e.clone()),
            Expr::Concat(parts) => {
                // walk from the LSB part; collect the pieces that overlap the slice
                let mut pieces = Vec::new();
                let mut base = 0i64;
                for p in parts.iter().rev() {
                    let pw = self.ty(p, sid)?.0 as i64;
                    let (s0, s1) = (off.max(base), (off + w).min(base + pw));
                    if s0 < s1 {
                        let piece =
                            if s0 == base && s1 == base + pw { p.clone() } else { self.slice_bits(p, s0 - base, s1 - s0, sid)? };
                        pieces.push(piece);
                    }
                    base += pw;
                }
                pieces.reverse();
                return Ok(if pieces.len() == 1 { pieces.pop().unwrap() } else { Expr::Concat(pieces) });
            }
            _ => {}
        }
        let d = decl(self, e).unwrap_or((self.ty(e, sid)?.0 as i64 - 1, 0));
        let idx = |o: i64| if d.0 >= d.1 { d.1 + o } else { d.1 - o };
        Ok(if w == 1 {
            Expr::Index(Box::new(e.clone()), Box::new(num(idx(off))))
        } else {
            Expr::Range(Box::new(e.clone()), Box::new(num(idx(off + w - 1))), Box::new(num(idx(off))))
        })
    }

    /// The value of a parameter (from its override if any): integer or real.
    fn param_value(&mut self, p: &ParamDecl, ov: Option<PVal>, sid: usize) -> R<Sym> {
        let untyped = !p.integer && p.range.is_none() && !p.signed;
        let v = match ov {
            Some(v) => v,
            None => {
                if self.is_real(&p.value, sid) {
                    PVal::Real(self.const_real(&p.value, sid)?)
                } else {
                    PVal::Int(self.const_eval(&p.value, sid)?)
                }
            }
        };
        if matches!(v, PVal::Type(_)) {
            return self.err(format!("parameter '{}' needs a value, not a type", p.name));
        }
        if p.real || (untyped && matches!(v, PVal::Real(_))) {
            let r = match v {
                PVal::Real(r) => r,
                PVal::Int(c) => c.to_i64().unwrap_or(0) as f64,
                PVal::Type(_) => unreachable!(),
            };
            return Ok(Sym::Real(r));
        }
        let c = match v {
            PVal::Int(c) => c,
            PVal::Real(r) => Const::from_i64(r.round() as i64, 32, true),
            PVal::Type(_) => unreachable!(),
        };
        Ok(Sym::Const(self.param_type(p, c, sid)?))
    }

    /// evaluate a parameter override in the instantiating scope
    fn pval(&mut self, e: &Expr, sid: usize) -> R<PVal> {
        if let Expr::Type(t) = e {
            return Ok(PVal::Type(Rc::new(self.resolve_type(t, sid)?)));
        }
        if self.is_real(e, sid) {
            Ok(PVal::Real(self.const_real(e, sid)?))
        } else {
            Ok(PVal::Int(self.const_eval(e, sid)?))
        }
    }

    /// A data type with its dimensions evaluated in scope `sid` (type parameters replaced by
    /// their types there).
    pub(crate) fn resolve_type(&mut self, t: &SvType, sid: usize) -> R<SvType> {
        let lit = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::parser::dec(v.unsigned_abs()));
            if v < 0 { Expr::Unary(UnOp::Neg, Box::new(n)) } else { n }
        };
        let mut dims = Vec::new();
        for (a, b) in &t.dims {
            dims.push((lit(self.const_int(a, sid)?), lit(self.const_int(b, sid)?)));
        }
        let mut unpacked = Vec::new();
        for (a, b) in &t.unpacked {
            unpacked.push((lit(self.const_int(a, sid)?), lit(self.const_int(b, sid)?)));
        }
        if let Some(p) = &t.param {
            let Some(Sym::Type(base)) = self.lookup(p, sid) else {
                return self.err(format!("'{p}' is not a type"));
            };
            let mut r = (*base).clone();
            if !dims.is_empty() {
                r.int_kind = false;
                dims.extend(r.dims);
                r.dims = dims;
            }
            unpacked.extend(r.unpacked);
            r.unpacked = unpacked;
            return Ok(r);
        }
        let mut r = t.clone();
        r.dims = dims;
        r.unpacked = unpacked;
        if let Some(f) = &t.fields {
            let mut f2 = (**f).clone();
            for (_, ft) in &mut f2.fields {
                *ft = self.resolve_type(ft, sid)?;
            }
            r.fields = Some(Rc::new(f2));
        }
        Ok(r)
    }

    fn param_type(&mut self, p: &ParamDecl, v: Const, sid: usize) -> R<Const> {
        if p.integer {
            return Ok(v.resize(32, v.signed).with_sign(true));
        }
        if p.range.is_some() {
            let (a, b) = self.range_vals(&p.range, sid)?;
            let w = ((a - b).abs() + 1) as u32;
            return Ok(v.resize(w, v.signed).with_sign(p.signed));
        }
        if p.signed {
            return Ok(v.with_sign(true));
        }
        Ok(v)
    }

    fn elab_items(
        &mut self,
        items: &'a [Item],
        sid: usize,
        module_level: bool,
        port_conns: &HashMap<String, Option<Expr>>,
        parent_scope: Option<usize>,
    ) -> R<()> {
        // 1. subroutines and local parameters of generate scopes
        for it in items {
            if let Item::Subroutine(f) = it {
                self.scopes[sid].funcs.insert(f.name.clone(), f);
            }
        }
        if !module_level {
            for it in items {
                if let Item::TypeParam { name, default: Some(d), line, .. } = it {
                    self.line = *line;
                    let t = Rc::new(self.resolve_type(d, sid)?);
                    self.scopes[sid].syms.insert(name.clone(), Sym::Type(t));
                }
                if let Item::Param(p) = it {
                    self.line = p.line;
                    let v = self.param_value(p, None, sid)?;
                    self.scopes[sid].syms.insert(p.name.clone(), v);
                if p.range.is_some() && !p.real {
                    let r = self.range_vals(&p.range, sid)?;
                    self.scopes[sid].param_ranges.insert(p.name.clone(), r);
                }
                }
            }
        }
        self.register_defparams(items, sid)?;
        // 2. declarations (merged by name: `output [3:0] q; reg [3:0] q;`)
        struct Merged<'d> {
            dir: Option<Dir>,
            kind: NetKind,
            net: NetType,
            real_net: bool,
            strength: Option<(u8, u8)>,
            delay: Option<&'d Expr>,
            signed: bool,
            range: Option<&'d Range>,
            packed: &'d [Range],
            dims: &'d [Range],
            init: Option<&'d Expr>,
            /// declared with this type parameter
            tparam: Option<&'d str>,
            line: u32,
        }
        let mut order: Vec<String> = Vec::new();
        let mut merged: HashMap<String, Merged> = HashMap::new();
        for it in items {
            if let Item::Decl(d) = it {
                if d.kind == NetKind::Genvar {
                    continue;
                }
                for dl in &d.names {
                    match merged.get_mut(&dl.name) {
                        Some(mg) => {
                            if d.dir.is_some() {
                                mg.dir = d.dir;
                            }
                            if d.explicit_kind && d.kind != NetKind::Wire {
                                mg.kind = d.kind;
                            }
                            if d.net != NetType::Wire {
                                mg.net = d.net;
                            }
                            mg.real_net |= d.real_net;
                            if d.strength.is_some() {
                                mg.strength = d.strength;
                            }
                            if d.delay.is_some() {
                                mg.delay = d.delay.as_ref();
                            }
                            mg.signed |= d.signed;
                            if d.range.is_some() {
                                mg.range = d.range.as_ref();
                                mg.packed = &d.packed;
                            }
                            if !dl.dims.is_empty() {
                                mg.dims = &dl.dims;
                            }
                            if dl.init.is_some() {
                                mg.init = dl.init.as_ref();
                            }
                            if d.type_param.is_some() {
                                mg.tparam = d.type_param.as_deref();
                            }
                        }
                        None => {
                            order.push(dl.name.clone());
                            merged.insert(
                                dl.name.clone(),
                                Merged {
                                    dir: d.dir,
                                    kind: d.kind,
                                    net: d.net,
                                    real_net: d.real_net,
                                    strength: d.strength,
                                    delay: d.delay.as_ref(),
                                    signed: d.signed,
                                    range: d.range.as_ref(),
                                    packed: &d.packed,
                                    dims: &dl.dims,
                                    init: dl.init.as_ref(),
                                    tparam: d.type_param.as_deref(),
                                    line: dl.line,
                                },
                            );
                        }
                    }
                }
            }
        }
        let mut port_dirs: HashMap<String, Dir> = HashMap::new();
        for name in order {
            let mut mg = merged.remove(&name).unwrap();
            self.line = mg.line;
            if let Some(d) = mg.dir {
                port_dirs.insert(name.clone(), d);
            }
            // the range and further packed dimensions, evaluated
            let mut range_v = None;
            let mut packed_v = Vec::new();
            if let Some(tp) = mg.tparam {
                let Some(Sym::Type(t)) = self.lookup(tp, sid) else {
                    return self.err(format!("'{tp}' is not a type"));
                };
                let (k, sg, r, p) = t.decl_parts();
                // (an input port of a logic type is a net)
                mg.kind = if k == NetKind::Reg && matches!(mg.dir, Some(Dir::Input | Dir::Inout)) { NetKind::Wire } else { k };
                mg.signed = sg;
                if let Some((a, b)) = r {
                    range_v = Some((self.const_int(&a, sid)?, self.const_int(&b, sid)?));
                }
                for (a, b) in &p {
                    packed_v.push((self.const_int(a, sid)?, self.const_int(b, sid)?));
                }
            } else {
                if let Some((a, b)) = mg.range {
                    range_v = Some((self.const_int(a, sid)?, self.const_int(b, sid)?));
                }
                for (a, b) in mg.packed {
                    packed_v.push((self.const_int(a, sid)?, self.const_int(b, sid)?));
                }
            }
            let (mut msb, mut lsb) = range_v.unwrap_or((0, 0));
            // packed dimensions: one flat vector [W-1:0], the dimensions kept for selects
            let mut pk: Option<Vec<(i64, i64)>> = None;
            if !packed_v.is_empty() {
                let mut dims = vec![(msb, lsb)];
                dims.extend(packed_v.iter().copied());
                let w: i64 = dims.iter().map(|(a, b)| (a - b).abs() + 1).product();
                (msb, lsb) = (w - 1, 0);
                pk = Some(dims);
            }
            let mut signed = mg.signed;
            let kind = match mg.kind {
                NetKind::Wire => SigKind::Wire,
                NetKind::Integer => {
                    (msb, lsb) = (31, 0);
                    signed = true;
                    SigKind::Integer
                }
                NetKind::Real if mg.real_net => {
                    (msb, lsb) = (63, 0);
                    SigKind::Wire
                }
                NetKind::Time | NetKind::Real => {
                    (msb, lsb) = (63, 0);
                    SigKind::Reg
                }
                NetKind::Event => SigKind::Reg,
                NetKind::Int(n) => {
                    if range_v.is_none() {
                        (msb, lsb) = (n as i64 - 1, 0);
                    }
                    SigKind::Integer
                }
                _ => SigKind::Reg,
            };
            let is_real = mg.kind == NetKind::Real;
            let is_event = mg.kind == NetKind::Event;
            let width = ((msb - lsb).abs() + 1) as u32;
            // a multi-dimensional array is stored flat (row-major); `md` keeps its dimensions
            let mut md: Option<Vec<(i64, u32)>> = None;
            let (depth, arr_lo) = match mg.dims.len() {
                0 => (0, 0),
                1 => {
                    let (a, b) = (self.const_int(&mg.dims[0].0, sid)?, self.const_int(&mg.dims[0].1, sid)?);
                    (((a - b).abs() + 1) as u32, a.min(b))
                }
                _ => {
                    let mut dims = Vec::new();
                    let mut total: u64 = 1;
                    for (x, y) in mg.dims {
                        let (a, b) = (self.const_int(x, sid)?, self.const_int(y, sid)?);
                        let n = ((a - b).abs() + 1) as u32;
                        total *= n as u64;
                        dims.push((a.min(b), n));
                    }
                    if total > u32::MAX as u64 {
                        return self.err(format!("array '{name}' is too large"));
                    }
                    md = Some(dims);
                    (total as u32, 0)
                }
            };
            let full = format!("{}.{}", self.scopes[sid].name, name);
            // port aliasing
            if let (Some(dir), Some(ps)) = (mg.dir, parent_scope) {
                if module_level {
                    if let Some(Some(conn)) = port_conns.get(&name) {
                        let mut aliased = None;
                        if depth == 0 {
                            if let Expr::Ident(pn) = conn {
                                let sym = match self.lookup(pn, ps) {
                                    Some(s) => Some(s),
                                    None => {
                                        // implicit net in the parent
                                        let s = self.implicit_net(pn, width, ps)?;
                                        let _ = (msb, lsb);
                                        Some(Sym::Sig(s))
                                    }
                                };
                                if let Some(Sym::Sig(s)) = sym {
                                    let si = self.sig(s);
                                    if si.width == width && si.depth == 0 {
                                        aliased = Some(s);
                                    }
                                }
                            }
                        }
                        if depth == 0 && aliased.is_none() {
                            aliased = self.elem_port_alias(conn, ps, width, &full);
                        }
                        if let Some(s) = aliased {
                            let si = self.sig(s);
                            let s = if (si.msb, si.lsb, si.signed) != (msb, lsb, signed) || pk.is_some() {
                                // own range / sign / packed dimensions: a view
                                let v = self.new_view(s, full.clone(), msb, lsb, signed);
                                self.prog.names.pop();
                                if let Some(d) = pk.take() {
                                    self.packed_dims.insert(v, d);
                                    self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(v));
                                }
                                v
                            } else {
                                s
                            };
                            let s0 = self.canon(s);
                            if kind != SigKind::Wire && self.sig(s0).kind == SigKind::Wire {
                                // a variable output collapsed onto a net: starts as x, not z
                                let (slot, w) = (self.sig(s0).slot as usize, self.sig(s0).width);
                                let n = rt::nw(w);
                                for i in 0..n {
                                    self.prog.mem[slot + i] = self.prog.mem[slot + n + i];
                                }
                                self.prog.signals[s0 as usize].kind = SigKind::Reg;
                                self.prog.signals[s as usize].kind = SigKind::Reg;
                            }
                            self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
                            self.prog.names.push((full, s));
                            continue;
                        }
                        let s = self.new_signal(full, width, signed, depth, msb, lsb, arr_lo, kind);
                        if is_real {
                            self.mark_real(s);
                        }
                        self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
                        let line = self.line;
                        match dir {
                            Dir::Input => {
                                self.jobs.push(Job::PortIn { child_sig: s, expr: conn.clone(), scope: ps, line })
                            }
                            Dir::Output => {
                                self.jobs.push(Job::PortOut { lhs: conn.clone(), child_sig: s, scope: ps, line })
                            }
                            Dir::Inout if self.link_exprs(&Expr::Ident(name.clone()), sid, conn, ps, None) => {}
                            Dir::Inout => self.warn(format!(
                                "inout port '{name}' connected to an expression: connection ignored"
                            )),
                        }
                        continue;
                    }
                }
            }
            let s = self.new_signal(full, width, signed, depth, msb, lsb, arr_lo, kind);
            if let Some(d) = pk.take() {
                self.packed_dims.insert(s, d);
                self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(s));
            }
            if let Some(d) = md.take() {
                self.md_dims.insert(s, d);
                // the flat storage, for index chains already turned into one index
                self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(s));
            }
            if is_real {
                self.mark_real(s);
            }
            if is_event {
                self.zero_init(s);
            }
            self.mark_two_state(s, mg.kind);
            if mg.net != NetType::Wire {
                self.net_types.insert(s, mg.net);
                if matches!(mg.net, NetType::Supply0 | NetType::Supply1) {
                    let d = if mg.net == NetType::Supply1 { "1" } else { "0" };
                    let rhs = Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: d.into(), fill: true });
                    let line = self.line;
                    self.cur_strength = Some((7, 7));
                    self.push_job(Job::Assign { lhs: Expr::Ident(name.clone()), rhs, delay: None, scope: sid, line });
                    self.cur_strength = None;
                }
            }
            self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
            // `unconnected_drive: an unconnected input port is pulled
            if let (Some(Dir::Input), true, Some(up)) = (mg.dir, module_level, self.scopes[sid].uncdrive) {
                if parent_scope.is_some() && !matches!(port_conns.get(&name), Some(Some(_))) && depth == 0 {
                    self.pulls.entry(s).or_default().push((0, width, up));
                }
            }
            if let Some(init) = mg.init {
                if kind == SigKind::Wire {
                    self.cur_strength = mg.strength;
                    self.push_job(Job::Assign {
                        lhs: Expr::Ident(name.clone()),
                        rhs: init.clone(),
                        delay: mg.delay.cloned(),
                        scope: sid,
                        line: self.line,
                    });
                    self.cur_strength = None;
                } else if is_real {
                    let r = self.const_real(init, sid)?;
                    let slot = self.sig(s).slot as usize;
                    self.prog.mem[slot] = r.to_bits();
                    self.prog.mem[slot + 1] = 0;
                    self.init_touch.push(s);
                } else {
                    if depth == 0 {
                        self.init_touch.push(s);
                    }
                    let c = self.const_eval(init, sid)?;
                    let c = c.resize(width, c.signed);
                    let slot = self.sig(s).slot as usize;
                    let n = rt::nw(width);
                    for i in 0..n {
                        self.prog.mem[slot + i] = c.a[i];
                        self.prog.mem[slot + n + i] = c.b[i];
                    }
                }
            }
        }
        // 3b. implicit nets: undeclared names on the left of continuous assignments and as gate
        // terminals become 1-bit wires
        let mut implicit: Vec<&String> = Vec::new();
        fn lhs_names<'e>(e: &'e Expr, out: &mut Vec<&'e String>) {
            match e {
                Expr::Ident(n) => out.push(n),
                Expr::Concat(v) => v.iter().for_each(|x| lhs_names(x, out)),
                _ => {}
            }
        }
        for it in items {
            match it {
                Item::ContAssign { lhs, .. } => lhs_names(lhs, &mut implicit),
                Item::Gate { terms, .. } => terms.iter().for_each(|t| lhs_names(t, &mut implicit)),
                _ => {}
            }
        }
        for n in implicit {
            if self.lookup(n, sid).is_none() {
                self.implicit_net(n, 1, sid)?;
            }
        }
        // 3b'. specify path delays on output ports
        if module_level && self.dialect.specify {
            self.path_delays(items, sid, &port_dirs)?;
        }
        // 3c. static tasks and functions, so hierarchical names can reach their variables
        for it in items {
            if let Item::Subroutine(f) = it {
                if !f.automatic {
                    self.func_instance(f, sid, &f.name)?;
                }
            }
        }
        // 4. everything else; interface instances first, so that modules can be connected to
        // interfaces declared after them
        let mut genblk = 0;
        let is_iface = |it: &Item| matches!(it, Item::Instances { module, .. } if self.mods.get(module.as_str()).is_some_and(|m| m.interface));
        let order: Vec<&'a Item> = items.iter().filter(|it| is_iface(it)).chain(items.iter().filter(|it| !is_iface(it))).collect();
        for it in order {
            match it {
                Item::Decl(_) | Item::Param(_) | Item::TypeParam { .. } | Item::Subroutine(_) | Item::Defparam { .. } | Item::SpecPath { .. } => {}
                Item::ContAssign { delay, lhs, rhs, strength, line } => {
                    self.cur_strength = *strength;
                    self.push_job(Job::Assign {
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                        delay: delay.clone(),
                        scope: sid,
                        line: *line,
                    });
                    self.cur_strength = None;
                }
                Item::Process { kind, body, line } => {
                    self.jobs.push(Job::Proc { kind: *kind, body, scope: sid, line: *line });
                }
                Item::Instances { module, params, insts, line } => {
                    self.line = *line;
                    let m = match self.mods.get(module.as_str()) {
                        Some(m) => *m,
                        None => return self.err(format!("unknown module '{module}'")),
                    };
                    if m.udp.is_some() {
                        // a user-defined primitive: positional terminals, an optional delay
                        let delay = match params.first() {
                            Some(Conn::Positional(Some(e))) => Some(e.clone()),
                            _ => None,
                        };
                        for inst in insts {
                            let mut terms = Vec::new();
                            for c in &inst.conns {
                                match c {
                                    Conn::Positional(Some(e)) | Conn::Named(_, Some(e)) => terms.push(e.clone()),
                                    _ => return self.err("unconnected UDP terminal"),
                                }
                            }
                            if terms.len() != m.ports.len() {
                                return self.err(format!("primitive '{module}' needs {} terminals", m.ports.len()));
                            }
                            self.jobs.push(Job::Udp { module: m, terms, delay: delay.clone(), scope: sid, line: inst.line });
                        }
                        continue;
                    }
                    let mut ovs = Vec::new();
                    for p in params {
                        match p {
                            Conn::Positional(Some(e)) => ovs.push((None, self.pval(e, sid)?)),
                            Conn::Named(n, Some(e)) => ovs.push((Some(n.clone()), self.pval(e, sid)?)),
                            _ => {}
                        }
                    }
                    for inst in insts {
                        self.line = inst.line;
                        if inst.name.starts_with('$') {
                            return self.err(format!("Instantiation of module {} requires an instance name.", m.name));
                        }
                        if inst.dims.len() > 1 {
                            return self.err("multi-dimensional arrays of instances are not supported");
                        }
                        if let Some((l, r)) = inst.dims.first() {
                            // an array of instances: name[i] for i from l to r, terminals sliced
                            let (l, r) = (self.const_int(l, sid)?, self.const_int(r, sid)?);
                            let n = ((l - r).abs() + 1) as u32;
                            let pws = self.port_widths(m, &ovs)?;
                            // elaborated lowest index first, like Icarus
                            for i in l.min(r)..=l.max(r) {
                                let k = (i - l).unsigned_abs() as u32;
                                let mut conns = Vec::new();
                                for (pos, c) in inst.conns.iter().enumerate() {
                                    let (port, e) = match c {
                                        Conn::Positional(e) => (m.ports.get(pos).cloned().unwrap_or_default(), e),
                                        Conn::Named(nm, e) => (nm.clone(), e),
                                        Conn::Wildcard => {
                                            conns.push(Conn::Wildcard);
                                            continue;
                                        }
                                    };
                                    let e = match e {
                                        Some(e) => {
                                            let pw = pws.get(&port).copied().unwrap_or(1);
                                            let tw = self.ty(e, sid)?.0;
                                            if n > 1 && tw == pw * n {
                                                Some(self.array_slice(e, k, n, pw, sid)?)
                                            } else if tw == pw || tw < pw * n {
                                                Some(e.clone())
                                            } else {
                                                return self.err(format!(
                                                    "port '{port}' of instance array '{}': {tw} bits do not fit {n} x {pw}",
                                                    inst.name
                                                ));
                                            }
                                        }
                                        None => None,
                                    };
                                    conns.push(match c {
                                        Conn::Positional(_) => Conn::Positional(e),
                                        _ => Conn::Named(port, e),
                                    });
                                }
                                // the generated connections live as long as the elaboration
                                let conns: &'a [Conn] = Box::leak(conns.into_boxed_slice());
                                let iname = format!("{}[{}]", inst.name, i);
                                let path = format!("{}.{}", self.scopes[sid].name, iname);
                                let child = self.instantiate(m, path, &ovs, Some((conns, sid)))?;
                                self.scopes[sid].children.insert(iname, child);
                            }
                            continue;
                        }
                        let path = format!("{}.{}", self.scopes[sid].name, inst.name);
                        let child = self.instantiate(m, path, &ovs, Some((&inst.conns, sid)))?;
                        self.scopes[sid].children.insert(inst.name.clone(), child);
                    }
                }
                Item::GenFor { init_var, init, cond, step_var, step, name, items: body, line } => {
                    self.line = *line;
                    genblk += 1;
                    let bname = name.clone().unwrap_or_else(|| format!("genblk{genblk}"));
                    let mut v = self.const_int(init, sid)?;
                    let mut iters = 0;
                    loop {
                        let it_name = format!("{bname}[{v}]");
                        let path = format!("{}.{}", self.scopes[sid].name, it_name);
                        let file = self.file.clone();
                        let isid = self.new_scope(path, Some(sid), file);
                        self.scopes[isid].syms.insert(init_var.clone(), Sym::Const(Const::from_i64(v, 32, true)));
                        if !self.const_eval(cond, isid)?.is_true() {
                            break;
                        }
                        self.scopes[sid].children.insert(it_name, isid);
                        self.elab_items(body, isid, false, &HashMap::new(), None)?;
                        if step_var != init_var {
                            return self.err("generate loop must step its own genvar");
                        }
                        v = self.const_int(step, isid)?;
                        iters += 1;
                        if iters > 1_000_000 {
                            return self.err("generate loop does not terminate");
                        }
                    }
                }
                Item::GenIf { cond, then_items, then_name, else_items, else_name, line } => {
                    self.line = *line;
                    genblk += 1;
                    let c = self.const_eval(cond, sid)?.is_true();
                    let (its, nm) = if c { (then_items, then_name) } else { (else_items, else_name) };
                    self.gen_scope(its, nm, genblk, sid)?;
                }
                Item::GenCase { expr, arms, line } => {
                    self.line = *line;
                    genblk += 1;
                    let v = self.const_eval(expr, sid)?;
                    let mut chosen = None;
                    for (labels, nm, its) in arms {
                        if labels.is_empty() {
                            if chosen.is_none() {
                                chosen = Some((its, nm));
                            }
                            continue;
                        }
                        let mut hit = false;
                        for l in labels {
                            let lv = self.const_eval(l, sid)?;
                            let w = lv.w.max(v.w);
                            if lv.resize(w, lv.signed).a == v.resize(w, v.signed).a {
                                hit = true;
                            }
                        }
                        if hit {
                            chosen = Some((its, nm));
                            break;
                        }
                    }
                    if let Some((its, nm)) = chosen {
                        self.gen_scope(its, nm, genblk, sid)?;
                    }
                }
                Item::GenBlock { name, items: its, line } => {
                    self.line = *line;
                    genblk += 1;
                    self.gen_scope(its, name, genblk, sid)?;
                }
                Item::Gate { kind, terms, line, dims, strength, delay, .. } => {
                    self.line = *line;
                    self.cur_strength = *strength;
                    self.cur_delay = delay.clone();
                    let r = self.gate_items(kind, terms, dims, sid, *line);
                    self.cur_strength = None;
                    self.cur_delay = None;
                    r?;
                }
            }
        }
        Ok(())
    }

    /// A gate instance or an array of them (1-bit terminals per instance).
    fn gate_items(&mut self, kind: &str, terms: &'a [Expr], dims: &Option<Range>, sid: usize, line: u32) -> R<()> {
                    match dims {
                        None => self.gate(kind, terms, sid, line)?,
                        Some((l, r)) => {
                            // an array of gates: 1-bit terminals per instance
                            let (l, r) = (self.const_int(l, sid)?, self.const_int(r, sid)?);
                            let n = ((l - r).abs() + 1) as u32;
                            for k in 0..n {
                                let mut ts = Vec::new();
                                for t in terms {
                                    let tw = self.ty(t, sid)?.0;
                                    ts.push(if tw == n && n > 1 { self.array_slice(t, k, n, 1, sid)? } else { t.clone() });
                                }
                                self.gate(kind, &ts, sid, line)?;
                            }
                        }
                    }
        Ok(())
    }

    fn gen_scope(&mut self, items: &'a [Item], name: &Option<String>, n: usize, sid: usize) -> R<()> {
        if items.is_empty() {
            return Ok(());
        }
        let bname = name.clone().unwrap_or_else(|| format!("genblk{n}"));
        let path = format!("{}.{}", self.scopes[sid].name, bname);
        let file = self.file.clone();
        let child = self.new_scope(path, Some(sid), file);
        self.scopes[sid].children.insert(bname, child);
        self.elab_items(items, child, false, &HashMap::new(), None)
    }

    /// Net bits named by `e` as (signal, lsb, width), most significant first; None if `e` is not
    /// made of constant selects of scalar nets.
    fn net_segs(&mut self, e: &Expr, sid: usize) -> Option<Vec<(u32, u32, u32)>> {
        let base = |me: &mut Self, b: &Expr| -> Option<u32> {
            match me.resolve_base(b, sid).ok()?? {
                expr::Base::Sig(s) => {
                    let i = me.sig(s);
                    (i.kind == SigKind::Wire && i.depth == 0 && !i.real).then_some(s)
                }
                _ => None,
            }
        };
        match e {
            Expr::Ident(_) | Expr::Hier(_) => {
                let s = base(self, e)?;
                Some(vec![(s, 0, self.sig(s).width)])
            }
            Expr::Index(b, i) => {
                let s = base(self, b)?;
                let i = self.const_value(i, sid)?;
                let info = self.sig(s);
                let (neg, k) = Self::bit_map(info.msb, info.lsb);
                let off = if neg { k - i } else { i + k };
                (off >= 0 && off < info.width as i64).then(|| vec![(s, off as u32, 1)])
            }
            Expr::Range(b, m, l) => {
                let s = base(self, b)?;
                let (m, l) = (self.const_value(m, sid)?, self.const_value(l, sid)?);
                let info = self.sig(s);
                let (neg, k) = Self::bit_map(info.msb, info.lsb);
                let (om, ol) = if neg { (k - m, k - l) } else { (m + k, l + k) };
                let (lo, hi) = (om.min(ol), om.max(ol));
                (lo >= 0 && hi < info.width as i64).then(|| vec![(s, lo as u32, (hi - lo + 1) as u32)])
            }
            Expr::Concat(v) => {
                let mut out = Vec::new();
                for x in v {
                    out.extend(self.net_segs(x, sid)?);
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// Link the bits of `x` (in scope `xs`) and `y` (in `ys`) bidirectionally, LSB first, over the
    /// common width. False if either is not made of nets.
    fn link_exprs(&mut self, x: &Expr, xs: usize, y: &Expr, ys: usize, ctl: Option<(Expr, usize, bool)>) -> bool {
        let (Some(xa), Some(ya)) = (self.net_segs(x, xs), self.net_segs(y, ys)) else { return false };
        // expand into LSB-first runs and pair them up
        let (mut xi, mut yi) = (xa.into_iter().rev(), ya.into_iter().rev());
        let (mut xc, mut yc) = (xi.next(), yi.next());
        while let (Some((xsig, xl, xw)), Some((ysig, yl, yw))) = (xc, yc) {
            let w = xw.min(yw);
            self.links.push(Link { a: xsig, a_lsb: xl, b: ysig, b_lsb: yl, w, ctl: ctl.clone() });
            xc = if xw > w { Some((xsig, xl + w, xw - w)) } else { xi.next() };
            yc = if yw > w { Some((ysig, yl + w, yw - w)) } else { yi.next() };
        }
        true
    }

    fn gate(&mut self, kind: &str, terms: &[Expr], sid: usize, line: u32) -> R<()> {
        if terms.len() < 2 && !matches!(kind, "pullup" | "pulldown") {
            return self.err("gate needs at least two terminals");
        }
        if kind.contains("tran") {
            // bidirectional switches (strengths are ignored: rtran = tran)
            let ctl = match kind.trim_start_matches('r') {
                "tran" if terms.len() == 2 => None,
                "tranif1" | "tranif0" if terms.len() == 3 => Some((terms[2].clone(), sid, kind.ends_with('1'))),
                _ => return self.err(format!("{kind}: wrong number of terminals")),
            };
            if !self.link_exprs(&terms[0], sid, &terms[1], sid, ctl) {
                return self.err(format!("{kind} terminals must be nets"));
            }
            return Ok(());
        }
        let b = |e: &Expr| Box::new(e.clone());
        let z = || Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: "z".into(), fill: true });
        match kind {
            "pullup" | "pulldown" => {
                // the pulled bits: (net, lsb, width, up)
                for t in terms {
                    match self.net_segs(t, sid) {
                        Some(segs) => {
                            for (s, lsb, w) in segs {
                                self.pulls.entry(s).or_default().push((lsb, w, kind == "pullup"));
                            }
                        }
                        None => {
                            let mut v = Vec::new();
                            self.lvalue_bases(t, sid, &mut v);
                            for s in v {
                                let w = self.sig(s).width;
                                self.pulls.entry(s).or_default().push((0, w, kind == "pullup"));
                            }
                        }
                    }
                }
            }
            "bufif0" | "bufif1" | "notif0" | "notif1" | "nmos" | "pmos" | "rnmos" | "rpmos" => {
                // (out, in, control): tri-state buffer; an x control merges the two choices
                if terms.len() != 3 {
                    return self.err(format!("{kind} needs three terminals"));
                }
                // bufif/notif turn a z input into x; the switches pass it through
                let data = if kind.starts_with("notif") {
                    Expr::Unary(UnOp::Not, b(&terms[1]))
                } else if kind.starts_with("bufif") {
                    Expr::Binary(BinOp::And, b(&terms[1]), b(&terms[1]))
                } else {
                    terms[1].clone()
                };
                let on_high = matches!(kind, "bufif1" | "notif1" | "nmos" | "rnmos");
                let rhs = if on_high {
                    Expr::Cond(b(&terms[2]), Box::new(data), Box::new(z()))
                } else {
                    Expr::Cond(b(&terms[2]), Box::new(z()), Box::new(data))
                };
                self.push_job(Job::Assign { lhs: terms[0].clone(), rhs, delay: self.cur_delay.clone(), scope: sid, line });
            }
            "cmos" | "rcmos" => {
                // (out, in, ncontrol, pcontrol)
                if terms.len() != 4 {
                    return self.err(format!("{kind} needs four terminals"));
                }
                let p_side = Expr::Cond(b(&terms[3]), Box::new(z()), b(&terms[1]));
                let rhs = Expr::Cond(b(&terms[2]), b(&terms[1]), Box::new(p_side));
                self.push_job(Job::Assign { lhs: terms[0].clone(), rhs, delay: self.cur_delay.clone(), scope: sid, line });
            }
            "not" | "buf" => {
                let input = terms.last().unwrap();
                for out in &terms[..terms.len() - 1] {
                    // a buffer turns z into x: `in & in`
                    let rhs = if kind == "not" {
                        Expr::Unary(UnOp::Not, b(input))
                    } else {
                        Expr::Binary(BinOp::And, b(input), b(input))
                    };
                    self.push_job(Job::Assign { lhs: out.clone(), rhs, delay: self.cur_delay.clone(), scope: sid, line });
                }
            }
            _ => {
                let (op, inv) = match kind {
                    "and" => (BinOp::And, false),
                    "nand" => (BinOp::And, true),
                    "or" => (BinOp::Or, false),
                    "nor" => (BinOp::Or, true),
                    "xor" => (BinOp::Xor, false),
                    _ => (BinOp::Xor, true),
                };
                let mut e = terms[1].clone();
                for t in &terms[2..] {
                    e = Expr::Binary(op, Box::new(e), b(t));
                }
                if inv {
                    e = Expr::Unary(UnOp::Not, Box::new(e));
                }
                self.push_job(Job::Assign { lhs: terms[0].clone(), rhs: e, delay: self.cur_delay.clone(), scope: sid, line });
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ code generation
    /// Register a named block / task body; returns its registry index.
    pub(crate) fn open_block(&mut self, path: String) -> Option<usize> {
        let proc = self.pb.last().map(|p| p.proc).unwrap_or(NONE);
        if proc == NONE {
            return None; // constant evaluation
        }
        let start = self.here();
        self.block_reg.push((path, rt::BlockDef { proc, start, end: start, children: Vec::new() }));
        let i = self.block_reg.len() - 1;
        self.open_blocks.push(i);
        Some(i)
    }

    pub(crate) fn close_block(&mut self, i: Option<usize>) {
        if let Some(i) = i {
            self.block_reg[i].1.end = self.here();
            let top = self.open_blocks.pop();
            debug_assert_eq!(top, Some(i));
        }
    }

    /// Map the `disable` targets to the registered blocks (first matching scope outwards).
    fn resolve_disables(&mut self) {
        let pending = std::mem::take(&mut self.pending_disables);
        for (target, cands, file, line) in pending {
            let mut ids = Vec::new();
            for c in &cands {
                ids = (0..self.block_reg.len() as u32).filter(|&i| &self.block_reg[i as usize].0 == c).collect();
                if !ids.is_empty() {
                    break;
                }
            }
            if ids.is_empty() && !cands.iter().any(|c| c.starts_with("$force:")) {
                let n = cands.first().map(|c| c.rsplit('.').next().unwrap_or(c).to_string()).unwrap_or_default();
                let m = format!("{file}:{line}: warning: disable: no block or task named '{n}'");
                if self.warned.insert(m.clone()) {
                    self.warnings.push(m);
                }
            }
            self.prog.disables[target as usize] = ids;
        }
        self.prog.blocks = self.block_reg.iter().map(|b| b.1.clone()).collect();
    }

    /// Assignment targets go to the job's driver signal when the net has several drivers.
    /// A view of a redirected net becomes a view of the job's driver signal.
    pub(crate) fn redir(&mut self, s: u32) -> u32 {
        let c = self.canon(s);
        let Some(&d) = self.redirect.get(&c) else { return s };
        if c == s {
            return d;
        }
        if let Some(&v) = self.drv_views.get(&(s, d)) {
            return v;
        }
        let i = self.sig(s).clone();
        let v = self.new_view(d, i.name, i.msb, i.lsb, i.signed);
        self.prog.names.pop();
        self.drv_views.insert((s, d), v);
        v
    }

    /// Signals an lvalue writes (whole signals; part selects count as the signal).
    pub(crate) fn lvalue_bases(&self, e: &Expr, sid: usize, out: &mut Vec<u32>) {
        match e {
            Expr::Ident(n) => {
                if let Some(Sym::Sig(s)) = self.lookup(n, sid) {
                    out.push(self.canon(s));
                }
            }
            Expr::Hier(p) => {
                if let Some(Sym::Sig(s)) = self.lookup_hier(p, sid) {
                    out.push(self.canon(s));
                }
            }
            Expr::Index(a, _) | Expr::Range(a, _, _) | Expr::IndexedRange(a, _, _, _) => self.lvalue_bases(a, sid, out),
            Expr::Concat(v) => {
                for x in v {
                    self.lvalue_bases(x, sid, out);
                }
            }
            _ => {}
        }
    }

    fn job_targets(&self, job: &Job) -> Vec<u32> {
        let mut v = Vec::new();
        match job {
            Job::Assign { lhs, scope, .. } | Job::PortOut { lhs, scope, .. } => self.lvalue_bases(lhs, *scope, &mut v),
            Job::PortIn { child_sig, .. } => v.push(*child_sig),
            Job::Udp { terms, scope, .. } => self.lvalue_bases(&terms[0], *scope, &mut v),
            Job::Proc { .. } => {}
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Names that some `force` / procedural `assign` targets (the last component of the
    /// target's name), in all modules.
    fn forced_names(&self) -> HashSet<String> {
        fn expr_name(e: &Expr, out: &mut HashSet<String>) {
            match e {
                Expr::Ident(n) => {
                    out.insert(n.clone());
                }
                Expr::Hier(p) => {
                    if let Some(n) = p.last() {
                        out.insert(n.clone());
                    }
                }
                Expr::Index(a, _) | Expr::Range(a, _, _) | Expr::IndexedRange(a, _, _, _) => expr_name(a, out),
                Expr::Concat(v) => v.iter().for_each(|x| expr_name(x, out)),
                _ => {}
            }
        }
        fn stmt(s: &Stmt, out: &mut HashSet<String>) {
            match s {
                Stmt::Force { lhs, .. } => expr_name(lhs, out),
                Stmt::Block { stmts, .. } => stmts.iter().for_each(|x| stmt(x, out)),
                Stmt::If { then_s, else_s, .. } => {
                    stmt(then_s, out);
                    if let Some(e) = else_s {
                        stmt(e, out);
                    }
                }
                Stmt::Case { arms, default, .. } => {
                    arms.iter().for_each(|a| stmt(&a.1, out));
                    if let Some(d) = default {
                        stmt(d, out);
                    }
                }
                Stmt::For { init, step, body, .. } => {
                    stmt(init, out);
                    stmt(step, out);
                    stmt(body, out);
                }
                Stmt::While { body, .. }
                | Stmt::DoWhile { body, .. }
                | Stmt::Repeat { body, .. }
                | Stmt::Forever { body, .. }
                | Stmt::Timed { body, .. }
                | Stmt::Wait { body, .. } => stmt(body, out),
                _ => {}
            }
        }
        fn items(its: &[Item], out: &mut HashSet<String>) {
            for it in its {
                match it {
                    Item::Process { body, .. } => stmt(body, out),
                    Item::Subroutine(f) => stmt(&f.body, out),
                    Item::GenFor { items: i, .. } | Item::GenBlock { items: i, .. } => items(i, out),
                    Item::GenIf { then_items, else_items, .. } => {
                        items(then_items, out);
                        items(else_items, out);
                    }
                    Item::GenCase { arms, .. } => arms.iter().for_each(|a| items(&a.2, out)),
                    _ => {}
                }
            }
        }
        let mut out = HashSet::new();
        for m in self.mods.values() {
            items(&m.items, &mut out);
        }
        out
    }

    /// A plain continuous assignment of a whole signal to a net of the same width (`assign z =
    /// a;`, `wire z = a;`, also `{a}`, a constant-index array element, a zero delay) makes the
    /// net another name for the source, as in the reference: `z` changes together with `a`,
    /// inside the process that writes `a`. Not for nets with other drivers, special net types,
    /// strengths, links or forces (a forced alias would force its source).
    fn alias_assigns(&mut self, jobs: Vec<Job<'a>>) -> Vec<Job<'a>> {
        let mut count: HashMap<u32, u32> = HashMap::new();
        for job in &jobs {
            for s in self.job_targets(job) {
                *count.entry(s).or_default() += 1;
            }
        }
        let linked: HashSet<u32> = self.links.iter().flat_map(|l| [self.canon(l.a), self.canon(l.b)]).collect();
        let fnames = self.forced_names();
        let mut forced: HashSet<u32> = HashSet::new();
        if !fnames.is_empty() {
            for sc in &self.scopes {
                for (n, sym) in &sc.syms {
                    if let (true, Sym::Sig(s)) = (fnames.contains(n), sym) {
                        forced.insert(self.canon(*s));
                    }
                }
            }
        }
        let old_strength = std::mem::take(&mut self.job_strength);
        let mut kept = Vec::with_capacity(jobs.len());
        let mut changed = false;
        for (k, job) in jobs.into_iter().enumerate() {
            if !old_strength.contains_key(&k) {
                if let Job::Assign { lhs, rhs, delay, scope, .. } = &job {
                    if let Some((z, a)) = self.alias_pair(lhs, rhs, delay.as_ref(), *scope) {
                        let i = self.sig(z);
                        let ok = count.get(&z) == Some(&1)
                            && i.kind != SigKind::Integer
                            && i.depth == 0
                            && i.alias_of.is_none()
                            && !i.real
                            && !self.net_types.contains_key(&z)
                            && !self.pulls.contains_key(&z)
                            && !self.two_state.contains(&z)
                            && !linked.contains(&z)
                            && !forced.contains(&z);
                        if ok {
                            if i.kind == SigKind::Reg {
                                self.alias_vars.insert(z);
                            }
                            self.view_of.insert(z, a);
                            changed = true;
                            continue;
                        }
                    }
                }
            }
            if let Some(st) = old_strength.get(&k) {
                self.job_strength.insert(kept.len(), *st);
            }
            kept.push(job);
        }
        if changed {
            // views share the storage of the signal they stand for
            let views: Vec<u32> = self.view_of.keys().copied().collect();
            for v in views {
                let b = self.canon(v);
                self.prog.signals[v as usize].slot = self.prog.signals[b as usize].slot;
            }
        }
        kept
    }

    /// For `assign z = a` with a whole-signal source of the same width: (net, source), both
    /// as the signals they stand for.
    fn alias_pair(&mut self, lhs: &Expr, rhs: &Expr, delay: Option<&Expr>, sid: usize) -> Option<(u32, u32)> {
        if let Some(d) = delay {
            if self.const_value(d, sid) != Some(0) {
                return None;
            }
        }
        if !matches!(lhs, Expr::Ident(_) | Expr::Hier(_)) {
            return None;
        }
        let Ok(Some(expr::Base::Sig(z))) = self.resolve_base(lhs, sid) else { return None };
        let z = self.canon(z);
        let mut rhs = rhs;
        while let Expr::Concat(v) = rhs {
            if v.len() != 1 {
                return None;
            }
            rhs = &v[0];
        }
        let width = self.sig(z).width;
        let a = match rhs {
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(rhs, sid) {
                Ok(Some(expr::Base::Sig(a))) if self.sig(a).depth == 0 => a,
                _ => return None,
            },
            Expr::Index(..) => {
                let name = format!("{}$alias", self.sig(z).name);
                let a = self.elem_port_alias(rhs, sid, width, &name)?;
                self.prog.names.retain(|n| n.1 != a);
                a
            }
            _ => return None,
        };
        let a = self.canon(a);
        let ai = self.sig(a);
        if a == z || ai.width != width || ai.real {
            return None;
        }
        Some((z, a))
    }

    fn codegen_jobs(&mut self) -> R<()> {
        let jobs = std::mem::take(&mut self.jobs);
        let jobs = self.alias_assigns(jobs);
        // nets with more than one continuous driver are resolved from per-driver signals
        let mut count: HashMap<u32, u32> = HashMap::new();
        for job in &jobs {
            for s in self.job_targets(job) {
                *count.entry(s).or_default() += 1;
            }
        }
        let multi: HashSet<u32> = count
            .into_iter()
            .filter(|&(s, n)| {
                let i = self.sig(s);
                let special = self.net_types.contains_key(&s) || self.pulls.contains_key(&s);
                (n > 1 || special) && i.kind == SigKind::Wire && i.depth == 0 && i.alias_of.is_none()
            })
            .map(|(s, _)| s)
            .collect();
        // linked nets (tran, inout ports on parts of nets) and nets with drivers of a
        // non-default strength are always resolved
        let mut multi = multi;
        let job_strength = std::mem::take(&mut self.job_strength);
        for (k, job) in jobs.iter().enumerate() {
            if job_strength.get(&k).is_some_and(|st| *st != (6, 6)) {
                for s in self.job_targets(job) {
                    let i = self.sig(s);
                    if i.kind == SigKind::Wire && i.depth == 0 && i.alias_of.is_none() {
                        multi.insert(s);
                    }
                }
            }
        }
        for l in &self.links {
            for s in [l.a, l.b] {
                if self.sig(s).alias_of.is_none() {
                    multi.insert(s);
                }
            }
        }
        // special nets nothing drives: their resolved value is constant
        let undriven: Vec<u32> = self
            .net_types
            .keys()
            .chain(self.pulls.keys())
            .copied()
            .filter(|s| !multi.contains(s))
            .collect();
        for s in undriven {
            if let Some(c) = self.pull_const(s) {
                let i = self.sig(s).clone();
                let n = rt::nw(i.width);
                for k in 0..n {
                    self.prog.mem[i.slot as usize + k] = c.a[k];
                    self.prog.mem[i.slot as usize + n + k] = c.b[k];
                }
            }
        }
        let mut drv_strength: HashMap<u32, (u8, u8)> = HashMap::new();
        for (jk, job) in jobs.iter().enumerate() {
            self.redirect.clear();
            for s in self.job_targets(job) {
                if multi.contains(&s) {
                    let i = self.sig(s).clone();
                    let k = self.drivers.get(&s).map(|v| v.len()).unwrap_or(0);
                    let d = self.new_signal(format!("{}$drv{k}", i.name), i.width, i.signed, 0, i.msb, i.lsb, 0, SigKind::Wire);
                    self.prog.names.pop(); // driver signals are not dumped
                    if let Some(&st) = job_strength.get(&jk) {
                        drv_strength.insert(d, st);
                    }
                    self.drivers.entry(s).or_default().push(d);
                    self.redirect.insert(s, d);
                }
            }
            self.codegen_job(job)?;
        }
        self.redirect.clear();
        let island = self.link_islands()?;
        // resolver processes: net = resolve(driver 0, driver 1, ...)
        let mut nets: Vec<(u32, Vec<u32>)> =
            std::mem::take(&mut self.drivers).into_iter().filter(|(n, _)| !island.contains(n)).collect();
        nets.sort();
        for (net, drv) in nets {
            let i = self.sig(net).clone();
            let p = self.begin_proc(i.name.clone(), ProcClass::Normal);
            self.prog.procs[p as usize].fusible = true;
            let top = self.here();
            let nt = self.net_types.get(&net).copied().unwrap_or(NetType::Wire);
            let rop = match nt {
                NetType::Wand => rt::BinOp::ResolveAnd,
                NetType::Wor => rt::BinOp::ResolveOr,
                _ => rt::BinOp::Resolve,
            };
            let mut v = self.sig(drv[0]).slot;
            let strengths = drv.iter().any(|d| drv_strength.get(d).is_some_and(|st| *st != (6, 6)));
            if strengths && rop == rt::BinOp::Resolve {
                // drivers of different strengths: the strongest wins (per bit)
                let list: Vec<(u32, u8, u8)> = drv
                    .iter()
                    .map(|d| {
                        let (s0, s1) = drv_strength.get(d).copied().unwrap_or((6, 6));
                        (self.sig(*d).slot, s0, s1)
                    })
                    .collect();
                let t = self.tmp(i.width);
                self.emit(Instr::ResolveStr { d: t, w: i.width, drv: list.into_boxed_slice() });
                v = t;
                self.prog.procs[p as usize].fusible = false;
            } else {
                for &d in &drv[1..] {
                    let t = self.tmp(i.width);
                    let b = self.sig(d).slot;
                    self.emit(Instr::Bin { op: rop, d: t, a: v, b, w: i.width });
                    v = t;
                }
            }
            // pulled nets: undriven (z) bits read as 0 / 1
            if let Some(c) = self.pull_const(net) {
                let c = self.const_slot(&c);
                let t = self.tmp(i.width);
                self.emit(Instr::Bin { op: rt::BinOp::PullZ, d: t, a: v, b: c, w: i.width });
                v = t;
            }
            self.emit(Instr::Store { t: rt::Target::Whole { sig: net }, s: v, mode: rt::StoreMode::Blocking });
            let ctl = self.make_ctl(drv.iter().map(|&d| (d, rt::Edge::Any, NONE)).collect());
            self.emit(Instr::Wait { ctl });
            self.emit(Instr::Jmp { to: top });
            self.end_proc();
        }
        Ok(())
    }

    /// A continuous assignment of a constant to a whole net (or net driver) sets its initial
    /// value, so processes see it already at time 0, as with Icarus.
    fn const_assign_init(&mut self, lhs: &Expr, rhs: &Expr, sid: usize) -> R<()> {
        if !matches!(lhs, Expr::Ident(_) | Expr::Hier(_)) || !self.is_const(rhs, sid) || self.is_real(rhs, sid) {
            return Ok(());
        }
        let Some(expr::Base::Sig(s0)) = self.resolve_base(lhs, sid)? else { return Ok(()) };
        let s = self.redir(s0);
        let info = self.sig(s).clone();
        if info.depth != 0 || info.real || info.forced {
            return Ok(());
        }
        let c = self.const_eval(rhs, sid)?;
        let c = c.resize(info.width, c.signed);
        let n = rt::nw(info.width);
        for i in 0..n {
            self.prog.mem[info.slot as usize + i] = c.a[i];
            self.prog.mem[info.slot as usize + n + i] = c.b[i];
        }
        self.init_touch_net.push(s);
        if s != s0 {
            self.init_touch_net.push(s0);
        }
        Ok(())
    }

    /// The time-0 change events of initial values: a process that runs after all others have
    /// started.
    fn init_events(&mut self) {
        let mut v = std::mem::take(&mut self.init_touch_net);
        if self.dialect.v2005 {
            v.append(&mut self.init_touch);
        }
        let mut seen = HashSet::new();
        v.retain(|s| seen.insert(*s));
        if v.is_empty() {
            return;
        }
        self.job_scope = usize::MAX;
        self.begin_proc("$init".into(), ProcClass::Normal);
        for sig in v {
            self.emit(Instr::Touch { sig });
        }
        self.emit(Instr::Halt);
        self.end_proc();
    }

    /// Module path delays (`specify`): an output port with paths is driven, through an inertial
    /// delay, from an internal copy that the module's logic drives; the delay is that of the path
    /// whose source changed last (each source records the time of its last change).
    fn path_delays(&mut self, items: &'a [Item], sid: usize, dirs: &HashMap<String, Dir>) -> R<()> {
        // destination -> (source, delay) pairs
        let mut by_dst: Vec<(String, Vec<(String, Expr)>)> = Vec::new();
        for it in items {
            let Item::SpecPath { srcs, dsts, delay, line } = it else { continue };
            self.line = *line;
            for d in dsts {
                if dirs.get(d) != Some(&Dir::Output) {
                    continue; // only output ports (inout paths are not modelled)
                }
                let e = match by_dst.iter_mut().find(|x| &x.0 == d) {
                    Some(e) => e,
                    None => {
                        by_dst.push((d.clone(), Vec::new()));
                        by_dst.last_mut().unwrap()
                    }
                };
                for s in srcs {
                    e.1.push((s.clone(), delay.clone()));
                }
            }
        }
        if by_dst.is_empty() {
            return Ok(());
        }
        let mut times: HashMap<String, String> = HashMap::new();
        for (dst, paths) in by_dst {
            let Some(Sym::Sig(port)) = self.scopes[sid].syms.get(&dst).cloned() else { continue };
            // the time of each source's last change
            for (src, _) in &paths {
                if times.contains_key(src) || self.lookup(src, sid).is_none() {
                    continue;
                }
                let tname = format!("$fxt_{src}");
                let full = format!("{}.{tname}", self.scopes[sid].name);
                let t = self.new_signal(full, 64, false, 0, 63, 0, 0, SigKind::Reg);
                self.prog.names.pop();
                self.zero_init(t);
                self.scopes[sid].syms.insert(tname.clone(), Sym::Sig(t));
                let body: &'a Stmt = Box::leak(Box::new(Stmt::Timed {
                    timing: Timing::Event(EventCtl::List(vec![(Edge::Any, Expr::Ident(src.clone()))])),
                    body: Box::new(Stmt::Assign {
                        lhs: Expr::Ident(tname.clone()),
                        rhs: Expr::SysCall("$time".into(), vec![]),
                        nonblocking: false,
                        timing: None,
                        line: self.line,
                    }),
                    line: self.line,
                }));
                self.jobs.push(Job::Proc { kind: ProcKind::Always, body, scope: sid, line: self.line });
                times.insert(src.clone(), tname);
            }
            let paths: Vec<(String, Expr)> = paths.into_iter().filter(|(s, _)| times.contains_key(s)).collect();
            if paths.is_empty() {
                continue;
            }
            // the internal copy the module's logic drives
            let info = self.sig(port).clone();
            let full = format!("{}.{dst}$int", self.scopes[sid].name);
            let int = self.new_signal(full, info.width, info.signed, 0, info.msb, info.lsb, 0, SigKind::Wire);
            self.prog.names.pop();
            let pname = format!("$fxport_{dst}");
            self.scopes[sid].syms.insert(dst.clone(), Sym::Sig(int));
            self.scopes[sid].syms.insert(pname.clone(), Sym::Sig(port));
            // (rise, fall, turn-off) of each path
            let comps = |d: &Expr| -> [Expr; 3] {
                match d {
                    Expr::SysCall(n, v) if n == "$__delays" => {
                        let t = v.get(2).cloned().unwrap_or_else(|| {
                            Expr::Cond(
                                Box::new(Expr::Binary(BinOp::Lt, Box::new(v[0].clone()), Box::new(v[1].clone()))),
                                Box::new(v[0].clone()),
                                Box::new(v[1].clone()),
                            )
                        });
                        [v[0].clone(), v[1].clone(), t]
                    }
                    d => [d.clone(), d.clone(), d.clone()],
                }
            };
            let tvar = |s: &str| Expr::Ident(times[s].clone());
            let mut sel: [Expr; 3] = comps(&paths[0].1);
            let mut tmax = tvar(&paths[0].0);
            for (src, d) in &paths[1..] {
                let newer = Box::new(Expr::Binary(BinOp::Gt, Box::new(tvar(src)), Box::new(tmax.clone())));
                let c = comps(d);
                for k in 0..3 {
                    sel[k] = Expr::Cond(newer.clone(), Box::new(c[k].clone()), Box::new(sel[k].clone()));
                }
                tmax = Expr::Cond(newer, Box::new(tvar(src)), Box::new(tmax));
            }
            // a multi-bit output: the LSB's transition picks the delay for all bits (as Icarus)
            let multi = info.width > 1;
            if multi {
                let w = "sorry: multi-bit module path delays are currently not fully supported.\n     : using the LSB delay for all bits.".to_string();
                if self.warned.insert(w.clone()) {
                    self.warnings.push(w);
                }
            }
            let delay = Expr::SysCall(if multi { "$__pathdelays" } else { "$__delays" }.into(), sel.to_vec());
            self.push_job(Job::Assign {
                lhs: Expr::Ident(pname),
                rhs: Expr::Ident(dst.clone()),
                delay: Some(delay),
                scope: sid,
                line: self.line,
            });
        }
        Ok(())
    }

    /// An implicitly declared net (`default_nettype; `none` makes it an error).
    fn implicit_net(&mut self, n: &str, width: u32, sid: usize) -> R<u32> {
        let nt = self.scopes[sid].nettype.clone();
        let net = match nt.as_deref() {
            None => NetType::Wire,
            Some("none") => return self.err(format!("undeclared identifier '{n}' (`default_nettype none)")),
            Some("wand" | "triand") => NetType::Wand,
            Some("wor" | "trior") => NetType::Wor,
            Some("tri0") => NetType::Tri0,
            Some("tri1") => NetType::Tri1,
            Some("supply0") => NetType::Supply0,
            Some("supply1") => NetType::Supply1,
            Some(_) => NetType::Wire,
        };
        let full = format!("{}.{}", self.scopes[sid].name, n);
        let s = self.new_signal(full, width, false, 0, width as i64 - 1, 0, 0, SigKind::Wire);
        self.scopes[sid].syms.insert(n.to_string(), Sym::Sig(s));
        if net != NetType::Wire {
            self.net_types.insert(s, net);
        }
        Ok(s)
    }

    /// What undriven (z) bits of a net read as: 1 (pullup, tri1), 0 (pulldown, tri0) or z.
    fn pull_const(&self, s: u32) -> Option<Const> {
        let w = self.sig(s).width;
        let mut c = match self.net_types.get(&s) {
            Some(NetType::Tri0) => Const::fill('0', w),
            Some(NetType::Tri1) => Const::fill('1', w),
            _ if self.pulls.contains_key(&s) => Const::fill('z', w),
            _ => return None,
        };
        for &(lsb, pw, up) in self.pulls.get(&s).map(|v| v.as_slice()).unwrap_or(&[]) {
            for b in lsb..(lsb + pw).min(w) {
                let (i, m) = ((b / 64) as usize, 1u64 << (b % 64));
                c.b[i] &= !m;
                if up {
                    c.a[i] |= m;
                } else {
                    c.a[i] &= !m;
                }
            }
        }
        Some(c)
    }

    /// Linked nets form islands (connected through links). One process per island computes all
    /// its nets at once: each net starts from its own drivers, then the links exchange values
    /// (resolve the two sides; a tranif with an unknown control blends) until every path is
    /// covered. Solving the island in one activation (as Icarus does) avoids the oscillation of
    /// separate per-link processes in loops. Returns the island nets (they get no resolver).
    fn link_islands(&mut self) -> R<HashSet<u32>> {
        let links = std::mem::take(&mut self.links);
        let mut nets_done = HashSet::new();
        if links.is_empty() {
            return Ok(nets_done);
        }
        // union-find over the linked nets
        let mut parent: HashMap<u32, u32> = HashMap::new();
        fn find(p: &mut HashMap<u32, u32>, x: u32) -> u32 {
            let mut r = x;
            while let Some(&q) = p.get(&r) {
                if q == r {
                    break;
                }
                r = q;
            }
            p.insert(x, r);
            r
        }
        for l in &links {
            for s in [l.a, l.b] {
                parent.entry(s).or_insert(s);
            }
            let (ra, rb) = (find(&mut parent, l.a), find(&mut parent, l.b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let mut islands: Vec<(u32, Vec<usize>)> = Vec::new();
        for (k, l) in links.iter().enumerate() {
            let r = find(&mut parent, l.a);
            match islands.iter_mut().find(|(x, _)| *x == r) {
                Some((_, v)) => v.push(k),
                None => islands.push((r, vec![k])),
            }
        }
        for (_, ks) in islands {
            let mut nets: Vec<u32> = Vec::new();
            for &k in &ks {
                for s in [links[k].a, links[k].b] {
                    if !nets.contains(&s) {
                        nets.push(s);
                    }
                }
            }
            let name = self.sig(nets[0]).name.clone();
            self.begin_proc(name, ProcClass::Normal);
            let top = self.here();
            let mut trig: Vec<(u32, rt::Edge, u32)> = Vec::new();
            // controls, evaluated once per activation; a control that is a net of the island
            // itself takes its value from the relaxation (starting from its own drivers)
            let mut ctls: Vec<Option<(u32, u32, bool)>> = Vec::new();
            let mut ctl_net: Vec<Option<u32>> = Vec::new();
            for &k in &ks {
                let own = match &links[k].ctl {
                    Some((ce @ (Expr::Ident(_) | Expr::Hier(_)), cs, _)) => match self.resolve_base(ce, *cs)? {
                        Some(expr::Base::Sig(s)) if nets.contains(&s) => Some(s),
                        _ => None,
                    },
                    _ => None,
                };
                ctl_net.push(own);
                if let (Some(s), Some((_, _, on1))) = (own, &links[k].ctl) {
                    ctls.push(Some((s, self.sig(s).width, *on1)));
                    continue;
                }
                ctls.push(match &links[k].ctl {
                    Some((ce, cs, on1)) => {
                        let (c, cw, _) = self.self_expr(ce, *cs)?;
                        let keep = self.tmp(cw);
                        self.emit(Instr::Copy { d: keep, s: c, w: cw });
                        let mut r = Vec::new();
                        self.reads_expr(ce, *cs, &mut r);
                        trig.extend(r.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)));
                        Some((keep, cw, *on1))
                    }
                    None => None,
                });
            }
            // each net from its own drivers
            let mut cur: HashMap<u32, u32> = HashMap::new();
            for &n in &nets {
                let w = self.sig(n).width;
                let drv = self.drivers.get(&n).cloned().unwrap_or_default();
                let v = self.tmp(w);
                if drv.is_empty() {
                    let z = self.const_slot(&Const::fill('z', w));
                    self.emit(Instr::Copy { d: v, s: z, w });
                } else {
                    let first = self.sig(drv[0]).slot;
                    self.emit(Instr::Copy { d: v, s: first, w });
                    for &d in &drv[1..] {
                        let ds = self.sig(d).slot;
                        self.emit(Instr::Bin { op: rt::BinOp::Resolve, d: v, a: v, b: ds, w });
                    }
                }
                trig.extend(drv.iter().map(|&d| (d, rt::Edge::Any, NONE)));
                cur.insert(n, v);
            }
            // relax over the links: one pass per link covers every path
            for _ in 0..ks.len() {
                for (j, &k) in ks.iter().enumerate() {
                    let l = links[k].clone();
                    let (ca, cb) = (cur[&l.a], cur[&l.b]);
                    let (wa, wb) = (self.sig(l.a).width, self.sig(l.b).width);
                    let ta = self.tmp(l.w);
                    let tb = self.tmp(l.w);
                    self.emit(Instr::Slice { d: ta, s: ca, sw: wa, lsb: l.a_lsb, w: l.w });
                    self.emit(Instr::Slice { d: tb, s: cb, sw: wb, lsb: l.b_lsb, w: l.w });
                    let r = self.tmp(l.w);
                    self.emit(Instr::Bin { op: rt::BinOp::Resolve, d: r, a: ta, b: tb, w: l.w });
                    let (ra, rb) = match ctls[j] {
                        None => (r, r),
                        Some((c, cw, on1)) => {
                            let c = match ctl_net[j] {
                                Some(n) => cur[&n],
                                None => c,
                            };
                            let (xa, xb) = (self.tmp(l.w), self.tmp(l.w));
                            let (ta2, tb2) = if on1 { ((r, ta), (r, tb)) } else { ((ta, r), (tb, r)) };
                            self.emit(Instr::Mux { d: xa, c, cw, t: ta2.0, f: ta2.1, w: l.w });
                            self.emit(Instr::Mux { d: xb, c, cw, t: tb2.0, f: tb2.1, w: l.w });
                            (xa, xb)
                        }
                    };
                    self.emit(Instr::Insert { d: ca, dw: wa, s: ra, sw: l.w, off: l.a_lsb });
                    self.emit(Instr::Insert { d: cb, dw: wb, s: rb, sw: l.w, off: l.b_lsb });
                }
            }
            for &n in &nets {
                let w = self.sig(n).width;
                let mut v = cur[&n];
                if let Some(c) = self.pull_const(n) {
                    let c = self.const_slot(&c);
                    let t = self.tmp(w);
                    self.emit(Instr::Bin { op: rt::BinOp::PullZ, d: t, a: v, b: c, w });
                    v = t;
                }
                self.emit(Instr::Store { t: rt::Target::Whole { sig: n }, s: v, mode: rt::StoreMode::Blocking });
                nets_done.insert(n);
            }
            if trig.is_empty() {
                self.emit(Instr::Halt);
            } else {
                let ctl = self.make_ctl(trig);
                self.emit(Instr::Wait { ctl });
                self.emit(Instr::Jmp { to: top });
            }
            self.end_proc();
        }
        Ok(nets_done)
    }

    fn codegen_job(&mut self, job: &Job<'a>) -> R<()> {
        // (only initial / always blocks start in scope order; continuous assignments and
        // port connections keep theirs)
        self.job_scope = match job {
            Job::Proc { scope, .. } => *scope,
            _ => usize::MAX - 1,
        };
        {
            match job {
                Job::Proc { kind, body, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    let class = if *kind == ProcKind::Final { ProcClass::Final } else { ProcClass::Normal };
                    let name = self.scopes[*scope].name.clone();
                    self.begin_proc(name, class);
                    match kind {
                        ProcKind::Initial | ProcKind::Final => {
                            self.stmt(body, *scope)?;
                            self.emit(Instr::Halt);
                        }
                        ProcKind::Always | ProcKind::AlwaysFf => {
                            if let Stmt::Timed { timing: Timing::Event(EventCtl::Star), .. } = body {
                                let p = self.pb.last().unwrap().proc;
                                self.prog.procs[p as usize].fusible = true;
                            }
                            let top = self.here();
                            self.stmt(body, *scope)?;
                            self.emit(Instr::Jmp { to: top });
                            if self.lint_enabled {
                                if let Stmt::Timed { timing: Timing::Event(EventCtl::List(items)), body: inner, .. } = body {
                                    self.lint_sensitivity(items, inner, *scope, *line);
                                }
                            }
                        }
                        ProcKind::AlwaysComb | ProcKind::AlwaysLatch => {
                            let p = self.pb.last().unwrap().proc;
                            self.prog.procs[p as usize].fusible = *kind == ProcKind::AlwaysComb;
                            let top = self.here();
                            self.stmt(body, *scope)?;
                            // IEEE 1800 9.2.2.2.1: what the block and the functions it calls read,
                            // except what the block writes itself
                            let mut reads = Vec::new();
                            self.func_reads.set(true);
                            self.reads_stmt(body, *scope, &mut reads);
                            self.func_reads.set(false);
                            let mut written = Vec::new();
                            self.blocking_writes(body, *scope, &mut written);
                            reads.retain(|(s, _)| !written.contains(s));
                            let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)).collect());
                            self.emit(Instr::Wait { ctl });
                            self.emit(Instr::Jmp { to: top });
                        }
                    }
                    self.end_proc();
                }
                Job::Assign { lhs, rhs, delay, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    if delay.is_none() {
                        self.const_assign_init(lhs, rhs, *scope)?;
                    }
                    self.assign_proc(stmt::Lhs::Expr(lhs, *scope), stmt::Rhs::Expr(rhs, *scope), delay.as_ref())?;
                }
                Job::PortIn { child_sig, expr, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    self.assign_proc(stmt::Lhs::Sig(*child_sig), stmt::Rhs::Expr(expr, *scope), None)?;
                }
                Job::Udp { module, terms, delay, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    self.udp_instance(module, terms, delay.as_ref(), *scope)?;
                }
                Job::PortOut { lhs, child_sig, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    self.assign_proc(stmt::Lhs::Expr(lhs, *scope), stmt::Rhs::Sig(*child_sig), None)?;
                }
            }
        }
        Ok(())
    }

    /// The runtime table of a UDP (converted once per primitive).
    fn udp_table(&mut self, m: &Module) -> R<u32> {
        if let Some(&i) = self.udp_index.get(&m.name) {
            return Ok(i);
        }
        let def = m.udp.as_ref().unwrap();
        let mask = |c: &str| -> Option<u8> {
            Some(match c {
                "0" => rt::UDP_0,
                "1" => rt::UDP_1,
                "x" => rt::UDP_X,
                "?" => rt::UDP_0 | rt::UDP_1 | rt::UDP_X,
                "b" => rt::UDP_0 | rt::UDP_1,
                _ => return None,
            })
        };
        let mut rows = Vec::new();
        for (ins, state, out) in &def.rows {
            let mut syms = Vec::new();
            let mut has_edge = false;
            for s in ins {
                let any = rt::UDP_0 | rt::UDP_1 | rt::UDP_X;
                let sym = match s.as_str() {
                    "r" => rt::UdpSym { edge: true, prev: rt::UDP_0, cur: rt::UDP_1 },
                    "f" => rt::UdpSym { edge: true, prev: rt::UDP_1, cur: rt::UDP_0 },
                    "p" => rt::UdpSym { edge: true, prev: rt::UDP_0 | rt::UDP_X, cur: rt::UDP_1 | rt::UDP_X },
                    "n" => rt::UdpSym { edge: true, prev: rt::UDP_1 | rt::UDP_X, cur: rt::UDP_0 | rt::UDP_X },
                    "*" => rt::UdpSym { edge: true, prev: any, cur: any },
                    e if e.starts_with('(') && e.len() == 4 => {
                        let (a, b) = (mask(&e[1..2]), mask(&e[2..3]));
                        match (a, b) {
                            (Some(a), Some(b)) => rt::UdpSym { edge: true, prev: a, cur: b },
                            _ => return self.err(format!("bad edge '{e}' in table of '{}'", m.name)),
                        }
                    }
                    l => match mask(l) {
                        Some(c) => rt::UdpSym { edge: false, prev: 0, cur: c },
                        None => return self.err(format!("bad symbol '{l}' in table of '{}'", m.name)),
                    },
                };
                has_edge |= sym.edge;
                syms.push(sym);
            }
            if syms.len() != m.ports.len() - 1 {
                return self.err(format!("table row of '{}' has {} inputs, expected {}", m.name, syms.len(), m.ports.len() - 1));
            }
            let st = match state {
                Some(s) => mask(s).ok_or_else(|| self.mkerr(format!("bad state '{s}' in table of '{}'", m.name)))?,
                None => 7,
            };
            let out = match out.as_str() {
                "0" => 0,
                "1" => 1,
                "x" => 2,
                "-" => 3,
                o => return self.err(format!("bad output '{o}' in table of '{}'", m.name)),
            };
            rows.push(rt::UdpRow { ins: syms, state: st, out, has_edge });
        }
        let i = self.prog.udps.len() as u32;
        self.prog.udps.push(rt::UdpTable { sequential: def.sequential, rows });
        self.udp_index.insert(m.name.clone(), i);
        Ok(i)
    }

    /// A process for a UDP instance: evaluate the table on every input change, drive the output.
    fn udp_instance(&mut self, m: &'a Module, terms: &[Expr], delay: Option<&Expr>, sid: usize) -> R<()> {
        let table = self.udp_table(m)?;
        let def = m.udp.as_ref().unwrap();
        let name = self.scopes[sid].name.clone();
        self.begin_proc(name, ProcClass::Normal);
        // state / output and previous inputs persist between activations (start as x)
        let set = |c: &mut Self, s: u32, v: (u64, u64)| {
            c.prog.mem[s as usize] = v.0;
            c.prog.mem[s as usize + 1] = v.1;
        };
        let q = self.ptmp(1);
        let init = match &def.init {
            Some(e) => {
                let c = self.const_eval(e, sid)?;
                (c.a[0] & 1, c.b[0] & 1)
            }
            None => (1, 1),
        };
        set(self, q, init);
        let mut prev = Vec::new();
        for _ in 1..terms.len() {
            let s = self.ptmp(1);
            set(self, s, (1, 1));
            prev.push(s);
        }
        // a UDP output starts as x (not z) until the table drives it
        let mut outs = Vec::new();
        self.lvalue_bases(&terms[0], sid, &mut outs);
        for o in outs {
            let i = self.sig(o).clone();
            if i.kind == SigKind::Wire && i.depth == 0 {
                let n = rt::nw(i.width);
                for k in 0..n {
                    let m = if k == n - 1 { rt::tmask(i.width) } else { !0 };
                    self.prog.mem[i.slot as usize + k] = m;
                    self.prog.mem[i.slot as usize + n + k] = m;
                }
            }
        }
        let top = self.here();
        let mut ins = Vec::new();
        for e in &terms[1..] {
            let v = self.rhs_value(e, sid, 1)?;
            let t = self.tmp(1);
            self.emit(Instr::Copy { d: t, s: v, w: 1 });
            ins.push(t);
        }
        self.emit(Instr::Udp { table, ins: ins.into_boxed_slice(), prev: prev.into_boxed_slice(), q });
        if let Some(d) = delay {
            // the output changes `d` later; evaluation goes on meanwhile (no input change is missed)
            let ds = self.delay_ticks(d, sid)?;
            self.store_value(&stmt::Lhs::Expr(&terms[0], sid), q, 1, rt::StoreMode::NbaDelay(ds))?;
        } else {
            self.store_value(&stmt::Lhs::Expr(&terms[0], sid), q, 1, rt::StoreMode::Blocking)?;
        }
        let mut reads = Vec::new();
        for e in &terms[1..] {
            self.reads_expr(e, sid, &mut reads);
        }
        let ctl = self.make_ctl(reads.into_iter().map(|(s, e)| (s, rt::Edge::Any, e)).collect());
        self.emit(Instr::Wait { ctl });
        self.emit(Instr::Jmp { to: top });
        self.end_proc();
        Ok(())
    }
}
