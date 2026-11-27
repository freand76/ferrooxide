//! The compiler: elaboration (`elab/`: hierarchy, parameters, declarations, generate, nets,
//! primitives, specify) and code generation (`codegen.rs`, `expr.rs`, `stmt.rs`) to
//! ferrooxide-rt bytecode. This file: the compiler state, scopes and name lookup, constant
//! evaluation.

mod class;
mod codegen;
mod elab;
mod enums;
mod expr;
mod foreach;
mod arrmeth;
mod assert;
mod heap;
mod konst;
mod lint;
mod random;
mod stmt;
mod strings;
mod types;
mod vif;

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
    /// the value of a string literal (or a concatenation of them, or a parameter with one)
    Str(Const),
    /// an empty positional override (`#( , 4)`): the parameter keeps its default
    Default,
    Real(f64),
    Type(Rc<SvType>),
    /// the value of an unpacked array parameter: its memory words
    Array(Rc<Vec<u64>>),
}

pub(crate) struct Scope<'a> {
    pub name: String,
    pub parent: Option<usize>,
    pub syms: HashMap<String, Sym>,
    pub children: HashMap<String, usize>,
    pub funcs: HashMap<String, &'a SubroutineDecl>,
    /// declared [msb:lsb] of parameters with a range (for bit and part selects)
    pub param_ranges: HashMap<String, (i64, i64)>,
    /// parameters whose value is a string literal: `$display(P)` prints them as text, as
    /// the reference does
    pub str_params: HashSet<String>,
    /// parameters declared `string` (typed as strings; the value is the text as a vector)
    pub string_params: HashSet<String>,
    /// the specparams declared here
    pub specparams: HashSet<String>,
    /// parameters whose value is `$` (unbounded, IEEE 1800-2017 6.20.2): x as a number,
    /// `$` in cycle ranges, true for $isunbounded
    pub unbounded_params: HashSet<String>,
    pub file: String,
    /// time unit and precision of the enclosing module (powers of ten of seconds)
    pub tunit: i32,
    pub tprec: i32,
    /// `default_nettype of the module (None: wire)
    pub nettype: Option<String>,
    /// `unconnected_drive of the module
    pub uncdrive: Option<bool>,
    /// package scopes whose names the scope sees after its own (modules, blocks with imports)
    pub imports: Vec<usize>,
    /// names imported one by one (`import P::x;` in a block): the package's scope
    pub name_imports: HashMap<String, usize>,
    /// a module instance's scope: the scope it is instantiated in (hierarchical names are
    /// searched upwards through it, IEEE 1800 23.8; simple names are not)
    pub inst_parent: Option<usize>,
}

enum Job<'a> {
    Proc { kind: ProcKind, body: &'a Stmt, scope: usize, line: u32 },
    Assign { lhs: Expr, rhs: Expr, delay: Option<Expr>, scope: usize, line: u32 },
    /// a concurrent assertion
    Assert { a: &'a ConcAssert, scope: usize },
    /// a variable initializer that is not a constant (a queue's): assigned once at time 0
    Init { lhs: Expr, rhs: Expr, scope: usize, line: u32 },
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
    /// private memory of a fork branch (`ProcDef::frame`)
    pub frame: Vec<(u32, u32)>,
}

pub(crate) struct FuncInst {
    pub scope: usize,
    pub ret: Option<u32>,
    /// (dir, signal) in declaration order
    pub ports: Vec<(Dir, u32)>,
    /// the memory words of its variables (lo, hi)
    pub mem: (u32, u32),
    /// a method: its `this` (the object's handle)
    pub this: Option<u32>,
}

/// A fork block whose branches are being compiled (`Code::forks`).
pub(crate) struct ForkCtx {
    pub st: *const Stmt,
    /// the scope the block is in
    pub sid: usize,
    /// the block's automatic variables (slot, words), private to each branch
    pub autos: Vec<(u32, u32)>,
    /// the block declares static variables
    pub statics: bool,
    /// the forks of the block that a recursive call reaches inside its branches start the
    /// same branch processes: their `Fork` and `Evict` instructions (process, index), patched
    /// with the branches when those are compiled
    pub again: Vec<(u32, usize)>,
    /// the memory (lo, words) of the subroutine call the branches read (the outer one)
    pub outer: Option<(u32, u32)>,
}

/// A recursive function compiled as a subroutine of a process: its body is placed after the
/// process's code; calls jump to it with a saved frame.
pub(crate) struct SubReq<'a> {
    /// the process nesting level (pb.len()) it belongs to
    pub level: usize,
    pub f: &'a SubroutineDecl,
    pub key: (usize, String),
    /// a call saves the function's variables too (automatic, or a constant function call),
    /// not only the temporaries
    pub whole: bool,
    /// CallSub instructions to point at the entry
    pub calls: Vec<usize>,
    /// SaveFrame instructions to complete with the ranges
    pub saves: Vec<usize>,
    pub entry: Option<usize>,
    /// memory allocated while compiling the body (block variables, temporaries)
    pub alloc: (u32, u32),
}

pub struct Compiler<'a> {
    pub prog: Program,
    pub(crate) scopes: Vec<Scope<'a>>,
    pub(crate) file: String,
    pub(crate) line: u32,
    /// simulation precision (power of ten of seconds): the smallest of all modules
    pub(crate) gprec: i32,
    dialect: Dialect,
    pub(crate) hier: Hier<'a>,
    pub(crate) diag: Diag,
    pub(crate) code: Code<'a>,
    pub(crate) subr: Subr<'a>,
    pub(crate) nets: Nets,
    pub(crate) sigs: Sigs,
    pub(crate) classes: class::Classes<'a>,
    pub(crate) enums: enums::Enums,
}

/// The module hierarchy: modules, instance scopes, packages, defparams.
#[derive(Default)]
pub(crate) struct Hier<'a> {
    pub(crate) mods: HashMap<&'a str, &'a Module>,
    /// arrays of instances and interface port arrays: (scope, name) -> (left, right); the
    /// elements are the children `name[i]` of the scope
    pub(crate) inst_arrays: HashMap<(usize, String), (i64, i64)>,
    /// the module (or interface) of each instance scope
    pub(crate) scope_module: HashMap<usize, &'a Module>,
    pub(crate) tops: HashMap<String, usize>,
    /// defparam overrides by full parameter path
    pub(crate) defparams: HashMap<String, PVal>,
    /// where each defparam is (file, line), for one whose target is not a parameter
    pub(crate) defparam_at: HashMap<String, (String, Line)>,
    /// parameters already evaluated (full paths), a defparam for one of them came too late,
    /// and whether this is the second elaboration
    pub(crate) params_done: HashSet<String>,
    pub(crate) late_defparam: bool,
    pub(crate) restarted: bool,
    pub(crate) top_names: HashSet<String>,
    /// package (and `$unit`) scopes by name
    pub(crate) pkg_scopes: HashMap<String, usize>,
}

/// Warnings and lint findings.
#[derive(Default)]
pub(crate) struct Diag {
    pub(crate) warnings: Vec<String>,
    /// messages of elaboration system tasks, printed when elaboration is done
    pub(crate) elab_msgs: Vec<String>,
    /// where `$error` / `$fatal` elaboration tasks were reached (elaboration fails)
    pub(crate) elab_errors: Vec<(String, u32)>,
    /// a `$fatal` elaboration task ended elaboration: later ones say nothing
    pub(crate) elab_fatal: bool,
    /// lint findings (only collected with `lint_enabled`)
    pub(crate) lint: Vec<String>,
    pub(crate) lint_seen: HashSet<String>,
    pub(crate) lint_enabled: bool,
    pub(crate) warned: HashSet<String>,
}

/// Code generation: jobs, processes, temporaries, constants, blocks, time-0 events.
#[derive(Default)]
pub(crate) struct Code<'a> {
    pub(crate) jobs: Vec<Job<'a>>,
    /// initializers of static properties of parameterized classes: compiled only if the
    /// class (a specialization, or the default one) is used
    pub(crate) lazy_inits: Vec<(u32, Job<'a>)>,
    /// variables procedural assignments write (for the check against continuous ones)
    pub(crate) proc_writes: HashSet<u32>,
    /// the assignment being compiled is a declaration's initializer (it may write a `const`)
    pub(crate) init_write: bool,
    /// the slot of the handle of the object being randomized: its generator gives the values
    pub(crate) rand_obj: Option<u32>,
    /// the classes whose `randomize()` is being compiled (rand handle properties nest them)
    pub(crate) rand_nest: Vec<u32>,
    /// hidden variables of assignments to parts of vector properties, by property select
    pub(crate) part_vars: HashMap<*const Expr, u32>,
    /// the fork blocks whose branches are being compiled
    pub(crate) forks: Vec<ForkCtx>,
    /// the process of static variable initializers (`Job::Init`) is open
    pub(crate) init_open: bool,
    pub(crate) consts: HashMap<Const, u32>,
    pub(crate) pb: Vec<PState>,
    pub(crate) scratch_top: u32,
    pub(crate) scratch_saved: Vec<u32>,
    /// texts interned at compile time -> handle (`Program::strings`)
    pub(crate) str_ids: HashMap<Vec<u8>, u64>,
    /// compiling a string's handle (no conversion of string values to characters)
    pub(crate) str_raw: bool,
    /// named blocks and task bodies for `disable`: (path, definition)
    pub(crate) block_reg: Vec<(String, rt::BlockDef)>,
    /// registry entries of the blocks being compiled (fork branches started inside them are
    /// stopped when they are disabled)
    pub(crate) open_blocks: Vec<usize>,
    /// named blocks open in constant evaluation (not registered): (path, jumps of the
    /// `disable`s of the block, patched to its end)
    pub(crate) const_blocks: Vec<(String, Vec<usize>)>,
    /// (enclosing scope, block statement) -> the scope of a block with declarations or
    /// imports, for the sensitivity of `@*`/`always_comb` (also unnamed blocks)
    pub(crate) block_scopes: HashMap<(usize, *const Stmt), usize>,
    /// (array method `with` expression, scope) -> the scope declaring its iterator and the
    /// iterator's element and index variables (`arrmeth.rs`)
    pub(crate) iter_scopes: HashMap<(*const Expr, usize), (usize, u32, u32)>,
    /// disable targets to resolve after code generation: (target, candidate paths, file, line)
    pub(crate) pending_disables: Vec<(u32, Vec<String>, String, u32)>,
    /// signals given an initial value before time 0 (declaration initializers, constant
    /// continuous assignments): they get a change event at time 0, as with Icarus
    pub(crate) init_touch: Vec<u32>,
    /// the same for nets with constant drivers (their events come first)
    pub(crate) init_touch_net: Vec<u32>,
    /// the scope of the job being compiled, and per process the scope it was created in
    pub(crate) job_scope: usize,
    pub(crate) proc_scope: Vec<usize>,
    /// UDP name -> index of its runtime table
    pub(crate) udp_index: HashMap<String, u32>,
    /// drive strength of the continuous assignment of each job (by index)
    pub(crate) job_strength: HashMap<usize, (u8, u8)>,
}

/// Functions and tasks: instances, inlining, recursion.
#[derive(Default)]
pub(crate) struct Subr<'a> {
    pub(crate) func_inst: HashMap<(usize, String), FuncInst>,
    pub(crate) call_depth: u32,
    /// functions being inlined: (definition scope, name, instance key)
    pub(crate) inline_stack: Vec<(usize, String, (usize, String))>,
    /// per open process: the length of `inline_stack` when it began
    pub(crate) proc_base: Vec<usize>,
    /// `static` variables of blocks in subroutines, by declaration: shared by all calls
    pub(crate) static_vars: HashMap<*const Declarator, u32>,
    /// subroutines whose body was compiled (a call inlines it): (declaring scope, name)
    pub(crate) compiled: HashSet<(usize, String)>,
    /// recursive functions compiled as subroutines (of the process at `level`)
    pub(crate) subs: Vec<SubReq<'a>>,
    /// counter for per-call storage of automatic functions and tasks
    pub(crate) auto_count: u32,
    /// read sets include signals read inside called functions (lint only)
    pub(crate) func_reads: std::cell::Cell<bool>,
    /// functions being followed by `func_reads` (recursion guard)
    pub(crate) func_reads_seen: std::cell::RefCell<Vec<(usize, String)>>,
    /// the result variable of the last call (an unpacked array result is read from it)
    pub(crate) last_ret: Option<u32>,
    /// a call whose unpacked array result is used as an array
    pub(crate) want_array: bool,
}

/// Nets: aliases and views, several drivers, pulls, bidirectional links, strengths.
#[derive(Default)]
pub(crate) struct Nets {
    /// (array, element) -> alias signal created for port connections
    pub(crate) elem_alias: HashMap<(u32, u32), u32>,
    /// views: another name for a signal with its own declared range and sign (a port of a
    /// different range or sign collapsed onto its connection, a net aliased to the source of
    /// a plain `assign`) -> the signal it stands for. Instructions refer to the view while
    /// compiling (selects and sign use its declaration) and to the signal afterwards.
    pub(crate) view_of: HashMap<u32, u32>,
    /// (view, driver signal) -> view of the driver (see `redir`)
    pub(crate) drv_views: HashMap<(u32, u32), u32>,
    /// variables made aliases by `alias_assigns` (a procedural store to one is an error)
    pub(crate) alias_vars: HashSet<u32>,
    /// nets of a special type (tri0, wand, ...): resolved even with one or no driver
    pub(crate) net_types: HashMap<u32, NetType>,
    /// nets that take one driver only (`uwire`, a user-defined nettype without a resolution
    /// function)
    pub(crate) single: HashSet<u32>,
    /// interconnect nets (only port connections use them)
    pub(crate) interconnect: HashSet<u32>,
    /// nets with a pullup (true) or pulldown (false) gate
    pub(crate) pulls: HashMap<u32, Vec<(u32, u32, bool)>>,
    /// nets with several continuous drivers -> their driver signals
    pub(crate) drivers: HashMap<u32, Vec<u32>>,
    /// while compiling a driver of a multi-driven net: net -> this driver's signal
    pub(crate) redirect: HashMap<u32, u32>,
    pub(crate) links: Vec<Link>,
    /// drive strength of the continuous assignments being created
    pub(crate) cur_strength: Option<(u8, u8)>,
    /// delay of the gate being instantiated
    pub(crate) cur_delay: Option<Expr>,
}

/// Facts about signals beyond `SignalInfo`, and the memory limit.
#[derive(Default)]
pub(crate) struct Sigs {
    /// the sampled copies of signals read by concurrent assertions
    pub(crate) sampled: HashMap<u32, u32>,
    /// unpacked array parameters: read-only arrays, constant in constant expressions
    pub(crate) const_arrays: HashSet<u32>,
    /// `const` variables: only their initializers write them (IEEE 1800-2017 6.20.6)
    pub(crate) konst: HashSet<u32>,
    /// events with a process for their nonblocking triggers (`->>`)
    pub(crate) nba_events: HashSet<u32>,
    /// 2-state variables (bit, int, byte, ...): stores turn x/z into 0, they start at 0
    pub(crate) two_state: HashSet<u32>,
    /// automatic variables (of automatic subroutines and automatic blocks)
    pub(crate) automatic: HashSet<u32>,
    /// memory limit (words) and the error raised when a declaration exceeds it
    pub(crate) mem_limit_words: u64,
    pub(crate) mem_error: Option<Error>,
    /// multi-dimensional arrays (stored flat): per dimension (lowest index, size, declared
    /// with a descending range)
    pub(crate) md_dims: HashMap<u32, Vec<(i64, u32, bool)>>,
    /// vectors with several packed dimensions (stored [W-1:0]): per dimension (left, right)
    pub(crate) packed_dims: HashMap<u32, Vec<(i64, i64)>>,
    /// arrays declared with a descending range (`mem [N:0]`), for the $readmem warning
    pub(crate) desc_arrays: std::collections::HashSet<u32>,
    /// some named event exists (`.triggered` needs the rewrite in `md_norm`)
    pub(crate) has_events: bool,
    /// the type of each signal (of its elements for an array), by signal id
    pub(crate) types: Vec<types::Type>,
    /// the layout of signals declared with a data type (struct members, assignment patterns)
    pub(crate) layout: HashMap<u32, Rc<types::Packed>>,
    /// the struct layout of the elements of queues and dynamic arrays of structs (member
    /// chains of their elements, `heap.rs`)
    pub(crate) elem_layout: HashMap<u32, Rc<types::Packed>>,
    /// a scratch variable per width for stores into a part of a queue element (`heap.rs`)
    pub(crate) elem_scratch: HashMap<u32, u32>,
    /// some signal has a struct type (member chains need the rewrite in `md_norm`)
    pub(crate) has_structs: bool,
    /// some struct has real or string members
    pub(crate) has_struct_values: bool,
    /// some queue or dynamic array exists (`q.size` needs the rewrite in `md_norm`)
    pub(crate) has_dyn: bool,
    /// the arrays that are elements of arrays (`Elem::nested`): their kind and element type
    pub(crate) nested: Vec<(types::DynKind, types::Elem)>,
    /// the arrays holding arrays elements of which are being changed (`DynHome::Elem`)
    pub(crate) outers: Vec<(heap::DynArr, u32)>,
}

/// Compile parsed modules into a runnable program. `top`: explicit top-level module(s).
pub fn compile(modules: &[Module], tops: &[String]) -> Result<(Program, Vec<String>), Error> {
    compile_lint(modules, tops, false).map(|(p, w, _)| (p, w))
}

/// Language rules where Verilog-2005 and SystemVerilog differ.
#[derive(Clone, Debug, Default)]
pub struct Dialect {
    /// Verilog-2005 (`-g2005`): a variable declaration initializer is an assignment at time 0,
    /// with a change event; in SystemVerilog (the default) it sets the value before time 0
    pub v2005: bool,
    /// `-gspecify`: specify path delays apply (otherwise they are ignored)
    pub specify: bool,
    /// `-gno-io-range-error`: a port declared without a range whose net or variable
    /// declaration has one is a warning, not an error (IEEE 1364-2005 12.3.3)
    pub io_range_warn: bool,
    /// `-Wno-anachronisms`: no warnings about such ports
    pub no_anachronisms: bool,
    /// `-P<top>.<param>=<value>`: parameter values of top-level modules (a number, a real or
    /// a string literal)
    pub top_params: Vec<(String, String)>,
    /// `--elab-only`: sources without a top-level module elaborate their packages and file
    /// scope (classes, for example) instead of being an error
    pub no_top_ok: bool,
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
        fn scan_mod<'b>(m: &'b Module, used: &mut HashSet<&'b str>) {
            scan(&m.items, used);
            m.nested.iter().for_each(|n| scan_mod(n, used));
        }
        for m in modules {
            scan_mod(m, &mut used);
        }
        // (nor one with a parameter that has no default: an instance must give its value)
        let no_default = |m: &Module| m.items.iter().any(|i| matches!(i, Item::Param(p) if p.value.is_none()));
        modules.iter().filter(|m| !used.contains(m.name.as_str()) && m.udp.is_none() && !m.interface && !m.package && !no_default(m)).collect()
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
    if top_mods.is_empty() && !(dialect.no_top_ok && modules.iter().any(|m| m.package)) {
        return Err(Error { msg: "no top-level module found".into(), file: String::new(), line: 0 });
    }
    // a keyword used as a name (IEEE 1800-2017 5.6.2); Verilog-2005 code keeps the names
    // 1800 added (`-g2005` or `begin_keywords "1364-2005")
    if !dialect.v2005 {
        let first = modules.iter().find_map(|m| Some((m, m.keyword_names.iter().min_by_key(|k| k.1)?)));
        if let Some((m, (n, l))) = first {
            let mut msg = format!("'{n}' is a keyword and cannot be a name");
            if ferrooxide_front::syntax::kw::Kw::from_str(n).is_some_and(|k| k.is_sv()) {
                msg.push_str(" in SystemVerilog (Verilog-2005 code: -g2005 or `begin_keywords \"1364-2005\")");
            }
            return Err(Error { msg, file: m.file.clone(), line: *l });
        }
    }
    // elaboration; a defparam found after its target parameter was evaluated restarts it once,
    // with all defparams known from the start
    let mut seed: Option<HashMap<String, PVal>> = None;
    let mut c = loop {
    let mut c = Compiler::new(modules);
    c.diag.lint_enabled = lint;
    c.dialect = dialect.clone();
    c.gprec = modules.iter().map(|m| m.timescale.map(|t| t.1).unwrap_or(0)).min().unwrap_or(0);
    if let Some(sd) = &seed {
        c.hier.defparams = sd.clone();
        c.hier.restarted = true;
    }
    c.declare_vifs(modules);
    c.hier.top_names = top_mods.iter().map(|m| m.name.clone()).collect();
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
        c.hier.pkg_scopes.insert(m.name.clone(), sid);
    }
    for m in &top_mods {
        let ovs = c.top_overrides(&m.name)?;
        let sid = c.instantiate(m, m.name.clone(), &ovs, None)?;
        c.hier.tops.insert(m.name.clone(), sid);
    }
    if c.hier.late_defparam && seed.is_none() {
        seed = Some(c.hier.defparams.clone());
        continue;
    }
    break c;
    };
    for m in &c.diag.elab_msgs {
        eprintln!("{m}");
    }
    if let Some((file, line)) = c.diag.elab_errors.first() {
        let n = c.diag.elab_errors.len();
        return Err(Error { msg: format!("{n} error(s) during elaboration ($error / $fatal)"), file: file.clone(), line: *line });
    }
    if let Some(e) = c.sigs.mem_error.take() {
        return Err(e);
    }
    c.codegen_jobs()?;
    c.check_defparams()?;
    c.init_events();
    c.resolve_disables();
    c.resolve_views();
    c.start_order();
    c.check_uncalled_subs()?;
    c.check_skipped_classes()?;
    // package variables are not dumped (as in the reference)
    if !c.hier.pkg_scopes.is_empty() {
        let pre: Vec<String> = c.hier.pkg_scopes.keys().map(|n| format!("{n}.")).collect();
        c.prog.names.retain(|(n, _)| !pre.iter().any(|p| n.starts_with(p.as_str())));
        let mut pk: Vec<String> = c.hier.pkg_scopes.keys().cloned().collect();
        pk.sort();
        c.prog.packages = pk;
    }
    if let Some(e) = c.sigs.mem_error.take() {
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
                // (only IEEE 1800 reserves it)
                if !ferrooxide_front::syntax::kw::Kw::from_str(n).is_some_and(|k| k.is_sv()) || !seen.insert(n) {
                    continue;
                }
                let msg = format!(
                    "{}:{l}: lint: '{n}' is a SystemVerilog keyword (IEEE 1800) used as a name; \
                     SystemVerilog tools reject it, rename it",
                    m.file
                );
                if c.diag.lint_seen.insert(msg.clone()) {
                    c.diag.lint.push(msg);
                }
            }
        }
        // in source order
        let key = |s: &String| -> (String, u64) {
            let mut it = s.splitn(3, ':');
            let f = it.next().unwrap_or("").to_string();
            (f, it.next().and_then(|l| l.parse().ok()).unwrap_or(0))
        };
        c.diag.lint.sort_by_key(key);
    }
    Ok((c.prog, c.diag.warnings, c.diag.lint))
}

impl<'a> Compiler<'a> {
    fn new(modules: &'a [Module]) -> Self {
        let mut mods = HashMap::new();
        for m in modules {
            mods.insert(m.name.as_str(), m);
        }
        Compiler {
            prog: Program { mem: vec![0; SCRATCH_WORDS as usize], ..Program::default() },
            scopes: Vec::new(),
            file: String::new(),
            line: 0,
            gprec: 0,
            dialect: Dialect::default(),
            hier: Hier {
                mods,
                ..Default::default()
            },
            diag: Diag::default(),
            code: Code {
                job_scope: usize::MAX,
                ..Default::default()
            },
            subr: Subr::default(),
            classes: Default::default(),
            enums: Default::default(),
            nets: Nets::default(),
            sigs: Sigs {
                mem_limit_words: std::env::var("FX_MAX_MEM_MB")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2048)
                    .min(32 * 1024)
                    * (1 << 20)
                    / 8,
                ..Default::default()
            },
        }
    }

    // ------------------------------------------------------------------ utilities
    pub(crate) fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(Error { msg: msg.into(), file: self.file.clone(), line: self.line })
    }

    /// Runs `f` at `line` (0: at the current location) and returns to the current location
    /// after it, so that code compiled inside (an inlined call) does not move it.
    pub(crate) fn at<T>(&mut self, line: u32, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.line;
        if line != 0 {
            self.line = line;
        }
        let r = f(self);
        self.line = saved;
        r
    }

    /// `file:line` of the current location in the source (through includes).
    pub(crate) fn loc(&self) -> String {
        ferrooxide_front::loc_str(&self.file, self.line)
    }

    pub(crate) fn warn(&mut self, msg: impl Into<String>) {
        let m = format!("{}: warning: {}", self.loc(), msg.into());
        if self.diag.warned.insert(m.clone()) {
            self.diag.warnings.push(m);
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
        let in_proc = self.code.pb.last().map(|p| p.proc != NONE).unwrap_or(false);
        let n = rt::words(w);
        if in_proc && self.code.scratch_top + n <= SCRATCH_WORDS {
            let s = self.code.scratch_top;
            self.code.scratch_top += n;
            return s;
        }
        self.alloc(w)
    }

    /// A temporary that stays live across suspension points (private to the process).
    pub(crate) fn ptmp(&mut self, w: u32) -> u32 {
        let s = self.alloc(w);
        self.note_frame(s, self.prog.mem.len() as u32);
        s
    }

    /// Memory [lo, hi) belongs to the fork branch being compiled (if one is).
    pub(crate) fn note_frame(&mut self, lo: u32, hi: u32) {
        if let Some(pb) = self.code.pb.last_mut() {
            if pb.proc != NONE && self.prog.procs[pb.proc as usize].class == rt::ProcClass::Spawned && hi > lo {
                pb.frame.push((lo, hi - lo));
            }
        }
    }

    pub(crate) fn const_slot(&mut self, c: &Const) -> u32 {
        let key = Const { signed: false, ..c.clone() };
        if let Some(&s) = self.code.consts.get(&key) {
            return s;
        }
        let s = self.alloc(c.w);
        let n = rt::nw(c.w);
        for i in 0..n {
            self.prog.mem[s as usize + i] = c.a[i];
            self.prog.mem[s as usize + n + i] = c.b[i];
        }
        self.code.consts.insert(key, s);
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
        if need > self.sigs.mem_limit_words {
            if self.sigs.mem_error.is_none() {
                self.sigs.mem_error = Some(Error {
                    msg: format!(
                        "'{}' needs {} MB of simulation memory, more than the limit of {} MB (set FX_MAX_MEM_MB to raise it)",
                        name,
                        need * 8 >> 20,
                        self.sigs.mem_limit_words * 8 >> 20
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
        let id = self.push_signal(SignalInfo {
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
        }, types::Type::vec(width, signed));
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
            str_params: HashSet::new(),
            unbounded_params: HashSet::new(),
            string_params: HashSet::new(),
            specparams: HashSet::new(),
            file,
            tunit: parent.map(|p| self.scopes[p].tunit).unwrap_or(0),
            tprec: parent.map(|p| self.scopes[p].tprec).unwrap_or(0),
            nettype: parent.and_then(|p| self.scopes[p].nettype.clone()),
            uncdrive: parent.and_then(|p| self.scopes[p].uncdrive),
            imports: Vec::new(),
            name_imports: HashMap::new(),
            inst_parent: None,
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
            self.sigs.two_state.insert(s);
            self.zero_init(s);
        }
    }

    pub(crate) fn mark_real(&mut self, s: u32) {
        let info = &mut self.prog.signals[s as usize];
        info.real = true;
        self.sigs.types[s as usize] = types::Type::Real;
        let info = &self.prog.signals[s as usize];
        let (slot, n) = (info.slot as usize, rt::words(info.width) as usize * info.depth.max(1) as usize);
        for w in &mut self.prog.mem[slot..slot + n] {
            *w = 0;
        }
    }

    /// The scope `p::` names from scope `sid`: a package or class; inside a class (or a
    /// specialization of it) its own name is that class (IEEE 1800-2017 8.25.1).
    pub(crate) fn prefix_scope(&self, p: &str, sid: usize) -> Option<usize> {
        if let Some((cs, name)) = self.enclosing_class_scope(sid) {
            if name == p {
                return Some(cs);
            }
        }
        self.hier.pkg_scopes.get(p).copied()
    }

    /// Declared range of a parameter named `name` visible from `sid`.
    pub(crate) fn param_range(&self, name: &str, mut sid: usize) -> Option<(i64, i64)> {
        if let Some((p, n)) = name.split_once("::") {
            return self.scopes[self.prefix_scope(p, sid)?].param_ranges.get(n).copied();
        }
        loop {
            if self.scopes[sid].syms.contains_key(name) {
                return self.scopes[sid].param_ranges.get(name).copied();
            }
            if let Some(&p) = self.scopes[sid].name_imports.get(name) {
                return self.scopes[p].param_ranges.get(name).copied();
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
        if let Some((p, n)) = name.split_once("::") {
            // `pkg::name`: in the package's scope only; `$flat:pkg::name` and the like name
            // `$flat:name` there
            if let Some((pre, p)) = p.strip_prefix('$').and_then(|r| r.split_once(':')) {
                return self.scopes[self.prefix_scope(p, sid)?].syms.get(&format!("${pre}:{n}")).cloned();
            }
            return self.scopes[self.prefix_scope(p, sid)?].syms.get(n).cloned();
        }
        loop {
            if let Some(s) = self.scopes[sid].syms.get(name) {
                return Some(s.clone());
            }
            if let Some(s) = self.scopes[sid].name_imports.get(name).and_then(|&p| self.scopes[p].syms.get(name)) {
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
            if let Some(f) = self.scopes[sid].name_imports.get(name).and_then(|&p| self.scopes[p].funcs.get(name).map(|f| (*f, p))) {
                return Some(f);
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
        // the rest of the path below a scope
        let down = |mut cur: usize| -> Option<usize> {
            for p in &parts[1..] {
                cur = *self.scopes[cur].children.get(p)?;
            }
            Some(cur)
        };
        // a scope by its number (an interface instance a virtual interface reaches, `vif.rs`)
        if let Some(n) = parts[0].strip_prefix("$scope#") {
            return down(n.parse().ok()?);
        }
        // inside a class (or a specialization of it), its name is that class
        if let Some((cs, name)) = self.enclosing_class_scope(sid) {
            if name == parts[0] {
                if let Some(r) = down(cs) {
                    return Some(r);
                }
            }
        }
        // find the scope for parts[0]: a child of this scope or of an enclosing one, or an
        // enclosing scope by its name or (an instance) its module's name, upwards through
        // the instances (IEEE 1800 23.8); the first one the rest of the path resolves in
        let mut s = Some(sid);
        while let Some(x) = s {
            if let Some(r) = self.scopes[x].children.get(&parts[0]).and_then(|&c| down(c)) {
                return Some(r);
            }
            let own = self.scopes[x].name.rsplit('.').next() == Some(parts[0].as_str())
                || (self.scopes[x].inst_parent.is_some() && self.hier.scope_module.get(&x).is_some_and(|m| m.name == parts[0]));
            if let Some(r) = own.then(|| down(x)).flatten() {
                return Some(r);
            }
            s = self.scopes[x].parent.or(self.scopes[x].inst_parent);
        }
        down(*self.hier.tops.get(&parts[0]).or_else(|| self.hier.pkg_scopes.get(&parts[0]))?)
    }

    /// A package (or `$unit`) scope.
    pub(crate) fn is_pkg_scope(&self, sid: usize) -> bool {
        self.hier.pkg_scopes.values().any(|&p| p == sid)
    }

    /// The signal a view stands for (the signal itself if it is not a view).
    pub(crate) fn canon(&self, mut s: u32) -> u32 {
        while let Some(&b) = self.nets.view_of.get(&s) {
            s = b;
        }
        s
    }

    pub(crate) fn sig(&self, id: u32) -> &SignalInfo {
        &self.prog.signals[id as usize]
    }

    // ------------------------------------------------------------------ constant evaluation
    pub(crate) fn const_eval(&mut self, e: &Expr, sid: usize) -> R<Const> {
        if let ExprKind::Num(n) = &e.kind {
            return Const::from_number(n).or_else(|m| self.err(m));
        }
        if let Some(c) = self.const_str_cmp(e, sid) {
            return Ok(c);
        }
        if let Some(m) = self.type_cmp(e, sid)? {
            return Ok(Const::from_i64(m as i64, 1, false));
        }
        if let ExprKind::Real(r) = &e.kind {
            return Ok(Const::from_i64(r.round() as i64, 64, false));
        }
        self.code.pb.push(PState { proc: NONE, ..Default::default() });
        let r = self.self_expr(e, sid);
        let r = r.and_then(|x| self.finish_subs().map(|_| x));
        let st = self.pop_pb();
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
        match &e.kind {
            ExprKind::Num(n) => Const::from_number(n).ok()?.to_i64(),
            ExprKind::Ident(n) => match self.lookup(n, sid)? {
                Sym::Const(c) => c.to_i64(),
                _ => None,
            },
            ExprKind::Binary(op, a, b) => {
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
            ExprKind::Unary(UnOp::Neg, a) => Some(-self.const_value(a, sid)?),
            // a member of a struct parameter (`P.m`, at most 64 bits)
            ExprKind::Member(..) if self.sigs.has_structs => {
                let Some(Ok(w)) = self.struct_walk(e, sid, false) else { return None };
                let (x, signed) = match w.expr.kind {
                    ExprKind::SysCall(f, mut a) if f == "$signed" && a.len() == 1 => (a.pop().unwrap(), true),
                    k => (Expr::from(k), false),
                };
                let ExprKind::IndexedRange(b, off, width, true) = &x.kind else { return None };
                let ExprKind::Ident(n) = &b.kind else { return None };
                let Some(Sym::Const(c)) = self.lookup(n, sid) else { return None };
                let (off, width) = (self.const_value(off, sid)?, self.const_value(width, sid)?);
                if !(1..=64).contains(&width) || off < 0 || off + width > c.w as i64 {
                    return None;
                }
                let mut v = Const::zero(width as u32);
                for k in 0..width as u32 {
                    let (a, b) = c.bit(off as u32 + k);
                    v.a[0] |= a << k;
                    v.b[0] |= b << k;
                }
                v.with_sign(signed).to_i64()
            }
            _ => None,
        }
    }

    /// Whether `e` is a constant expression (IEEE 1800-2017 11.2.1): literals, parameters,
    /// constant system and user function calls of constant arguments; no variables or nets.
    /// (Names not found are left to the error at their use.)
    pub(crate) fn is_const_expr(&self, e: &Expr, sid: usize) -> bool {
        let all = |c: &Self, v: &[Expr]| v.iter().all(|x| c.is_const_expr(x, sid));
        match &e.kind {
            ExprKind::Ident(n) => !matches!(self.lookup(n, sid), Some(Sym::Sig(s)) if !self.sigs.const_arrays.contains(&s)),
            ExprKind::Hier(p) => !matches!(self.lookup_hier(p, sid), Some(Sym::Sig(s)) if !self.sigs.const_arrays.contains(&s)),
            ExprKind::Unary(_, a) | ExprKind::Member(a, _) => self.is_const_expr(a, sid),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => self.is_const_expr(a, sid) && self.is_const_expr(b, sid),
            ExprKind::Cond(a, b, c) | ExprKind::Range(a, b, c) | ExprKind::IndexedRange(a, b, c, _) => {
                self.is_const_expr(a, sid) && self.is_const_expr(b, sid) && self.is_const_expr(c, sid)
            }
            ExprKind::Concat(v) => all(self, v),
            ExprKind::Repl(n, v) => self.is_const_expr(n, sid) && all(self, v),
            ExprKind::Call(_, args) => all(self, args),
            // queries of a declaration are constant whatever they name
            ExprKind::SysCall(n, _)
                if matches!(
                    n.as_str(),
                    "$bits" | "$size" | "$left" | "$right" | "$low" | "$high" | "$increment" | "$dimensions" | "$unpacked_dimensions" | "$typename" | "$isunbounded"
                ) =>
            {
                true
            }
            ExprKind::SysCall(n, _) if matches!(n.as_str(), "$time" | "$stime" | "$realtime" | "$random" | "$urandom" | "$urandom_range" | "$dist_uniform" | "$dist_normal" | "$dist_exponential" | "$dist_poisson" | "$dist_chi_square" | "$dist_t" | "$dist_erlang" | "$__process_self" | "$__process_status") => false,
            ExprKind::SysCall(_, args) => all(self, args),
            _ => true,
        }
    }

    /// A part-select bound or width (`[m:l]`, `[b +: w]`), which must be constant: None when
    /// it has x/z bits.
    pub(crate) fn select_bound(&mut self, e: &Expr, sid: usize) -> R<Option<i64>> {
        if !self.is_const_expr(e, sid) {
            return self.err("a part-select bound is not a constant expression (use an indexed part-select, [b +: w])");
        }
        self.const_index(e, sid)
    }

    /// A constant index: None when it has x/z bits (such a select reads x, writes nothing).
    pub(crate) fn const_index(&mut self, e: &Expr, sid: usize) -> R<Option<i64>> {
        self.no_real_index(e, sid)?;
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
