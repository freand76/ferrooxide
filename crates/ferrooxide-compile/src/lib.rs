//! Elaboration (parameters, hierarchy, generate) and code generation to ferrooxide-rt bytecode.

mod expr;
mod konst;
mod lint;
mod stmt;

pub use konst::Const;

use ferrooxide_front::ast::*;
use ferrooxide_front::Error;
use ferrooxide_rt::{self as rt, Instr, Program, ProcClass, ProcDef, SigKind, SignalInfo, NONE};
use std::collections::{HashMap, HashSet};

pub(crate) type R<T> = Result<T, Error>;

/// Size of the shared scratch region for process temporaries, at the start of memory.
const SCRATCH_WORDS: u32 = 1 << 16;

#[derive(Clone, Debug)]
pub(crate) enum Sym {
    Sig(u32),
    Const(Const),
    /// a real parameter
    Real(f64),
}

/// A parameter value given by an override.
#[derive(Clone, Debug)]
pub(crate) enum PVal {
    Int(Const),
    Real(f64),
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
    pub(crate) call_depth: u32,
    warned: HashSet<String>,
    scratch_top: u32,
    scratch_saved: Vec<u32>,
    /// simulation precision (power of ten of seconds): the smallest of all modules
    pub(crate) gprec: i32,
    /// (array, element) -> alias signal created for port connections
    elem_alias: HashMap<(u32, u32), u32>,
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
    /// defparam overrides by full parameter path
    defparams: HashMap<String, PVal>,
    top_names: HashSet<String>,
    /// UDP name -> index of its runtime table
    pub(crate) udp_index: HashMap<String, u32>,
    /// counter for per-call storage of automatic functions and tasks
    pub(crate) auto_count: u32,
    /// nets of a special type (tri0, wand, ...): resolved even with one or no driver
    pub(crate) net_types: HashMap<u32, NetType>,
    /// nets with a pullup (true) or pulldown (false) gate
    pub(crate) pulls: HashMap<u32, bool>,
    /// nets with several continuous drivers -> their driver signals
    drivers: HashMap<u32, Vec<u32>>,
    /// while compiling a driver of a multi-driven net: net -> this driver's signal
    redirect: HashMap<u32, u32>,
}

/// Compile parsed modules into a runnable program. `top`: explicit top-level module(s).
pub fn compile(modules: &[Module], tops: &[String]) -> Result<(Program, Vec<String>), Error> {
    compile_lint(modules, tops, false).map(|(p, w, _)| (p, w))
}

/// Compile and, with `lint`, also collect lint findings: (program, warnings, lint findings).
pub fn compile_lint(modules: &[Module], tops: &[String], lint: bool) -> Result<(Program, Vec<String>, Vec<String>), Error> {
    let mut c = Compiler::new(modules);
    c.lint_enabled = lint;
    c.gprec = modules.iter().map(|m| m.timescale.map(|t| t.1).unwrap_or(0)).min().unwrap_or(0);
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
        modules.iter().filter(|m| !used.contains(m.name.as_str()) && m.udp.is_none()).collect()
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
    for m in top_mods {
        let sid = c.instantiate(m, m.name.clone(), &[], None)?;
        c.tops.insert(m.name.clone(), sid);
    }
    if let Some(e) = c.mem_error.take() {
        return Err(e);
    }
    c.codegen_jobs()?;
    c.resolve_disables();
    if let Some(e) = c.mem_error.take() {
        return Err(e);
    }
    c.prog.scratch_words = SCRATCH_WORDS;
    c.prog.precision = c.gprec;
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
            call_depth: 0,
            warned: HashSet::new(),
            scratch_top: 0,
            scratch_saved: Vec::new(),
            gprec: 0,
            elem_alias: HashMap::new(),
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
            top_names: HashSet::new(),
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

    pub(crate) fn emit(&mut self, i: Instr) -> usize {
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
        self.prog.procs.push(ProcDef { name, code: Vec::new(), class, parent: NONE, fusible: false });
        self.pb.push(PState { proc: id, ..Default::default() });
        self.scratch_saved.push(self.scratch_top);
        self.scratch_top = 0;
        id
    }

    pub(crate) fn end_proc(&mut self) {
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
            sid = self.scopes[sid].parent?;
        }
    }

    /// Resolve a hierarchical path a.b.c to a symbol.
    pub(crate) fn lookup_hier(&self, parts: &[String], sid: usize) -> Option<Sym> {
        let cur = self.hier_scope(&parts[..parts.len() - 1], sid)?;
        self.scopes[cur].syms.get(parts.last().unwrap()).cloned()
    }

    /// The scope named by a hierarchical path (instance names), searched from `sid` upwards and
    /// then from the top-level modules.
    pub(crate) fn hier_scope(&self, parts: &[String], sid: usize) -> Option<usize> {
        if parts.is_empty() {
            return Some(sid);
        }
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
            None => *self.tops.get(&parts[0])?,
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
                    _ => None,
                }
            }
            _ => None,
        }
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
        self.scopes[sid].tunit = m.timescale.map(|t| t.0).unwrap_or(0);
        self.scopes[sid].tprec = m.timescale.map(|t| t.1).unwrap_or(0);
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
        self.elab_items(&m.items, sid, true, &port_conns, parent_scope)?;
        self.file = saved_file;
        Ok(sid)
    }

    /// Evaluate the parameters of module `m` (with overrides) into scope `sid`, in order.
    fn eval_params(&mut self, m: &'a Module, overrides: &[(Option<String>, PVal)], sid: usize) -> R<()> {
        let mut pos = 0usize;
        for it in &m.items {
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
                    if let Some(d) = self.defparams.get(&format!("{}.{}", self.scopes[sid].name, p.name)) {
                        ov = Some(d.clone());
                    }
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
            let key = match base {
                Some(k) => k,
                None if self.top_names.contains(&target[0]) && target.len() > 1 => rel,
                None => format!("{}.{rel}", self.scopes[sid].name),
            };
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
        if p.real || (untyped && matches!(v, PVal::Real(_))) {
            let r = match v {
                PVal::Real(r) => r,
                PVal::Int(c) => c.to_i64().unwrap_or(0) as f64,
            };
            return Ok(Sym::Real(r));
        }
        let c = match v {
            PVal::Int(c) => c,
            PVal::Real(r) => Const::from_i64(r.round() as i64, 32, true),
        };
        Ok(Sym::Const(self.param_type(p, c, sid)?))
    }

    /// evaluate a parameter override in the instantiating scope
    fn pval(&mut self, e: &Expr, sid: usize) -> R<PVal> {
        if self.is_real(e, sid) {
            Ok(PVal::Real(self.const_real(e, sid)?))
        } else {
            Ok(PVal::Int(self.const_eval(e, sid)?))
        }
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
            signed: bool,
            range: Option<&'d Range>,
            dims: &'d [Range],
            init: Option<&'d Expr>,
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
                            mg.signed |= d.signed;
                            if d.range.is_some() {
                                mg.range = d.range.as_ref();
                            }
                            if !dl.dims.is_empty() {
                                mg.dims = &dl.dims;
                            }
                            if dl.init.is_some() {
                                mg.init = dl.init.as_ref();
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
                                    signed: d.signed,
                                    range: d.range.as_ref(),
                                    dims: &dl.dims,
                                    init: dl.init.as_ref(),
                                    line: dl.line,
                                },
                            );
                        }
                    }
                }
            }
        }
        for name in order {
            let mg = merged.remove(&name).unwrap();
            self.line = mg.line;
            let (mut msb, mut lsb) = match mg.range {
                Some((a, b)) => (self.const_int(a, sid)?, self.const_int(b, sid)?),
                None => (0, 0),
            };
            let mut signed = mg.signed;
            let kind = match mg.kind {
                NetKind::Wire => SigKind::Wire,
                NetKind::Integer => {
                    (msb, lsb) = (31, 0);
                    signed = true;
                    SigKind::Integer
                }
                NetKind::Time | NetKind::Real => {
                    (msb, lsb) = (63, 0);
                    SigKind::Reg
                }
                NetKind::Event => SigKind::Reg,
                NetKind::Int(n) => {
                    if mg.range.is_none() {
                        (msb, lsb) = (n as i64 - 1, 0);
                    }
                    SigKind::Integer
                }
                _ => SigKind::Reg,
            };
            let is_real = mg.kind == NetKind::Real;
            let is_event = mg.kind == NetKind::Event;
            let width = ((msb - lsb).abs() + 1) as u32;
            let (depth, arr_lo) = match mg.dims.len() {
                0 => (0, 0),
                1 => {
                    let (a, b) = (self.const_int(&mg.dims[0].0, sid)?, self.const_int(&mg.dims[0].1, sid)?);
                    (((a - b).abs() + 1) as u32, a.min(b))
                }
                _ => return self.err(format!("multi-dimensional array '{name}' is not supported yet")),
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
                                        let pfull = format!("{}.{}", self.scopes[ps].name, pn);
                                        let s = self.new_signal(pfull, width, false, 0, msb, lsb, 0, SigKind::Wire);
                                        self.scopes[ps].syms.insert(pn.clone(), Sym::Sig(s));
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
                            if kind != SigKind::Wire && self.sig(s).kind == SigKind::Wire {
                                // a variable output collapsed onto a net: starts as x, not z
                                let (slot, w) = (self.sig(s).slot as usize, self.sig(s).width);
                                let n = rt::nw(w);
                                for i in 0..n {
                                    self.prog.mem[slot + i] = self.prog.mem[slot + n + i];
                                }
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
                            Dir::Inout => self.warn(format!(
                                "inout port '{name}' connected to an expression: connection ignored"
                            )),
                        }
                        continue;
                    }
                }
            }
            let s = self.new_signal(full, width, signed, depth, msb, lsb, arr_lo, kind);
            if is_real {
                self.mark_real(s);
            }
            if is_event {
                self.zero_init(s);
            }
            if mg.net != NetType::Wire {
                self.net_types.insert(s, mg.net);
                if matches!(mg.net, NetType::Supply0 | NetType::Supply1) {
                    let d = if mg.net == NetType::Supply1 { "1" } else { "0" };
                    let rhs = Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: d.into(), fill: true });
                    let line = self.line;
                    self.jobs.push(Job::Assign { lhs: Expr::Ident(name.clone()), rhs, delay: None, scope: sid, line });
                }
            }
            self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
            if let Some(init) = mg.init {
                if kind == SigKind::Wire {
                    self.jobs.push(Job::Assign {
                        lhs: Expr::Ident(name.clone()),
                        rhs: init.clone(),
                        delay: None,
                        scope: sid,
                        line: self.line,
                    });
                } else if is_real {
                    let r = self.const_real(init, sid)?;
                    let slot = self.sig(s).slot as usize;
                    self.prog.mem[slot] = r.to_bits();
                    self.prog.mem[slot + 1] = 0;
                } else {
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
                let full = format!("{}.{}", self.scopes[sid].name, n);
                let s = self.new_signal(full, 1, false, 0, 0, 0, 0, SigKind::Wire);
                self.scopes[sid].syms.insert(n.clone(), Sym::Sig(s));
            }
        }
        // 3c. static tasks and functions, so hierarchical names can reach their variables
        for it in items {
            if let Item::Subroutine(f) = it {
                if !f.automatic {
                    self.func_instance(f, sid, &f.name)?;
                }
            }
        }
        // 4. everything else
        let mut genblk = 0;
        for it in items {
            match it {
                Item::Decl(_) | Item::Param(_) | Item::Subroutine(_) | Item::Defparam { .. } => {}
                Item::ContAssign { delay, lhs, rhs, line } => {
                    self.jobs.push(Job::Assign {
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                        delay: delay.clone(),
                        scope: sid,
                        line: *line,
                    });
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
                Item::Gate { kind, terms, line, dims, .. } => {
                    self.line = *line;
                    match dims {
                        None => self.gate(kind, terms, sid, *line)?,
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
                                self.gate(kind, &ts, sid, *line)?;
                            }
                        }
                    }
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

    fn gate(&mut self, kind: &str, terms: &[Expr], sid: usize, line: u32) -> R<()> {
        if terms.len() < 2 && !matches!(kind, "pullup" | "pulldown") {
            return self.err("gate needs at least two terminals");
        }
        let b = |e: &Expr| Box::new(e.clone());
        let z = || Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: "z".into(), fill: true });
        match kind {
            "pullup" | "pulldown" => {
                for t in terms {
                    let mut v = Vec::new();
                    self.lvalue_bases(t, sid, &mut v);
                    for s in v {
                        self.pulls.insert(s, kind == "pullup");
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
                self.jobs.push(Job::Assign { lhs: terms[0].clone(), rhs, delay: None, scope: sid, line });
            }
            "cmos" | "rcmos" => {
                // (out, in, ncontrol, pcontrol)
                if terms.len() != 4 {
                    return self.err(format!("{kind} needs four terminals"));
                }
                let p_side = Expr::Cond(b(&terms[3]), Box::new(z()), b(&terms[1]));
                let rhs = Expr::Cond(b(&terms[2]), b(&terms[1]), Box::new(p_side));
                self.jobs.push(Job::Assign { lhs: terms[0].clone(), rhs, delay: None, scope: sid, line });
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
                    self.jobs.push(Job::Assign { lhs: out.clone(), rhs, delay: None, scope: sid, line });
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
                self.jobs.push(Job::Assign { lhs: terms[0].clone(), rhs: e, delay: None, scope: sid, line });
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
    pub(crate) fn redir(&self, s: u32) -> u32 {
        self.redirect.get(&s).copied().unwrap_or(s)
    }

    /// Signals an lvalue writes (whole signals; part selects count as the signal).
    pub(crate) fn lvalue_bases(&self, e: &Expr, sid: usize, out: &mut Vec<u32>) {
        match e {
            Expr::Ident(n) => {
                if let Some(Sym::Sig(s)) = self.lookup(n, sid) {
                    out.push(s);
                }
            }
            Expr::Hier(p) => {
                if let Some(Sym::Sig(s)) = self.lookup_hier(p, sid) {
                    out.push(s);
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

    fn codegen_jobs(&mut self) -> R<()> {
        let jobs = std::mem::take(&mut self.jobs);
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
        // special nets nothing drives: their resolved value is constant
        let undriven: Vec<u32> = self
            .net_types
            .keys()
            .chain(self.pulls.keys())
            .copied()
            .filter(|s| !multi.contains(s))
            .collect();
        for s in undriven {
            let pull = match (self.net_types.get(&s), self.pulls.get(&s)) {
                (_, Some(&p)) => Some(p),
                (Some(NetType::Tri0), _) => Some(false),
                (Some(NetType::Tri1), _) => Some(true),
                _ => None,
            };
            if let Some(p) = pull {
                let i = self.sig(s).clone();
                let n = rt::nw(i.width);
                for k in 0..n {
                    let m = if k == n - 1 { rt::tmask(i.width) } else { !0 };
                    self.prog.mem[i.slot as usize + k] = if p { m } else { 0 };
                    self.prog.mem[i.slot as usize + n + k] = 0;
                }
            }
        }
        for job in &jobs {
            self.redirect.clear();
            for s in self.job_targets(job) {
                if multi.contains(&s) {
                    let i = self.sig(s).clone();
                    let k = self.drivers.get(&s).map(|v| v.len()).unwrap_or(0);
                    let d = self.new_signal(format!("{}$drv{k}", i.name), i.width, i.signed, 0, i.msb, i.lsb, 0, SigKind::Wire);
                    self.prog.names.pop(); // driver signals are not dumped
                    self.drivers.entry(s).or_default().push(d);
                    self.redirect.insert(s, d);
                }
            }
            self.codegen_job(job)?;
        }
        self.redirect.clear();
        // resolver processes: net = resolve(driver 0, driver 1, ...)
        let mut nets: Vec<(u32, Vec<u32>)> = std::mem::take(&mut self.drivers).into_iter().collect();
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
            for &d in &drv[1..] {
                let t = self.tmp(i.width);
                let b = self.sig(d).slot;
                self.emit(Instr::Bin { op: rop, d: t, a: v, b, w: i.width });
                v = t;
            }
            // pulled nets: undriven (z) bits read as 0 / 1
            let pull = match (self.pulls.get(&net), nt) {
                (Some(&p), _) => Some(p),
                (None, NetType::Tri0) => Some(false),
                (None, NetType::Tri1) => Some(true),
                _ => None,
            };
            if let Some(p) = pull {
                let c = self.const_slot(&Const::fill(if p { '1' } else { '0' }, i.width));
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

    fn codegen_job(&mut self, job: &Job<'a>) -> R<()> {
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
                            let mut reads = Vec::new();
                            self.reads_stmt(body, *scope, &mut reads);
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
