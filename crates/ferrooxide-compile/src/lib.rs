//! The compiler: elaboration (`elab/`: hierarchy, parameters, declarations, generate, nets,
//! primitives, specify) and code generation (`codegen.rs`, `expr.rs`, `stmt.rs`) to
//! ferrooxide-rt bytecode. This file: the compiler state, scopes and name lookup, constant
//! evaluation.

mod codegen;
mod elab;
mod expr;
mod konst;
mod lint;
mod stmt;
mod strings;

pub use konst::Const;

use ferrooxide_front::ast::*;
use ferrooxide_front::Error;
use ferrooxide_rt::{self as rt, Instr, Program, ProcClass, ProcDef, SigKind, SignalInfo, NONE};
use ferrooxide_front::lower::SvType;
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
    /// arrays of instances and interface port arrays: (scope, name) -> (left, right); the
    /// elements are the children `name[i]` of the scope
    inst_arrays: HashMap<(usize, String), (i64, i64)>,
    /// the module (or interface) of each instance scope
    scope_module: HashMap<usize, &'a Module>,
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
    /// texts interned at compile time -> handle (`Program::strings`)
    pub(crate) str_ids: HashMap<Vec<u8>, u64>,
    /// compiling a string's handle (no conversion of string values to characters)
    pub(crate) str_raw: bool,
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
    /// arrays declared with a descending range (`mem [N:0]`), for the $readmem warning
    pub(crate) desc_arrays: std::collections::HashSet<u32>,
    /// some named event exists (`.triggered` needs the rewrite in `md_norm`)
    pub(crate) has_events: bool,
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
            inst_arrays: HashMap::new(),
            scope_module: HashMap::new(),
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
            str_ids: HashMap::new(),
            str_raw: false,
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
            desc_arrays: Default::default(),
            has_events: false,
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
            string: false,
            event: false,
        });
        self.prog.names.push((name, id));
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
            for &p in &self.scopes[sid].imports {
                if self.scopes[p].syms.contains_key(name) {
                    return self.scopes[p].param_ranges.get(name).copied();
                }
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
                    let e = ferrooxide_front::lower::hier_index_expr(n)?;
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
}
