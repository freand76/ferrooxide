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
}

#[derive(Default)]
pub(crate) struct PState {
    pub proc: u32,
    pub code: Vec<Instr>,
    /// loop stack: (break patches, continue patches)
    pub loops: Vec<(Vec<usize>, Vec<usize>)>,
    /// named blocks: (name, patches to block end)
    pub blocks: Vec<(String, Vec<usize>)>,
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
        modules.iter().filter(|m| !used.contains(m.name.as_str())).collect()
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
    for m in top_mods {
        let sid = c.instantiate(m, m.name.clone(), &[], None)?;
        c.tops.insert(m.name.clone(), sid);
    }
    c.codegen_jobs()?;
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
        let elems = depth.max(1);
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
        self.prog.procs.push(ProcDef { name, code: Vec::new(), class, fusible: false });
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

    pub(crate) fn mark_real(&mut self, s: u32) {
        let info = &mut self.prog.signals[s as usize];
        info.real = true;
        let (slot, n) = (info.slot as usize, rt::words(info.width) as usize * info.depth.max(1) as usize);
        for w in &mut self.prog.mem[slot..slot + n] {
            *w = 0;
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
        // parameters, in order
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
                let v = self.param_value(p, ov, sid)?;
                self.scopes[sid].syms.insert(p.name.clone(), v);
            }
        }
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
                }
            }
        }
        // 2. declarations (merged by name: `output [3:0] q; reg [3:0] q;`)
        struct Merged<'d> {
            dir: Option<Dir>,
            kind: NetKind,
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
                NetKind::Int(n) => {
                    if mg.range.is_none() {
                        (msb, lsb) = (n as i64 - 1, 0);
                    }
                    SigKind::Integer
                }
                _ => SigKind::Reg,
            };
            let is_real = mg.kind == NetKind::Real;
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
        // 4. everything else
        let mut genblk = 0;
        for it in items {
            match it {
                Item::Decl(_) | Item::Param(_) | Item::Subroutine(_) => {}
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
                        if !inst.dims.is_empty() {
                            return self.err("arrays of instances are not supported yet");
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
                Item::Gate { kind, terms, line, .. } => {
                    self.line = *line;
                    self.gate(kind, terms, sid, *line)?;
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
        if terms.len() < 2 {
            return self.err("gate needs at least two terminals");
        }
        let b = |e: &Expr| Box::new(e.clone());
        match kind {
            "not" | "buf" => {
                let input = terms.last().unwrap();
                for out in &terms[..terms.len() - 1] {
                    let rhs = if kind == "not" { Expr::Unary(UnOp::Not, b(input)) } else { input.clone() };
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
                n > 1 && i.kind == SigKind::Wire && i.depth == 0 && i.alias_of.is_none()
            })
            .map(|(s, _)| s)
            .collect();
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
            let mut v = self.sig(drv[0]).slot;
            for &d in &drv[1..] {
                let t = self.tmp(i.width);
                let b = self.sig(d).slot;
                self.emit(Instr::Bin { op: rt::BinOp::Resolve, d: t, a: v, b, w: i.width });
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
                Job::PortOut { lhs, child_sig, scope, line } => {
                    self.line = *line;
                    self.file = self.scopes[*scope].file.clone();
                    self.assign_proc(stmt::Lhs::Expr(lhs, *scope), stmt::Rhs::Sig(*child_sig), None)?;
                }
            }
        }
        Ok(())
    }
}
