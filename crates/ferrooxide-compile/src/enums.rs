//! Enum types in the compiler: which signals, struct members, class properties and function
//! results hold an enum, the enum type of an expression, and the assignment rule (IEEE
//! 1800-2017 6.19.3): a variable of an enum type takes only a value of that type, else an
//! explicit cast.
//!
//! An enum type is its list of constants as the lowering made it (one shared list per
//! declared enum) together with the scope that declares the constants, so an enum declared in
//! a module is a different type in each instance. Explicit casts are lowered to `$__cast`
//! without their target type: every cast is taken as one to the target's enum (this lets
//! `E2'(x)` into a variable of type E1 pass, never rejects valid code).

use crate::*;
use std::collections::HashMap;

/// The enum types seen so far.
#[derive(Default)]
pub(crate) struct Enums {
    /// (constant list, declaring scope) -> id
    ids: HashMap<(usize, usize), u32>,
    /// constant list -> the scopes that declare its constants (one per module instance)
    owners: HashMap<usize, Vec<usize>>,
    /// (declaring scope, constant) -> the id of its enum
    consts: HashMap<(usize, String), u32>,
    /// signals (of arrays: their elements) of an enum type -> id
    pub(crate) sigs: HashMap<u32, u32>,
    /// queues and dynamic arrays of an enum type -> the id of their elements' enum
    pub(crate) dyn_sigs: HashMap<u32, u32>,
    /// the values of each enum's constants so far, with their names
    values: HashMap<u32, Vec<(Const, String)>>,
}

/// The enum type of an expression.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumOf {
    /// not an enum
    No,
    Id(u32),
    /// an explicit cast (target type not known here)
    Cast,
}

impl<'a> Compiler<'a> {
    /// Enum constant `p` declared in scope `sid`.
    pub(crate) fn enum_const(&mut self, p: &ParamDecl, sid: usize) {
        let Some(names) = &p.enum_type else { return };
        let key = (Rc::as_ptr(names) as usize, sid);
        let next = self.enums.ids.len() as u32;
        let id = *self.enums.ids.entry(key).or_insert(next);
        let owners = self.enums.owners.entry(key.0).or_default();
        if !owners.contains(&sid) {
            owners.push(sid);
        }
        self.enums.consts.insert((sid, p.name.clone()), id);
    }

    /// Enum constant `p` (of scope `sid`) has value `v`: no other constant of its enum may
    /// have the same value (IEEE 1800-2017 6.19).
    pub(crate) fn enum_value(&mut self, p: &ParamDecl, v: &Sym, sid: usize) -> R<()> {
        let (Some(&id), Sym::Const(c)) = (self.enums.consts.get(&(sid, p.name.clone())), v) else { return Ok(()) };
        if p.enum_type.is_none() {
            return Ok(());
        }
        let vals = self.enums.values.entry(id).or_default();
        // (declared again: a subroutine compiled once more)
        if vals.iter().any(|(_, n)| *n == p.name) {
            return Ok(());
        }
        if let Some((_, other)) = vals.iter().find(|(x, _)| x == c) {
            let other = other.clone();
            return self.err(format!("enum constant '{}' has the same value as '{other}'", p.name));
        }
        vals.push((c.clone(), p.name.clone()));
        Ok(())
    }

    /// The id of the enum with constants `names` used in scope `sid`: the one whose
    /// constants are declared in `sid` or a scope around it, else (a package's) the first
    /// (NONE: no constants of it were declared).
    pub(crate) fn enum_id(&mut self, names: &Rc<Vec<String>>, sid: usize) -> u32 {
        let ptr = Rc::as_ptr(names) as usize;
        let Some(owners) = self.enums.owners.get(&ptr) else { return NONE };
        let mut x = Some(sid);
        let mut owner = owners[0];
        while let Some(s) = x {
            if owners.contains(&s) {
                owner = s;
                break;
            }
            x = self.scopes[s].parent.or(self.scopes[s].inst_parent);
        }
        self.enums.ids[&(ptr, owner)]
    }

    /// The enum id of a value of type `p` used in scope `sid` (NONE: not an enum; a packed
    /// array of enums is none either).
    pub(crate) fn packed_enum(&mut self, p: &types::Packed, sid: usize) -> u32 {
        match &p.enum_names {
            Some(n) if p.dims.len() <= 1 && p.fields.is_none() => self.enum_id(&n.clone(), sid),
            _ => NONE,
        }
    }

    /// The scope that declares constant `name` as seen from `sid`, and its name there.
    fn const_scope(&self, name: &str, mut sid: usize) -> Option<(usize, String)> {
        if let Some((p, n)) = name.split_once("::") {
            return Some((self.prefix_scope(p, sid)?, n.to_string()));
        }
        loop {
            if self.scopes[sid].syms.contains_key(name) {
                return Some((sid, name.to_string()));
            }
            if let Some(&p) = self.scopes[sid].name_imports.get(name) {
                if self.scopes[p].syms.contains_key(name) {
                    return Some((p, name.to_string()));
                }
            }
            for &p in &self.scopes[sid].imports {
                if self.scopes[p].syms.contains_key(name) {
                    return Some((p, name.to_string()));
                }
            }
            sid = self.scopes[sid].parent?;
        }
    }

    /// The enum type of `e`.
    pub(crate) fn enum_of(&mut self, e: &Expr, sid: usize) -> R<EnumOf> {
        // an enum method of a type parameter's enum variable that gives a value of it
        if let ExprKind::MethodCall(o, m, _) = &e.kind {
            if matches!(m.as_str(), "first" | "last" | "next" | "prev") {
                if let Some(id) = self.var_enum(o, sid) {
                    return Ok(EnumOf::Id(id));
                }
            }
        }
        Ok(match &e.kind {
            ExprKind::Ident(_) | ExprKind::Hier(_) | ExprKind::Index(..) => {
                // an enum constant (also through a hierarchical name: `top.A`)
                let konst = match &e.kind {
                    ExprKind::Ident(n) if matches!(self.lookup(n, sid), Some(Sym::Const(_))) => self.const_scope(n, sid),
                    ExprKind::Hier(p) if matches!(self.lookup_hier(p, sid), Some(Sym::Const(_))) => {
                        self.hier_scope(&p[..p.len() - 1], sid).map(|s| (s, p[p.len() - 1].clone()))
                    }
                    _ => None,
                };
                if let Some(k) = konst {
                    return Ok(self.enums.consts.get(&k).map_or(EnumOf::No, |&id| EnumOf::Id(id)));
                }
                // a variable, or a whole element of an array
                let mut nidx = 0;
                let mut base = e;
                while let ExprKind::Index(a, _) = &base.kind {
                    nidx += 1;
                    base = a;
                }
                let sym = match &base.kind {
                    ExprKind::Ident(n) => self.lookup(n, sid),
                    ExprKind::Hier(p) => self.lookup_hier(p, sid),
                    _ => None,
                };
                if let Some(Sym::Sig(s)) = sym {
                    // an element of a queue, dynamic or associative array of an enum
                    if nidx == 1 && matches!(self.sig_type(s), crate::types::Type::Dyn { .. }) {
                        return Ok(self.enums.dyn_sigs.get(&s).map_or(EnumOf::No, |&id| EnumOf::Id(id)));
                    }
                    let i = self.sig(s);
                    let nun = if i.depth > 0 { self.sigs.md_dims.get(&s).map_or(1, |d| d.len()) } else { 0 };
                    if nidx == nun {
                        return Ok(self.enums.sigs.get(&s).map_or(EnumOf::No, |&id| EnumOf::Id(id)));
                    }
                    return Ok(EnumOf::No);
                }
                if matches!(&e.kind, ExprKind::Index(..)) {
                    self.prop_enum(e, sid)?
                } else {
                    EnumOf::No
                }
            }
            ExprKind::Member(..) => {
                let p = self.prop_enum(e, sid)?;
                if p != EnumOf::No {
                    return Ok(p);
                }
                match self.target_layout(e, sid) {
                    Some(p) => match self.packed_enum(&p, sid) {
                        NONE => EnumOf::No,
                        id => EnumOf::Id(id),
                    },
                    None => EnumOf::No,
                }
            }
            ExprKind::Cond(_, a, b) => {
                let (x, y) = (self.enum_of(a, sid)?, self.enum_of(b, sid)?);
                match (x, y) {
                    (EnumOf::Cast, y) => y,
                    (x, EnumOf::Cast) => x,
                    (x, y) if x == y => x,
                    _ => EnumOf::No,
                }
            }
            ExprKind::SysCall(n, _) if n == "$__cast" => EnumOf::Cast,
            ExprKind::Call(n, _) => match self.lookup_func(n, sid) {
                Some((f, fs)) => match f.ret_ty.enum_names.clone().filter(|_| f.ret_ty.dims.len() <= 1 && f.ret_ty.unpacked.is_empty()) {
                    Some(names) => match self.enum_id(&names, fs) {
                        NONE => EnumOf::No,
                        id => EnumOf::Id(id),
                    },
                    None => EnumOf::No,
                },
                None => EnumOf::No,
            },
            _ => EnumOf::No,
        })
    }

    /// The enum type of a class property (or an element of an array property) `e`.
    fn prop_enum(&mut self, e: &Expr, sid: usize) -> R<EnumOf> {
        if self.classes.list.is_empty() {
            return Ok(EnumOf::No);
        }
        let id = match &e.kind {
            ExprKind::Member(b, m) => match self.handle_class(b, sid)? {
                Some(c) if self.classes.list[c as usize].props.iter().any(|p| p.name == *m) => self.scalar_prop(c, m)?.enum_id,
                // a constant of the class (`obj.A`)
                Some(c) => {
                    let scope = self.classes.list[c as usize].scope;
                    self.enums.consts.get(&(scope, m.clone())).copied().unwrap_or(NONE)
                }
                None => NONE,
            },
            ExprKind::Index(..) => match self.prop_elem(e, sid)? {
                Some((_, p, _)) => p.enum_id,
                None => NONE,
            },
            _ => NONE,
        };
        Ok(if id == NONE { EnumOf::No } else { EnumOf::Id(id) })
    }

    /// A value `rhs` stored into a target of enum `want` (NONE: no enum, nothing to check).
    pub(crate) fn check_enum(&mut self, want: u32, rhs: &Expr, sid: usize) -> R<()> {
        if want == NONE {
            return Ok(());
        }
        match self.enum_of(rhs, sid)? {
            EnumOf::Cast => Ok(()),
            EnumOf::Id(id) if id == want => Ok(()),
            EnumOf::Id(_) => self.err("an enum value assigned to a target of another enum type (it needs a cast)"),
            EnumOf::No => self.err("a value that is not of the target's enum type needs a cast to it"),
        }
    }

    /// `lhs = rhs`: an enum target takes only a value of its type.
    pub(crate) fn check_enum_assign(&mut self, lhs: &Expr, rhs: &Expr, sid: usize) -> R<()> {
        if self.enums.ids.is_empty() {
            return Ok(());
        }
        // one queue or dynamic array assigned to another: the same element enum (or none)
        if let (Some((l, _)), Some((r, _))) = (self.dyn_var(lhs, sid), self.dyn_var(rhs, sid)) {
            let (a, b) = (self.enums.dyn_sigs.get(&l), self.enums.dyn_sigs.get(&r));
            if a != b {
                return self.err("arrays assigned to each other need equivalent element types (the enum types differ)");
            }
        }
        let want = match self.enum_of(lhs, sid)? {
            EnumOf::Id(id) => id,
            _ => return Ok(()),
        };
        self.check_enum(want, rhs, sid)
    }

    /// Queue or dynamic array `s` declared with data type `t` in scope `sid`: the enum of its
    /// elements, if any.
    pub(crate) fn dyn_enum(&mut self, s: u32, t: Option<&ferrooxide_front::lower::SvType>, sid: usize) {
        let Some(names) = t.and_then(|t| t.enum_names.clone().filter(|_| t.dims.len() <= 1)) else { return };
        let id = self.enum_id(&names, sid);
        if id != NONE {
            self.enums.dyn_sigs.insert(s, id);
        }
    }

    /// Argument `a` of port `ps`: an enum input takes a value of its type (`into`); an output
    /// copied to an enum argument must be of the argument's type.
    pub(crate) fn check_enum_arg(&mut self, ps: u32, a: &Expr, sid: usize, into: bool) -> R<()> {
        if self.enums.ids.is_empty() || matches!(&a.kind, ExprKind::EmptyArg) {
            return Ok(());
        }
        let port = self.enums.sigs.get(&ps).copied().unwrap_or(NONE);
        if into {
            return self.check_enum(port, a, sid);
        }
        match self.enum_of(a, sid)? {
            EnumOf::Id(id) if id != port => self.err("an output argument of an enum type needs a port of that enum type"),
            _ => Ok(()),
        }
    }
}

/// `c` as a sized binary literal.
fn const_lit(c: &Const) -> Expr {
    let digits: String = (0..c.w)
        .rev()
        .map(|i| match c.bit(i) {
            (0, 0) => '0',
            (1, 0) => '1',
            (0, _) => 'z',
            _ => 'x',
        })
        .collect();
    ExprKind::Num(Number { size: Some(c.w), signed: c.signed, base: 2, based: true, digits, fill: false }).into()
}

impl<'a> Compiler<'a> {
    /// The enum of variable `e` (none for anything else).
    pub(crate) fn var_enum(&self, e: &Expr, sid: usize) -> Option<u32> {
        let sym = match &e.kind {
            ExprKind::Ident(n) => self.lookup(n, sid)?,
            ExprKind::Hier(p) => self.lookup_hier(p, sid)?,
            _ => return None,
        };
        match sym {
            Sym::Sig(s) => self.enums.sigs.get(&s).copied(),
            _ => None,
        }
    }

    /// `v.first()`, `.last()`, `.num()`, `.next(N)`, `.prev(N)`, `.name()` of an enum
    /// variable whose enum only elaboration knows (declared with a type parameter): the
    /// expression it stands for, as the lowering writes it for other enum variables (IEEE
    /// 1800-2017 6.19.5). None for any other expression.
    pub(crate) fn enum_method(&self, e: &Expr, sid: usize) -> R<Option<Expr>> {
        let ExprKind::MethodCall(o, m, args) = &e.kind else { return Ok(None) };
        if !matches!(m.as_str(), "first" | "last" | "num" | "next" | "prev" | "name") {
            return Ok(None);
        }
        let Some(id) = self.var_enum(o, sid) else { return Ok(None) };
        let vals = self.enums.values.get(&id).cloned().unwrap_or_default();
        let n = vals.len();
        if n == 0 {
            return Ok(None);
        }
        let w = vals[0].0.w;
        let bx = |e: Expr| Box::new(e);
        let chain = |val: &dyn Fn(usize) -> Expr, dflt: Expr| -> Expr {
            (0..n).rev().fold(dflt, |acc, i| {
                let c = ExprKind::Binary(BinOp::CaseEq, o.clone(), bx(const_lit(&vals[i].0)));
                ExprKind::Cond(bx(c.into()), bx(val(i)), bx(acc)).into()
            })
        };
        let x: Expr = ExprKind::Num(Number { size: Some(w), signed: false, base: 2, based: true, digits: "x".into(), fill: true }).into();
        let x = ExprKind::SysCall("$__cast".into(), vec![ExprKind::Num(dec(w as u64)).into(), ExprKind::Num(dec(2)).into(), x]).into();
        Ok(Some(match m.as_str() {
            "first" => const_lit(&vals[0].0),
            "last" => const_lit(&vals[n - 1].0),
            "num" => ExprKind::Num(dec(n as u64)).into(),
            "name" => chain(&|i| ExprKind::Str(vals[i].1.clone()).into(), ExprKind::Str(String::new()).into()),
            _ => {
                let k = match args.iter().map(|a| &a.kind).collect::<Vec<_>>().as_slice() {
                    [] => 1,
                    [ExprKind::Num(a)] => match Const::from_number(a).ok().and_then(|c| c.to_i64()) {
                        Some(k) if k >= 0 => k as usize,
                        _ => return self.err(format!(".{m}() needs a count of at least 0")),
                    },
                    _ => return self.err(format!("a count of .{m}() that is not a literal is not supported here")),
                };
                let step = if m == "next" { k % n } else { n - k % n };
                chain(&|i| const_lit(&vals[(i + step) % n].0), x)
            }
        }))
    }
}

