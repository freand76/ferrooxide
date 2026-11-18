//! Lowering: the syntax tree (`crate::syntax::tree`) to the AST the compiler reads
//! (`crate::ast`). It does what the old parser does besides parsing, walking the tree in
//! source order: type and variable tables, struct member chains, assignment patterns, enum
//! constants and methods, array queries, packages and `$unit`, time literals, the expansions
//! of foreach, case-inside, unique/priority, `iff`, `++`, hierarchical-name segments and
//! unnamed instances. What has nothing behind it fails with "... is not supported" at its
//! line. `FX_FRONT=new` selects it (see `crate::parse_files`).

mod class;
mod expr;
mod items;
mod stmt;
mod types;

use crate::ast::*;
pub use types::{Package, SvType};
use crate::syntax::tree as t;
use crate::{Error, Options, Result};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub(crate) struct Lower {
    file: String,
    timescale: Option<(i32, i32)>,
    default_nettype: Option<String>,
    unconnected_drive: Option<bool>,
    /// counter for names of unnamed instances (per file)
    unnamed: u32,
    /// typedefs in scope (file scope and packages carry over to the next file)
    types: HashMap<String, Rc<SvType>>,
    /// declared names of the current module with their type and unpacked dimensions
    vars: HashMap<String, (Rc<SvType>, Vec<Range>)>,
    /// the data type of the declaration being lowered
    decl_type: Option<Rc<SvType>>,
    /// items produced while lowering a declaration (enum constants)
    pending: Vec<Item>,
    /// typedefs lowered ahead of their place for a forward declaration (node addresses)
    early: HashSet<usize>,
    /// items at file scope ($unit): added to every module that follows
    unit_items: Vec<Item>,
    /// variables, functions and tasks at file scope (in the `$unit` module) and the types
    unit_vars: Vec<Item>,
    unit_var_types: HashMap<String, (Rc<SvType>, Vec<Range>)>,
    /// packages imported at file scope
    unit_pkgs: Vec<String>,
    unit_name: String,
    /// `timeunit` / `timeprecision` at file scope and in the current module
    file_tu: (Option<i32>, Option<i32>),
    local_tu: (Option<i32>, Option<i32>),
    /// in a package or file-scope items: the scope's name and its parameters
    type_scope: Option<(String, HashSet<String>)>,
    /// modules made for packages (added after the file's modules)
    pkg_modules: Vec<Module>,
    /// names that are SystemVerilog keywords (lint), until the next module ends
    kw_names: Vec<(String, Line)>,
    packages: HashMap<String, Rc<Package>>,
    /// packages the current module (or package) imports or refers to
    mod_pkgs: Vec<String>,
    /// lowering a package: its imports, exports and the names it refers to
    pkg_ctx: Option<PkgCtx>,
    /// the return type of the function being lowered (for `return '{...}`)
    modports: Vec<String>,
    /// lowering a function or task body
    in_subroutine: bool,
    /// in a `module automatic` / `package automatic`: subroutines are automatic by default
    default_automatic: bool,
    /// lowering the items of a program
    in_program: bool,
    /// classes declared so far: the names their methods reach through `this`
    classes: HashMap<String, Rc<class::ClassNames>>,
}

/// While lowering a package: what its `export` items need.
#[derive(Default)]
struct PkgCtx {
    /// `import p::n`: (p, n, the package declaring n)
    explicit: Vec<(String, String, String)>,
    /// `import p::*`
    wildcard: Vec<String>,
    /// type names that came in through imports (not declared in the package)
    imported_types: HashSet<String>,
    /// names the package refers to
    refs: HashSet<String>,
    /// `export p::n` items, `*` for wildcards
    exports: Vec<(String, String, t::Loc)>,
}

/// Lower the parsed files (one compilation unit unless `separate_units`).
pub fn lower_files(files: &[t::SourceFile], opts: &Options) -> Result<Vec<Module>> {
    let mut l = Lower::new();
    let mut modules = Vec::new();
    for (k, f) in files.iter().enumerate() {
        if opts.separate_units && k > 0 {
            modules.extend(l.unit_module());
            l.timescale = None;
            l.default_nettype = None;
            l.unconnected_drive = None;
            l.types = HashMap::new();
            l.unit_vars.clear();
            l.unit_pkgs.clear();
            l.unit_items.clear();
            l.unit_var_types = HashMap::new();
            l.file_tu = (None, None);
            l.unit_name = format!("$unit#{k}");
        }
        l.file = f.name.clone();
        l.unnamed = 0;
        l.kw_names.clear();
        l.mod_pkgs.clear();
        l.local_tu = (None, None);
        modules.extend(l.source(&f.items)?);
    }
    modules.extend(l.unit_module());
    Ok(modules)
}

impl Lower {
    fn new() -> Lower {
        Lower {
            file: String::new(),
            timescale: None,
            default_nettype: None,
            unconnected_drive: None,
            unnamed: 0,
            types: HashMap::new(),
            vars: HashMap::new(),
            decl_type: None,
            pending: Vec::new(),
            early: HashSet::new(),
            unit_items: Vec::new(),
            unit_vars: Vec::new(),
            unit_var_types: HashMap::new(),
            unit_pkgs: Vec::new(),
            unit_name: "$unit".into(),
            file_tu: (None, None),
            local_tu: (None, None),
            type_scope: None,
            pkg_modules: Vec::new(),
            kw_names: Vec::new(),
            packages: HashMap::new(),
            mod_pkgs: Vec::new(),
            pkg_ctx: None,
            modports: Vec::new(),
            in_subroutine: false,
            default_automatic: false,
            in_program: false,
            classes: HashMap::new(),
        }
    }

    pub(crate) fn err<X>(&self, loc: t::Loc, msg: impl Into<String>) -> Result<X> {
        Err(Error { msg: msg.into(), file: self.file.clone(), line: loc.line })
    }

    pub(crate) fn unsupported<X>(&self, loc: t::Loc, what: &str) -> Result<X> {
        self.err(loc, format!("{what} is not supported"))
    }

    /// A name read from the source: SystemVerilog keywords used as names are noted (lint).
    pub(crate) fn name(&mut self, n: &str, loc: t::Loc) -> String {
        self.note_ref(n);
        if SV_RESERVED.contains(&n) {
            self.kw_names.push((n.to_string(), loc.line));
        }
        n.to_string()
    }

    fn directive(&mut self, d: &t::Directive) {
        match d {
            t::Directive::Timescale(u, p) => self.timescale = Some((*u, *p)),
            t::Directive::DefaultNettype(n) => self.default_nettype = if n == "wire" { None } else { Some(n.clone()) },
            t::Directive::UnconnectedDrive(d) => self.unconnected_drive = *d,
        }
    }

    // ---------------------------------------------------------------- source

    fn source(&mut self, descs: &[t::Description]) -> Result<Vec<Module>> {
        let mut mods = Vec::new();
        // file-scope items, for forward typedefs (the rest of the file scope)
        let file_items: Vec<&t::Item> = descs.iter().filter_map(|d| if let t::Description::Item(i) = d { Some(i) } else { None }).collect();
        let mut nitem = 0;
        for d in descs {
            match d {
                t::Description::Directive(d, _) => self.directive(d),
                t::Description::Module(m) => match m.kind {
                    _ if m.extern_ => return self.unsupported(m.loc, "an extern module declaration"),
                    t::ModuleKind::Module | t::ModuleKind::Macromodule | t::ModuleKind::Interface | t::ModuleKind::Program => {
                        mods.push(self.module(m, false)?)
                    }
                    t::ModuleKind::Checker => return self.unsupported(m.loc, "a checker"),
                },
                t::Description::Udp(u) => mods.push(self.primitive(u)?),
                t::Description::Package(p) => self.package(p)?,
                t::Description::Config(c) => return self.unsupported(c.loc, "a configuration"),
                t::Description::Item(it) => {
                    nitem += 1;
                    let rest: Vec<&t::Item> = file_items[nitem..].to_vec();
                    self.unit_item(it, &rest)?;
                }
            }
        }
        mods.append(&mut self.pkg_modules);
        Ok(mods)
    }

    /// An item at file scope ($unit).
    fn unit_item(&mut self, it: &t::Item, rest: &[&t::Item]) -> Result<()> {
        match it {
            t::Item::Import(v, _) => self.import(v, true),
            t::Item::Empty(_) => Ok(()),
            t::Item::Directive(d, _) => {
                self.directive(d);
                Ok(())
            }
            t::Item::Timeunits { unit, precision, loc } => {
                let (u, p) = self.timeunits(*unit, *precision, *loc)?;
                // (after a `timescale they are ignored, as in the reference)
                if self.timescale.is_none() {
                    self.file_tu = (u.or(self.file_tu.0), p.or(self.file_tu.1));
                }
                Ok(())
            }
            t::Item::Typedef(_) | t::Item::Subroutine(_) => self.unit_scoped(it, rest),
            t::Item::Class(c) => {
                // in the `$unit` scope, as file-scope functions
                let c = self.class_decl(c)?;
                self.unit_vars.push(Item::Class(Rc::new(c)));
                Ok(())
            }
            t::Item::Param(p) if matches!(p.kw, Some(crate::syntax::kw::Kw::Parameter | crate::syntax::kw::Kw::Localparam)) => {
                self.unit_scoped(it, rest)
            }
            t::Item::Net(_) | t::Item::Var(_) => {
                // a file-scope variable: one instance, shared by all modules
                let before: HashSet<String> = self.vars.keys().cloned().collect();
                let mut v = Vec::new();
                self.module_item(it, rest, &mut v)?;
                for (k, t) in &self.vars {
                    if !before.contains(k) {
                        self.unit_var_types.insert(k.clone(), t.clone());
                    }
                }
                for it in v {
                    if matches!(it, Item::Decl(_)) {
                        self.unit_vars.push(it);
                    } else {
                        self.unit_items.push(it);
                    }
                }
                Ok(())
            }
            other => self.unsupported(item_loc(other), &format!("{} at file scope", item_what(other))),
        }
    }

    /// Typedefs, parameters, functions and tasks at file scope: types refer to the
    /// file scope's parameters; subroutines run in the `$unit` scope.
    fn unit_scoped(&mut self, it: &t::Item, rest: &[&t::Item]) -> Result<()> {
        let names = self.unit_items.iter().filter_map(|i| if let Item::Param(p) = i { Some(p.name.clone()) } else { None }).collect();
        self.type_scope = Some((self.unit_name.clone(), names));
        let mut v = Vec::new();
        let r = self.module_item(it, rest, &mut v);
        self.type_scope = None;
        r?;
        for it in v {
            if matches!(it, Item::Subroutine(_)) {
                self.unit_vars.push(it);
            } else {
                self.unit_items.push(it);
            }
        }
        Ok(())
    }

    /// After the last file: the `$unit` module holding the file-scope variables, if any.
    fn unit_module(&mut self) -> Option<Module> {
        if self.unit_vars.is_empty() && !self.unit_items.iter().any(|i| matches!(i, Item::Param(_))) {
            return None;
        }
        let mut items = self.unit_items.clone();
        items.append(&mut self.unit_vars);
        self.local_tu = (None, None);
        Some(self.package_module(self.unit_name.clone(), items, self.unit_pkgs.clone()))
    }

    /// `timeunit` / `timeprecision` values as powers of ten (only 1, 10 or 100 units).
    pub(crate) fn timeunits(&self, unit: Option<(f64, i32)>, prec: Option<(f64, i32)>, loc: t::Loc) -> Result<(Option<i32>, Option<i32>)> {
        let one = |x: Option<(f64, i32)>| -> Result<Option<i32>> {
            match x {
                None => Ok(None),
                Some((v, e)) if v == 1.0 || v == 10.0 || v == 100.0 => Ok(Some(e + v.log10().round() as i32)),
                Some(_) => self.err(loc, "expected a time literal of 1, 10 or 100 units"),
            }
        };
        Ok((one(unit)?, one(prec)?))
    }

    /// The time unit and precision in effect.
    pub(crate) fn eff_timescale(&self) -> Option<(i32, i32)> {
        let base = self.timescale.or(match self.file_tu {
            (None, None) => None,
            (u, p) => Some((u.unwrap_or(0), p.unwrap_or(0))),
        });
        match self.local_tu {
            (None, None) => base,
            (u, p) => Some((u.or(base.map(|b| b.0)).unwrap_or(0), p.or(base.map(|b| b.1)).unwrap_or(0))),
        }
    }

    /// A time literal in units of the current module, rounded to its precision, as a real.
    pub(crate) fn time_literal(&self, v: f64, e: i32) -> Expr {
        let (unit, prec) = self.eff_timescale().unwrap_or((0, 0));
        let scale = |v: f64, k: i32| if k >= 0 { v * 10f64.powi(k) } else { v / 10f64.powi(-k) };
        let steps = scale(v, e - prec).round();
        Expr::Real(scale(steps, prec - unit))
    }

    fn package_module(&self, name: String, items: Vec<Item>, imports: Vec<String>) -> Module {
        Module {
            name,
            timescale: self.eff_timescale(),
            default_nettype: self.default_nettype.clone(),
            unconnected_drive: None,
            file: self.file.clone(),
            line: 0,
            ports: vec![],
            items,
            udp: None,
            keyword_names: vec![],
            interface: false,
            iface_ports: vec![],
            modports: vec![],
            package: true,
            imports,
            program: false,
            nested: vec![],
            is_nested: false,
        }
    }

    // ---------------------------------------------------------------- modules

    /// A module, interface or program; `nested`: declared inside another one, whose names
    /// (types, variables, imports, time unit) it sees.
    fn module(&mut self, m: &t::ModuleDecl, nested: bool) -> Result<Module> {
        let saved_auto = std::mem::replace(&mut self.default_automatic, m.lifetime == Some(t::Lifetime::Automatic));
        let r = self.module_inner(m, nested);
        self.default_automatic = saved_auto;
        r
    }

    fn module_inner(&mut self, m: &t::ModuleDecl, nested: bool) -> Result<Module> {
        let line = m.loc.line;
        let interface = m.kind == t::ModuleKind::Interface;
        let program = m.kind == t::ModuleKind::Program;
        let saved_program = std::mem::replace(&mut self.in_program, program);
        let mname = self.name(&m.name, m.loc);
        let mut iface_ports = Vec::new();
        self.modports.clear();
        let saved_types = self.types.clone();
        self.local_tu = (None, None);
        if !nested {
            self.vars.clear();
            self.vars.extend(self.unit_var_types.iter().map(|(k, v)| (k.clone(), v.clone())));
            self.mod_pkgs.clear();
        }
        let mut items = Vec::new();
        self.import(&m.imports, false)?;
        let mut ports = Vec::new();
        if let Some(params) = &m.params {
            self.param_ports(params, &mut items)?;
        }
        match &m.ports {
            t::Ports::None => {}
            t::Ports::Wildcard => return self.unsupported(m.loc, "a `.*` port list"),
            t::Ports::NonAnsi(v) => {
                for p in v {
                    match (&p.name, &p.expr) {
                        (None, None) => ports.push(String::new()),
                        (None, Some(t::Expr { kind: t::ExprKind::Ident(n), loc })) => ports.push(self.name(n, *loc)),
                        _ => return self.unsupported(p.loc, "a port expression"),
                    }
                }
            }
            t::Ports::Ansi(v) => self.ansi_ports(v, &mut ports, &mut iface_ports, &mut items)?,
        }
        let refs: Vec<&t::Item> = m.items.iter().filter(|i| !matches!(i, t::Item::Module(_))).collect();
        self.item_list(&refs, &mut items)?;
        let mut nested_mods = Vec::new();
        for it in &m.items {
            if let t::Item::Module(n) = it {
                nested_mods.push(self.nested_module(m, n)?);
            }
        }
        self.in_program = saved_program;
        self.types = saved_types;
        // names of the packages it uses and of the file scope are found in their scopes
        let pkgs = std::mem::take(&mut self.mod_pkgs);
        let mut imports = pkgs;
        imports.extend(self.unit_pkgs.iter().cloned());
        imports.push(self.unit_name.clone());
        Ok(Module {
            name: mname,
            timescale: self.eff_timescale(),
            default_nettype: self.default_nettype.clone(),
            unconnected_drive: self.unconnected_drive,
            file: self.file.clone(),
            line,
            ports,
            items,
            udp: None,
            keyword_names: std::mem::take(&mut self.kw_names),
            interface,
            iface_ports,
            modports: std::mem::take(&mut self.modports),
            package: false,
            imports,
            program,
            nested: nested_mods,
            is_nested: nested,
        })
    }

    /// A module or program declared in module `outer`: lowered with the enclosing module's
    /// names in scope, which are restored afterwards.
    fn nested_module(&mut self, outer: &t::ModuleDecl, n: &t::ModuleDecl) -> Result<Module> {
        if n.extern_ {
            return self.unsupported(n.loc, "an extern module declaration");
        }
        if n.kind == t::ModuleKind::Checker {
            return self.unsupported(n.loc, "a checker");
        }
        if outer.kind == t::ModuleKind::Program {
            return self.err(n.loc, "module, program, or interface declarations are not allowed in program blocks");
        }
        if outer.kind == t::ModuleKind::Interface && !matches!(n.kind, t::ModuleKind::Program | t::ModuleKind::Interface) {
            return self.err(n.loc, "module declarations are not allowed in interfaces");
        }
        // the enclosing module's time unit and precision apply unless it declares its own
        let eff = self.eff_timescale();
        let saved_ts = std::mem::replace(&mut self.timescale, eff);
        let saved_tu = self.local_tu;
        let saved_vars = self.vars.clone();
        let saved_pkgs = self.mod_pkgs.clone();
        let saved_modports = std::mem::take(&mut self.modports);
        let saved_kw = std::mem::take(&mut self.kw_names);
        let r = self.module(n, true);
        self.timescale = saved_ts;
        self.local_tu = saved_tu;
        self.vars = saved_vars;
        self.mod_pkgs = saved_pkgs;
        self.modports = saved_modports;
        self.kw_names = saved_kw;
        r
    }

    /// The `#(...)` parameter port list of a module.
    fn param_ports(&mut self, params: &[t::ParamDecl], items: &mut Vec<Item>) -> Result<()> {
        use crate::syntax::kw::Kw;
        let mut local = false;
        for p in params {
            match p.kw {
                Some(Kw::Parameter) => local = false,
                Some(Kw::Localparam) => local = true,
                _ => {}
            }
            match &p.kind {
                t::ParamKind::Type { assigns } => {
                    for (n, d, loc) in assigns {
                        let it = self.type_param(local, n, d.as_ref(), *loc)?;
                        items.push(it);
                    }
                }
                t::ParamKind::Value { ty, assigns } => {
                    let pt = self.param_type(ty)?;
                    for d in assigns {
                        let it = self.param_assign(local, &pt, d)?;
                        items.append(&mut self.pending);
                        items.push(Item::Param(it));
                    }
                }
            }
        }
        Ok(())
    }

    /// An ANSI port list: what a port omits it inherits from the one before.
    fn ansi_ports(
        &mut self,
        v: &[t::AnsiPort],
        ports: &mut Vec<String>,
        iface_ports: &mut Vec<(String, String, Option<Range>, Option<String>)>,
        items: &mut Vec<Item>,
    ) -> Result<()> {
        let mut cur: Option<Decl> = None;
        let mut cur_iface: Option<(String, Option<String>)> = None;
        for p in v {
            let pline = p.loc.line;
            if p.expr.is_some() {
                return self.unsupported(p.loc, "an explicit port `.name(expr)`");
            }
            let iface = match &p.header {
                t::PortHeader::Interface { name: Some(n), modport } if p.dir.is_none() => Some((n.clone(), modport.clone())),
                t::PortHeader::Interface { .. } => return self.unsupported(p.loc, "a generic interface port"),
                t::PortHeader::None if p.dir.is_none() && cur_iface.is_some() => cur_iface.clone(),
                _ => None,
            };
            if let Some((iface, modport)) = iface {
                let pname = self.name(&p.name, p.loc);
                let dims = match p.dims.as_slice() {
                    [] => None,
                    [d] => Some(self.dim(d, p.loc)?),
                    _ => return self.unsupported(p.loc, "a multi-dimensional interface port array"),
                };
                if p.default.is_some() {
                    return self.unsupported(p.loc, "a default value of an interface port");
                }
                iface_ports.push((pname.clone(), iface.clone(), dims, modport.clone()));
                ports.push(pname);
                cur = None;
                cur_iface = Some((iface, modport));
                continue;
            }
            cur_iface = None;
            let dir = match p.dir {
                None => None,
                Some(t::Dir::Input) => Some(Dir::Input),
                Some(t::Dir::Output) => Some(Dir::Output),
                Some(t::Dir::Inout) => Some(Dir::Inout),
                Some(_) => return self.unsupported(p.loc, "a ref port"),
            };
            match (&p.header, dir) {
                (t::PortHeader::Data { net, var, ty }, Some(_)) => cur = Some(self.decl_head(dir, *net, *var, ty, None, None, p.loc)?),
                (_, Some(_)) => {
                    let ty = t::DataType { kind: t::TypeKind::Implicit, signed: None, packed: vec![], loc: p.loc };
                    cur = Some(self.decl_head(dir, None, false, &ty, None, None, p.loc)?);
                }
                _ if cur.is_none() => return self.err(p.loc, "expected port direction"),
                (t::PortHeader::Data { net, var, ty }, None) => {
                    // a new type without a direction: inherits the direction
                    let dir = cur.as_ref().unwrap().dir;
                    cur = Some(self.decl_head(dir, *net, *var, ty, None, None, p.loc)?);
                }
                _ => {}
            }
            let pname = self.name(&p.name, p.loc);
            let (dims, heap) = self.declarator_dims(&p.dims, p.loc)?;
            let init = match &p.default {
                Some(e) => Some(self.expr(e)?),
                None => None,
            };
            self.declare_var_heap(&pname, &dims, heap.as_ref());
            items.append(&mut self.pending);
            let mut d = cur.clone().unwrap();
            d.line = pline;
            d.names = vec![Declarator { name: pname.clone(), dims, heap, init, line: pline }];
            items.push(Item::Decl(d));
            ports.push(pname);
        }
        Ok(())
    }

    /// `primitive name (ports); declarations; [initial q = v;] table ... endtable endprimitive`
    fn primitive(&mut self, u: &t::UdpDecl) -> Result<Module> {
        let name = self.name(&u.name, u.loc);
        let mut items = Vec::new();
        let mut sequential = false;
        let mut init = None;
        let ports: Vec<String> = u.ports.clone();
        for it in &u.decls {
            let (dir, net, var, ty, decls, loc) = match it {
                t::Item::Port(p) => (Some(p.dir), p.net, p.var, &p.ty, &p.decls, p.loc),
                t::Item::Var(v) => (None, None, v.var, &v.ty, &v.decls, v.loc),
                t::Item::Net(n) => (None, n.net, false, &n.ty, &n.decls, n.loc),
                other => return self.unsupported(item_loc(other), &format!("{} in a primitive", item_what(other))),
            };
            let dir = match dir {
                None => None,
                Some(t::Dir::Input) => Some(Dir::Input),
                Some(t::Dir::Output) => Some(Dir::Output),
                Some(_) => return self.unsupported(loc, "this port direction in a primitive"),
            };
            let mut d = self.decl_head(dir, net, var, ty, None, None, loc)?;
            if d.kind == NetKind::Reg {
                sequential = true;
            }
            let mut names = Vec::new();
            for x in decls {
                let n = self.name(&x.name, x.loc);
                if !x.dims.is_empty() {
                    return self.unsupported(x.loc, "an array in a primitive");
                }
                // `output reg q = 1'b0`: the initial value
                if let Some(e) = &x.init {
                    if d.kind != NetKind::Reg {
                        return self.err(x.loc, "an initial value needs an `output reg`");
                    }
                    let v = self.expr(e)?;
                    if init.is_none() {
                        init = Some(v);
                    }
                }
                names.push(Declarator { name: n, dims: vec![], heap: None, init: None, line: x.loc.line });
            }
            d.names = names;
            items.push(Item::Decl(d));
        }
        if let Some((_, e)) = &u.init {
            init = Some(self.expr(e)?);
        }
        let rows = u.rows.clone();
        let sequential = sequential || rows.iter().any(|r| r.1.is_some());
        Ok(Module {
            name,
            timescale: self.eff_timescale(),
            default_nettype: None,
            unconnected_drive: None,
            file: self.file.clone(),
            line: u.loc.line,
            ports,
            items,
            udp: Some(UdpDef { sequential, rows, init }),
            keyword_names: std::mem::take(&mut self.kw_names),
            interface: false,
            iface_ports: vec![],
            modports: vec![],
            package: false,
            imports: vec![],
            program: false,
            nested: vec![],
            is_nested: false,
        })
    }

    // ---------------------------------------------------------------- packages

    /// `package name; items endpackage`
    fn package(&mut self, p: &t::PackageDecl) -> Result<()> {
        let saved_auto = std::mem::replace(&mut self.default_automatic, p.lifetime == Some(t::Lifetime::Automatic));
        let r = self.package_inner(p);
        self.default_automatic = saved_auto;
        r
    }

    fn package_inner(&mut self, p: &t::PackageDecl) -> Result<()> {
        let name = self.name(&p.name, p.loc);
        let saved_types = self.types.clone();
        let saved_pkgs = std::mem::take(&mut self.mod_pkgs);
        let saved_vars = std::mem::take(&mut self.vars);
        let saved_tu = std::mem::take(&mut self.local_tu);
        let saved_scope = self.type_scope.replace((name.clone(), Default::default()));
        let saved_ctx = self.pkg_ctx.replace(PkgCtx::default());
        let mut own = Vec::new();
        let refs: Vec<&t::Item> = p.items.iter().collect();
        for (k, it) in refs.iter().enumerate() {
            let n = own.len();
            self.module_item(it, &refs[k + 1..], &mut own)?;
            let new_params: Vec<String> = own[n..].iter().filter_map(|i| if let Item::Param(p) = i { Some(p.name.clone()) } else { None }).collect();
            if let Some((_, names)) = &mut self.type_scope {
                names.extend(new_params);
            }
        }
        self.type_scope = saved_scope;
        let ctx = std::mem::replace(&mut self.pkg_ctx, saved_ctx).unwrap_or_default();
        // the package's typedefs: those added while lowering it
        let mut types = HashMap::new();
        let mut names = HashSet::new();
        for (k, v) in &self.types {
            if saved_types.get(k).is_none_or(|o| !Rc::ptr_eq(o, v)) {
                types.insert(k.clone(), v.clone());
                if !ctx.imported_types.contains(k) {
                    names.insert(k.clone());
                }
            }
        }
        for it in &own {
            match it {
                Item::Param(p) => {
                    names.insert(p.name.clone());
                }
                Item::Decl(d) => names.extend(d.names.iter().map(|n| n.name.clone())),
                Item::Subroutine(f) => {
                    names.insert(f.name.clone());
                }
                _ => {}
            }
        }
        let exports = self.exports(&ctx, &names)?;
        // (the packages it imports are found through its scope's imports)
        let used = std::mem::replace(&mut self.mod_pkgs, saved_pkgs);
        let mut items = Vec::new();
        let mut var_items = Vec::new();
        for it in own {
            // variables, functions and tasks stay in the package's own scope
            if matches!(it, Item::Decl(_) | Item::Subroutine(_)) {
                var_items.push(it);
            } else {
                items.push(it);
            }
        }
        let funcs = var_items.iter().filter_map(|it| if let Item::Subroutine(f) = it { Some(f.name.clone()) } else { None }).collect();
        let mut vars = HashMap::new();
        let declared = std::mem::replace(&mut self.vars, saved_vars);
        for it in &var_items {
            if let Item::Decl(d) = it {
                for n in &d.names {
                    if let Some(t) = declared.get(&n.name) {
                        vars.insert(n.name.clone(), t.clone());
                    }
                }
            }
        }
        let mut all = items;
        all.extend(var_items);
        let m = self.package_module(name.clone(), all, used);
        self.pkg_modules.push(m);
        self.local_tu = saved_tu;
        self.types = saved_types;
        self.packages.insert(name, Rc::new(Package { types, vars, funcs, names, exports }));
        Ok(())
    }

    /// The package that declares `name` as seen through package `pkg` (itself, or the
    /// package it exports the name from).
    fn origin(&self, pkg: &str, name: &str) -> Option<String> {
        let p = self.packages.get(pkg)?;
        if p.names.contains(name) {
            return Some(pkg.to_string());
        }
        p.exports.get(name).cloned()
    }

    /// A package's `export` items (26.6): name -> declaring package. An exported name must
    /// be imported into the package: explicitly, or through a wildcard import and then not
    /// declared in the package itself; `p::*` and `*::*` export the names imported
    /// explicitly and those of wildcard imports the package refers to.
    fn exports(&self, ctx: &PkgCtx, own: &HashSet<String>) -> Result<HashMap<String, String>> {
        let mut out = HashMap::new();
        for (pkg, name, loc) in &ctx.exports {
            let all_pkgs = pkg == "*";
            if name == "*" {
                if !all_pkgs && !ctx.explicit.iter().any(|e| &e.0 == pkg) && !ctx.wildcard.contains(pkg) {
                    return self.err(*loc, format!("package '{pkg}' is not imported here"));
                }
                for (via, n, o) in &ctx.explicit {
                    if all_pkgs || via == pkg {
                        out.insert(n.clone(), o.clone());
                    }
                }
                for via in ctx.wildcard.iter().filter(|w| all_pkgs || *w == pkg) {
                    for n in ctx.refs.iter().filter(|n| !own.contains(*n)) {
                        if let Some(o) = self.origin(via, n) {
                            out.entry(n.clone()).or_insert(o);
                        }
                    }
                }
                continue;
            }
            if all_pkgs {
                return self.err(*loc, "`*::name` is not a valid export");
            }
            let o = match ctx.explicit.iter().find(|e| &e.0 == pkg && &e.1 == name) {
                Some(e) => Some(e.2.clone()),
                None if ctx.wildcard.contains(pkg) && !own.contains(name) => self.origin(pkg, name),
                None => None,
            };
            let Some(o) = o else {
                return self.err(*loc, format!("'{pkg}::{name}' is not imported into this package"));
            };
            out.insert(name.clone(), o);
        }
        Ok(out)
    }

    /// Notes a name the package being lowered refers to (wildcard exports).
    pub(crate) fn note_ref(&mut self, n: &str) {
        if let Some(c) = &mut self.pkg_ctx {
            if !c.refs.contains(n) {
                c.refs.insert(n.to_string());
            }
        }
    }

    /// `import p::*, q::name;` (in a module, a package, or at file scope).
    pub(crate) fn import(&mut self, v: &[t::Import], file_scope: bool) -> Result<()> {
        for im in v {
            let via = im.package.clone();
            let Some(vp) = self.packages.get(&via).cloned() else {
                return self.err(im.loc, format!("unknown package '{via}'"));
            };
            // a name the package exports comes from the package that declares it
            let mut pkg = via.clone();
            let mut p = vp.clone();
            if let Some(n) = &im.name {
                let known = vp.names.contains(n) || vp.exports.contains_key(n) || vp.types.contains_key(n) || vp.vars.contains_key(n) || vp.funcs.contains(n);
                if !known {
                    return self.err(im.loc, format!("'{n}' is not declared in or exported by package '{via}'"));
                }
                let o = self.origin(&via, n).unwrap_or(via.clone());
                if o != via {
                    p = self.packages[&o].clone();
                    pkg = o;
                }
                if let Some(c) = &mut self.pkg_ctx {
                    c.explicit.push((via.clone(), n.clone(), pkg.clone()));
                    if p.types.contains_key(n) {
                        c.imported_types.insert(n.clone());
                    }
                }
            } else {
                if let Some(c) = &mut self.pkg_ctx {
                    c.wildcard.push(via.clone());
                    c.imported_types.extend(p.types.keys().cloned());
                }
                // and a wildcard import includes what the package exports
                let ex: Vec<t::Import> = vp.exports.keys().map(|n| t::Import { package: via.clone(), name: Some(n.clone()), loc: im.loc }).collect();
                if !ex.is_empty() {
                    let saved = self.pkg_ctx.take();
                    self.import(&ex, file_scope)?;
                    self.pkg_ctx = saved;
                }
            }
            match &im.name {
                None => {
                    for (k, v) in &p.types {
                        self.types.insert(k.clone(), v.clone());
                    }
                    for (k, v) in &p.vars {
                        self.vars.entry(k.clone()).or_insert_with(|| v.clone());
                        if file_scope {
                            self.unit_var_types.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
                Some(n) => {
                    if let Some(t) = p.types.get(n) {
                        self.types.insert(n.clone(), t.clone());
                    }
                    if let Some(v) = p.vars.get(n) {
                        self.vars.entry(n.clone()).or_insert_with(|| v.clone());
                        if file_scope {
                            self.unit_var_types.entry(n.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
            }
            if file_scope && !self.unit_pkgs.contains(&pkg) {
                self.unit_pkgs.push(pkg.clone());
            }
            if !file_scope && !self.mod_pkgs.contains(&pkg) {
                self.mod_pkgs.push(pkg);
            }
        }
        Ok(())
    }

    /// A reference `pkg::...`: the package's items become part of the current module.
    pub(crate) fn use_package(&mut self, pkg: &str) {
        if !self.mod_pkgs.iter().any(|p| p == pkg) {
            self.mod_pkgs.push(pkg.to_string());
        }
    }
}

pub(crate) fn item_loc(it: &t::Item) -> t::Loc {
    use t::Item as I;
    match it {
        I::Net(d) => d.loc,
        I::Var(d) => d.loc,
        I::Port(d) => d.loc,
        I::Param(p) => p.loc,
        I::Typedef(t::Typedef::Type { loc, .. } | t::Typedef::Forward { loc, .. } | t::Typedef::IfaceType { loc, .. }) => *loc,
        I::Import(_, l) | I::Export(_, l) | I::NetAlias(_, l) | I::Generate(_, l) | I::Defparam(_, l) | I::Specify(_, l) | I::Modport(_, l) => *l,
        I::Genvar(v) => v.first().map(|x| x.1).unwrap_or_default(),
        I::ContAssign(c) => c.loc,
        I::Process(p) => p.loc,
        I::Subroutine(s) => s.loc,
        I::Dpi(d) => d.loc,
        I::Instances(i) => i.loc,
        I::Gates(g) => g.loc,
        I::GenFor(g) => g.loc,
        I::GenIf(g) => g.loc,
        I::GenCase(g) => g.loc,
        I::GenBlock(b) => b.loc,
        I::Clocking(c) => c.loc,
        I::ClockingRef { loc, .. } | I::Timeunits { loc, .. } | I::ElabTask { loc, .. } | I::Nettype { loc, .. } => *loc,
        I::DefaultDisable(_, l) | I::Directive(_, l) | I::Empty(l) => *l,
        I::Assertion(a) => a.loc,
        I::Property(p) => p.loc,
        I::Sequence(s) => s.loc,
        I::Let(l) => l.loc,
        I::Covergroup(c) => c.loc,
        I::Class(c) => c.loc,
        I::Constraint(c) => c.loc,
        I::Bind(b) => b.loc,
        I::Module(m) => m.loc,
    }
}

/// What an item is, for messages.
pub(crate) fn item_what(it: &t::Item) -> &'static str {
    use t::Item as I;
    match it {
        I::Net(_) => "a net declaration",
        I::Var(_) => "a variable declaration",
        I::Port(_) => "a port declaration",
        I::Param(_) => "a parameter",
        I::Typedef(_) => "a typedef",
        I::Import(..) => "an import",
        I::Export(..) => "an export",
        I::Genvar(_) => "a genvar",
        I::ContAssign(_) => "a continuous assignment",
        I::NetAlias(..) => "a net alias",
        I::Process(_) => "a process",
        I::Subroutine(_) => "a function or task",
        I::Dpi(_) => "a DPI import or export",
        I::Instances(_) => "an instance",
        I::Gates(_) => "a gate",
        I::Generate(..) | I::GenFor(_) | I::GenIf(_) | I::GenCase(_) | I::GenBlock(_) => "a generate construct",
        I::Defparam(..) => "a defparam",
        I::Specify(..) => "a specify block",
        I::Modport(..) => "a modport",
        I::Clocking(_) | I::ClockingRef { .. } => "a clocking block",
        I::DefaultDisable(..) => "default disable iff",
        I::Assertion(_) => "a concurrent assertion",
        I::Property(_) => "a property declaration",
        I::Sequence(_) => "a sequence declaration",
        I::Let(_) => "a let declaration",
        I::Covergroup(_) => "a covergroup",
        I::Class(_) => "a class",
        I::Constraint(_) => "a constraint",
        I::Bind(_) => "bind",
        I::Module(_) => "a nested module",
        I::Timeunits { .. } => "timeunit",
        I::ElabTask { .. } => "an elaboration system task",
        I::Nettype { .. } => "a user-defined nettype",
        I::Directive(..) => "a compiler directive",
        I::Empty(_) => "';'",
    }
}

/// Keywords IEEE 1800 (SystemVerilog) reserves beyond those of Verilog-2005: legal names in
/// Verilog-2005 code, but such code does not compile as SystemVerilog (lint check).
pub const SV_RESERVED: &[&str] = &[
    "accept_on", "alias", "always_comb", "always_ff", "always_latch", "assert", "assume", "before", "bind", "bins",
    "binsof", "bit", "break", "byte", "chandle", "checker", "class", "clocking", "const", "constraint", "context",
    "continue", "cover", "covergroup", "coverpoint", "cross", "dist", "do", "endchecker", "endclass", "endclocking",
    "endgroup", "endinterface", "endpackage", "endprogram", "endproperty", "endsequence", "enum", "eventually",
    "expect", "export", "extends", "extern", "final", "first_match", "foreach", "forkjoin", "global", "iff",
    "ignore_bins", "illegal_bins", "implements", "implies", "import", "inside", "int", "interconnect", "interface",
    "intersect", "join_any", "join_none", "let", "local", "logic", "longint", "matches", "modport", "nettype", "new",
    "nexttime", "null", "package", "packed", "priority", "program", "property", "protected", "pure", "rand", "randc",
    "randcase", "randsequence", "ref", "reject_on", "restrict", "return", "s_always", "s_eventually", "s_nexttime",
    "s_until", "s_until_with", "sequence", "shortint", "shortreal", "soft", "solve", "static", "string", "strong",
    "struct", "super", "sync_accept_on", "sync_reject_on", "tagged", "this", "throughout", "timeprecision",
    "timeunit", "type", "typedef", "union", "unique", "unique0", "until", "until_with", "untyped", "var", "virtual",
    "void", "wait_order", "weak", "wildcard", "with", "within",
];

thread_local! {
    /// Index expressions of hierarchical-name segments that are not literals (`blk[i].x`): the
    /// segment is stored as `blk[@N]` and resolved by the compiler in the referencing scope.
    static HIER_INDEX: std::cell::RefCell<Vec<Expr>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The index expression of a `name[@N]` hierarchical-name segment.
pub fn hier_index_expr(n: usize) -> Option<Expr> {
    HIER_INDEX.with(|h| h.borrow().get(n).cloned())
}

/// Store the index expression of a computed `name[@N]` segment; returns N.
pub(crate) fn push_hier_index(e: Expr) -> usize {
    HIER_INDEX.with(|h| {
        let mut h = h.borrow_mut();
        h.push(e);
        h.len() - 1
    })
}

/// Non-ANSI subroutine ports typed by a second declaration (`input x; integer x;`): the
/// port takes the type, and the type-only declaration is dropped.
pub(crate) fn merge_port_types(decls: Vec<Decl>) -> Vec<Decl> {
    let typed: Vec<&Decl> = decls.iter().filter(|d| d.dir.is_none()).collect();
    let find = |n: &str| typed.iter().find(|d| d.names.iter().any(|x| x.name == n)).copied();
    let port_names: Vec<String> =
        decls.iter().filter(|d| d.dir.is_some()).flat_map(|d| d.names.iter().map(|x| x.name.clone())).collect();
    if !port_names.iter().any(|n| find(n).is_some()) {
        return decls;
    }
    let mut out = Vec::new();
    for d in &decls {
        if d.dir.is_some() {
            for x in &d.names {
                let mut nd = d.clone();
                nd.names = vec![x.clone()];
                if let Some(t) = find(&x.name) {
                    nd.kind = t.kind;
                    nd.net = t.net;
                    nd.explicit_kind = true;
                    nd.signed |= t.signed;
                    if t.range.is_some() {
                        nd.range = t.range.clone();
                        nd.packed = t.packed.clone();
                    }
                    if t.type_param.is_some() {
                        nd.type_param = t.type_param.clone();
                    }
                    if t.ty.is_some() {
                        nd.ty = t.ty.clone();
                    }
                }
                out.push(nd);
            }
        } else {
            let mut nd = d.clone();
            nd.names.retain(|x| !port_names.contains(&x.name));
            if !nd.names.is_empty() {
                out.push(nd);
            }
        }
    }
    out
}
