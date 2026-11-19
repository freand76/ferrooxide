//! Data types: typedefs, declaration heads, parameter types, enum methods, array queries
//! and casts.
//!
//! A type without a struct becomes the declaration's kind, sign and packed ranges; a type
//! containing a struct is declared as one flat vector `[W-1:0]`. Declarations keep their type
//! (`Decl::ty`): struct members (`Expr::Member`) and assignment patterns (`Expr::Pattern`)
//! are resolved against it in elaboration. Enum constants become `localparam`s of the base
//! type; `T'(e)`, `N'(e)` are `$__cast(width, sign, e)` (sign 0, 1 or 2: keep).

use super::Lower;
use crate::ast::*;
use crate::syntax::tree as t;
use crate::Result;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct SvType {
    pub kind: NetKind,
    pub signed: bool,
    /// packed dimensions, outermost first; integer types have their implicit [n-1:0]
    pub dims: Vec<Range>,
    /// the element is a packed struct / union
    pub fields: Option<Rc<Struct>>,
    /// `int`, `integer`, `byte`, ...: declared without a range
    pub int_kind: bool,
    /// an enum: its constants in order
    pub enum_names: Option<Rc<Vec<String>>>,
    /// unpacked dimensions of a typedef (`typedef logic [7:0] T [4];`): a declaration of the
    /// type is an array with these dimensions after its own
    pub unpacked: Vec<Range>,
    /// a typedef of a queue or dynamic array (`typedef int Q[$];`)
    pub heap: Option<HeapKind>,
    /// a type parameter of this name (resolved per instance; `dims` are outer to it)
    pub param: Option<String>,
    /// a class: its name (the value is a 64-bit handle)
    pub class: Option<String>,
}

/// A package: its items (with those of the packages it imports) and its typedefs.
#[derive(Debug, Default)]
pub struct Package {
    /// its variables (in the package's own module) and their types
    pub vars: std::collections::HashMap<String, (Rc<SvType>, Vec<Range>)>,
    /// its functions and tasks (in the package's own module)
    pub funcs: std::collections::HashSet<String>,
    pub types: std::collections::HashMap<String, Rc<SvType>>,
    /// the names it declares (parameters, enum constants, variables, subroutines, types)
    pub names: std::collections::HashSet<String>,
    /// `export`ed names: name -> the package that declares it
    pub exports: std::collections::HashMap<String, String>,
    /// the classes it declares (for `extends P::C`)
    pub(crate) classes: std::collections::HashMap<String, Rc<super::class::ClassNames>>,
}

#[derive(Debug, Clone)]
pub struct Struct {
    pub union: bool,
    /// `struct packed`: integral members only, the whole is a vector; an unpacked struct is
    /// stored flat as well, but its members may be real or string
    pub packed: bool,
    pub fields: Vec<(String, SvType)>,
}

/// The value of a plain literal (no x/z), possibly negated.
pub(crate) fn lit(e: &Expr) -> Option<i64> {
    match e {
        Expr::Num(n) if !n.fill && !n.digits.contains(['x', 'z', 'X', 'Z', '?']) => {
            i64::from_str_radix(&n.digits.replace('_', ""), n.base).ok()
        }
        Expr::Unary(UnOp::Neg, a) => lit(a).map(|v| -v),
        _ => None,
    }
}

pub(crate) fn num(v: i64) -> Expr {
    if v < 0 {
        Expr::Unary(UnOp::Neg, Box::new(Expr::Num(dec(-v as u64))))
    } else {
        Expr::Num(dec(v as u64))
    }
}

fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
    if let (Some(x), Some(y)) = (lit(&a), lit(&b)) {
        match op {
            BinOp::Add => return num(x + y),
            BinOp::Sub => return num(x - y),
            BinOp::Mul => return num(x * y),
            _ => {}
        }
    }
    match (op, lit(&a), lit(&b)) {
        (BinOp::Add, Some(0), _) => return b,
        (BinOp::Add | BinOp::Sub, _, Some(0)) => return a,
        (BinOp::Mul, Some(1), _) => return b,
        (BinOp::Mul, _, Some(1)) => return a,
        _ => {}
    }
    Expr::Binary(op, Box::new(a), Box::new(b))
}

pub(crate) fn add(a: Expr, b: Expr) -> Expr {
    bin(BinOp::Add, a, b)
}
pub(crate) fn sub(a: Expr, b: Expr) -> Expr {
    bin(BinOp::Sub, a, b)
}
pub(crate) fn mul(a: Expr, b: Expr) -> Expr {
    bin(BinOp::Mul, a, b)
}

/// Number of elements of a dimension [a:b].
pub(crate) fn dim_size(r: &Range) -> Expr {
    if let (Some(a), Some(b)) = (lit(&r.0), lit(&r.1)) {
        return num((a - b).abs() + 1);
    }
    let ge = Expr::Binary(BinOp::Ge, Box::new(r.0.clone()), Box::new(r.1.clone()));
    let d = Expr::Cond(Box::new(ge), Box::new(sub(r.0.clone(), r.1.clone())), Box::new(sub(r.1.clone(), r.0.clone())));
    add(d, num(1))
}

impl SvType {
    pub fn vector(kind: NetKind, signed: bool, dims: Vec<Range>) -> SvType {
        SvType { kind, signed, dims, fields: None, int_kind: false, enum_names: None, unpacked: vec![], heap: None, param: None, class: None }
    }

    /// Width of one element after the packed dimensions.
    pub fn leaf_width(&self) -> Expr {
        if let Some(p) = &self.param {
            return Expr::SysCall("$bits".into(), vec![Expr::Ident(p.clone())]);
        }
        match &self.fields {
            None => num(1),
            Some(s) if s.union => s.fields.first().map_or(num(0), |f| f.1.width()),
            Some(s) => s.fields.iter().fold(num(0), |acc, f| add(acc, f.1.width())),
        }
    }

    /// Width of the dimensions from `from` on, times the element width.
    pub fn width_from(&self, from: usize) -> Expr {
        self.dims[from..].iter().fold(self.leaf_width(), |acc, d| mul(dim_size(d), acc))
    }

    pub fn width(&self) -> Expr {
        self.width_from(0)
    }

    /// `$bits` of the type: the packed width times the unpacked dimensions.
    pub fn bits(&self) -> Expr {
        self.unpacked.iter().fold(self.width(), |acc, d| mul(dim_size(d), acc))
    }

    pub fn has_struct(&self) -> bool {
        self.fields.is_some()
    }

    /// Declaration parts: kind, sign, range, further packed ranges.
    pub fn decl_parts(&self) -> (NetKind, bool, Option<Range>, Vec<Range>) {
        if self.has_struct() {
            let kind = if self.kind == NetKind::Bit { NetKind::Bit } else { NetKind::Reg };
            return (kind, self.signed, Some((sub(self.width(), num(1)), num(0))), vec![]);
        }
        if self.int_kind {
            return (self.kind, self.signed, None, vec![]);
        }
        let mut it = self.dims.iter().cloned();
        (self.kind, self.signed, it.next(), it.collect())
    }
}

/// Builtin type keywords: (kind, signed, width of an integer type).
pub(crate) fn builtin(s: &str) -> Option<(NetKind, bool, Option<u32>)> {
    Some(match s {
        "logic" | "reg" => (NetKind::Reg, false, None),
        "bit" => (NetKind::Bit, false, None),
        "int" => (NetKind::Int(32), true, Some(32)),
        "integer" => (NetKind::Integer, true, Some(32)),
        "byte" => (NetKind::Int(8), true, Some(8)),
        "shortint" => (NetKind::Int(16), true, Some(16)),
        "longint" => (NetKind::Int(64), true, Some(64)),
        "time" => (NetKind::Time, false, Some(64)),
        "real" | "realtime" => (NetKind::Real, true, Some(64)),
        _ => return None,
    })
}


/// `w - 1`, folded for literals.
pub fn minus_one(w: Expr) -> Expr {
    sub(w, num(1))
}

/// `$__cast(width, sign, e)`: sign 0 unsigned, 1 signed, 2 the expression's.
pub fn cast(width: Expr, e: Expr, sign: u64) -> Expr {
    Expr::SysCall("$__cast".into(), vec![width, Expr::Num(dec(sign)), e])
}

/// `n` -> `scope.n` for the names in `names`.
pub(crate) fn qualify_expr(e: &mut Expr, names: &std::collections::HashMap<String, String>) {
    match e {
        Expr::Ident(n) if names.contains_key(n.as_str()) => *e = Expr::Hier(vec![names[n.as_str()].clone(), n.clone()]),
        Expr::Index(a, b) => {
            qualify_expr(a, names);
            qualify_expr(b, names);
        }
        Expr::Range(a, b, c) | Expr::IndexedRange(a, b, c, _) | Expr::Cond(a, b, c) => {
            qualify_expr(a, names);
            qualify_expr(b, names);
            qualify_expr(c, names);
        }
        Expr::Unary(_, a) | Expr::IncDec { e: a, .. } => qualify_expr(a, names),
        Expr::Binary(_, a, b) => {
            qualify_expr(a, names);
            qualify_expr(b, names);
        }
        Expr::Concat(v) | Expr::Call(_, v) | Expr::SysCall(_, v) => v.iter_mut().for_each(|x| qualify_expr(x, names)),
        Expr::Repl(a, v) => {
            qualify_expr(a, names);
            v.iter_mut().for_each(|x| qualify_expr(x, names));
        }
        _ => {}
    }
}

pub(crate) struct PType {
    pub signed: bool,
    pub integer: bool,
    pub real: bool,
    pub range: Option<Range>,
    pub sv: Option<SvType>,
}

/// One step after a name: `.m`, `[i]`, `[a:b]`, `(args)`.
#[derive(Clone)]
pub(crate) enum Op<'a> {
    Member(&'a str, t::Loc),
    Index(&'a t::Expr),
    Range(t::RangeKind, &'a t::Expr, &'a t::Expr),
    Call(&'a [t::Arg], t::Loc),
}

pub(crate) fn int_name(k: t::IntKw) -> &'static str {
    match k {
        t::IntKw::Bit => "bit",
        t::IntKw::Logic => "logic",
        t::IntKw::Reg => "reg",
        t::IntKw::Byte => "byte",
        t::IntKw::Shortint => "shortint",
        t::IntKw::Int => "int",
        t::IntKw::Longint => "longint",
        t::IntKw::Integer => "integer",
        t::IntKw::Time => "time",
    }
}

impl Lower {
    /// A dimension as a range (`[n]` is `[0:n-1]`).
    pub(crate) fn dim(&mut self, d: &t::Dim, loc: t::Loc) -> Result<Range> {
        match d {
            t::Dim::Range(a, b) => Ok((self.expr(a)?, self.expr(b)?)),
            t::Dim::Size(n) => {
                let n = self.expr(n)?;
                Ok((Expr::Num(dec(0)), Expr::Binary(BinOp::Sub, Box::new(n), Box::new(Expr::Num(dec(1))))))
            }
            t::Dim::Unsized => self.unsupported(loc, "a dynamic array"),
            t::Dim::Queue(_) => self.unsupported(loc, "a queue"),
            t::Dim::Assoc(_) => self.unsupported(loc, "an associative array"),
        }
    }

    /// The unpacked dimensions of a declarator: ranges, or the one dimension of a queue or
    /// dynamic array.
    pub(crate) fn unpacked_dims(&mut self, v: &[t::Dim], loc: t::Loc) -> Result<(Vec<Range>, Option<HeapKind>)> {
        let heap = |d: &t::Dim| matches!(d, t::Dim::Unsized | t::Dim::Queue(_) | t::Dim::Assoc(_));
        match v {
            [t::Dim::Unsized] => Ok((vec![], Some(HeapKind::Dynamic))),
            [t::Dim::Queue(max)] => {
                let max = match max {
                    Some(e) => Some(self.expr(e)?),
                    None => None,
                };
                Ok((vec![], Some(HeapKind::Queue(max))))
            }
            [t::Dim::Assoc(None)] => Ok((vec![], Some(HeapKind::Assoc(None)))),
            [t::Dim::Assoc(Some(k))] => {
                let kt = if matches!(k.kind, t::TypeKind::String) && k.packed.is_empty() {
                    SvType { int_kind: true, ..SvType::vector(NetKind::String, false, vec![(num(63), num(0))]) }
                } else {
                    let kt = self.data_type_req(k)?;
                    if kt.has_struct() || kt.kind == NetKind::Real || kt.heap.is_some() {
                        return self.unsupported(loc, "this associative array key type");
                    }
                    kt
                };
                Ok((vec![], Some(HeapKind::Assoc(Some(Rc::new(kt))))))
            }
            v if v.iter().any(heap) => self.unsupported(loc, "an array of queues or dynamic arrays"),
            v => Ok((self.dims(v, loc)?, None)),
        }
    }

    pub(crate) fn dims(&mut self, v: &[t::Dim], loc: t::Loc) -> Result<Vec<Range>> {
        let mut out = Vec::new();
        for d in v {
            out.push(self.dim(d, loc)?);
        }
        Ok(out)
    }

    /// The type is resolved through the type tables (struct, enum, a type name).
    fn is_sv_type(&self, ty: &t::DataType) -> bool {
        match &ty.kind {
            t::TypeKind::Struct(_) | t::TypeKind::Enum(_) => true,
            t::TypeKind::Named(n) => n.scopes.is_empty() || n.scopes.len() == 1,
            _ => false,
        }
    }

    /// A data type, or None if it is none the old type tables know (implicit, string, ...).
    pub(crate) fn data_type(&mut self, ty: &t::DataType) -> Result<Option<SvType>> {
        match &ty.kind {
            t::TypeKind::Struct(st) => {
                if st.tagged {
                    return self.unsupported(ty.loc, "a tagged union");
                }
                let signed = ty.signed.unwrap_or(false);
                let mut fields = Vec::new();
                for m in &st.members {
                    if m.rand.is_some() {
                        return self.unsupported(m.loc, "a rand struct member");
                    }
                    let ft = match self.data_type(&m.ty)? {
                        Some(ft) => ft,
                        // a string member: a 64-bit handle
                        None if matches!(m.ty.kind, t::TypeKind::String) && m.ty.packed.is_empty() => {
                            SvType { int_kind: true, ..SvType::vector(NetKind::String, false, vec![(num(63), num(0))]) }
                        }
                        None => return self.err(m.loc, "expected a member type"),
                    };
                    // IEEE 1800 7.2.1: the members of a packed struct are integral
                    if st.packed && matches!(ft.kind, NetKind::Real | NetKind::String) {
                        return self.err(m.loc, "a member of a packed struct or union must be of an integral type");
                    }
                    if st.packed && ft.fields.as_ref().is_some_and(|f| !f.packed) {
                        return self.err(m.loc, "a member of a packed struct or union cannot be an unpacked struct");
                    }
                    for d in &m.decls {
                        if !d.dims.is_empty() {
                            return self.unsupported(d.loc, "an unpacked struct member array");
                        }
                        if d.init.is_some() {
                            return self.unsupported(d.loc, "a struct member default");
                        }
                        let n = self.name(&d.name, d.loc);
                        fields.push((n, ft.clone()));
                    }
                }
                let dims = self.dims(&ty.packed, ty.loc)?;
                let kind = if fields.iter().all(|f| f.1.kind == NetKind::Bit) { NetKind::Bit } else { NetKind::Reg };
                Ok(Some(SvType { kind, signed, dims, fields: Some(Rc::new(Struct { union: st.union, packed: st.packed, fields })), int_kind: false, enum_names: None, unpacked: vec![], heap: None, param: None, class: None }))
            }
            t::TypeKind::Enum(et) => {
                let int32 = |signed: bool| SvType { kind: NetKind::Int(32), signed, dims: vec![(num(31), num(0))], fields: None, int_kind: true, enum_names: None, unpacked: vec![], heap: None, param: None, class: None };
                let base = match &et.base {
                    None => int32(true),
                    Some(b) => match self.data_type(b)? {
                        Some(t) => t,
                        None if matches!(b.kind, t::TypeKind::Implicit) && b.packed.is_empty() => int32(b.signed.unwrap_or(true)),
                        None => return self.unsupported(b.loc, "this enum base type"),
                    },
                };
                let mut prev: Option<String> = None;
                let mut names = Vec::new();
                let (_, signed, range, _) = base.decl_parts();
                let range = range.or_else(|| base.int_kind.then(|| base.dims[0].clone()));
                for it in &et.items {
                    // `name[N]`: name0 .. name(N-1); `name[N:M]`: nameN .. nameM (6.19)
                    let base_name = self.name(&it.name, it.loc);
                    let item_names: Vec<String> = match &it.range {
                        None => vec![base_name],
                        Some((a, b)) => {
                            let lit = |l: &mut Self, e: &t::Expr| -> Result<i64> {
                                let x = l.expr(e)?;
                                match super::expr::lit_index(&x) {
                                    Some(v) if v >= 0 => Ok(v),
                                    _ => l.err(e.loc, "an enum name range needs a non-negative integer literal"),
                                }
                            };
                            let (lo, hi) = match b {
                                None => {
                                    let n = lit(self, a)?;
                                    if n == 0 {
                                        return self.err(it.loc, "an enum name range of size 0");
                                    }
                                    (0, n - 1)
                                }
                                Some(b) => (lit(self, a)?, lit(self, b)?),
                            };
                            let ks: Vec<i64> = if lo <= hi { (lo..=hi).collect() } else { (hi..=lo).rev().collect() };
                            ks.iter().map(|k| format!("{base_name}{k}")).collect()
                        }
                    };
                    for (k, name) in item_names.into_iter().enumerate() {
                        let value = match (&it.value, k) {
                            (Some(e), 0) => self.expr(e)?,
                            _ => match &prev {
                                Some(p) => add(Expr::Ident(p.clone()), num(1)),
                                None => num(0),
                            },
                        };
                        self.pending.push(Item::Param(ParamDecl { local: true, signed, range: range.clone(), integer: false, real: false, ty: None, name: name.clone(), value: Some(value), line: it.loc.line }));
                        names.push(name.clone());
                        prev = Some(name);
                    }
                }
                let mut t = base;
                t.enum_names = Some(Rc::new(names));
                // dimensions after the enum are outer to its base type's
                let mut outer = self.dims(&ty.packed, ty.loc)?;
                if !outer.is_empty() {
                    t.int_kind = false;
                    outer.extend(t.dims);
                    t.dims = outer;
                }
                Ok(Some(t))
            }
            t::TypeKind::Int(k) => {
                let (kind, signed, iw) = builtin(int_name(*k)).unwrap();
                let signed = ty.signed.unwrap_or(signed);
                let mut dims = Vec::new();
                if let Some(w) = iw {
                    dims.push((num(w as i64 - 1), num(0)));
                }
                dims.extend(self.dims(&ty.packed, ty.loc)?);
                Ok(Some(SvType { kind, signed, dims, fields: None, int_kind: iw.is_some(), enum_names: None, unpacked: vec![], heap: None, param: None, class: None }))
            }
            t::TypeKind::Real(_) => {
                let (kind, signed, iw) = builtin("real").unwrap();
                let mut dims = vec![(num(iw.unwrap() as i64 - 1), num(0))];
                dims.extend(self.dims(&ty.packed, ty.loc)?);
                Ok(Some(SvType { kind, signed, dims, fields: None, int_kind: true, enum_names: None, unpacked: vec![], heap: None, param: None, class: None }))
            }
            t::TypeKind::Named(n) => {
                if n.params.is_some() || n.scopes.iter().any(|s| s.params.is_some()) || n.scopes.len() > 1 {
                    return self.unsupported(ty.loc, "a parameterized class type");
                }
                if let Some(s) = n.scopes.first() {
                    // `C::T`: a typedef of class C
                    if let Some(t) = self.classes.get(&s.name).and_then(|c| c.types.get(&n.name)) {
                        let mut t = (**t).clone();
                        t.signed = ty.signed.unwrap_or(t.signed);
                        if !ty.packed.is_empty() {
                            return self.unsupported(ty.loc, "packed dimensions after a class type");
                        }
                        return Ok(Some(t));
                    }
                    // `pkg::T`: the package's type becomes visible here
                    let Some(p) = self.packages.get(&s.name).cloned() else { return Ok(None) };
                    let Some(t) = p.types.get(&n.name) else { return Ok(None) };
                    self.types.insert(n.name.clone(), t.clone());
                    self.use_package(&s.name);
                }
                self.note_ref(&n.name);
                let Some(t) = self.types.get(&n.name).cloned() else { return Ok(None) };
                let mut t = (*t).clone();
                t.signed = ty.signed.unwrap_or(t.signed);
                // dimensions after a type name are outer to the type's own
                let mut outer = self.dims(&ty.packed, ty.loc)?;
                if !outer.is_empty() {
                    t.int_kind = false;
                    outer.extend(t.dims);
                    t.dims = outer;
                }
                Ok(Some(t))
            }
            _ => Ok(None),
        }
    }

    /// A data type that must resolve.
    pub(crate) fn data_type_req(&mut self, ty: &t::DataType) -> Result<SvType> {
        match self.data_type(ty)? {
            Some(t) => Ok(t),
            None => match &ty.kind {
                t::TypeKind::Named(n) => self.err(ty.loc, format!("unknown type '{}'", n.name)),
                _ => self.unsupported(ty.loc, "this data type"),
            },
        }
    }

    /// `typedef ...;`; `rest`: the items after it in the scope (for forward declarations).
    pub(crate) fn typedef(&mut self, td: &t::Typedef, rest: &[&t::Item]) -> Result<()> {
        match td {
            t::Typedef::Forward { kind, name, loc, .. } => {
                use crate::syntax::kw::Kw;
                if !self.types.contains_key(name) {
                    // the definition comes later in this scope: lower it now, so that uses
                    // before it know the type
                    if let Some(def) = find_typedef(rest, name) {
                        self.typedef(def, &[])?;
                        self.early.insert(def as *const t::Typedef as usize);
                    } else if find_class(rest, name) {
                        // a class declared later: its name is a class type from here on
                        self.types.insert(name.clone(), Self::class_type(name));
                    }
                }
                if let Some(t) = self.types.get(name) {
                    let (ok, what) = match kind {
                        Some(Kw::Enum) => (t.enum_names.is_some(), "enum"),
                        Some(Kw::Struct) => (t.fields.as_ref().is_some_and(|f| !f.union), "struct"),
                        Some(Kw::Union) => (t.fields.as_ref().is_some_and(|f| f.union), "union"),
                        Some(Kw::Class) => (t.class.is_some(), "class"),
                        Some(_) => (false, "class"),
                        None => (true, ""),
                    };
                    if !ok {
                        return self.err(*loc, format!("'{name}' is declared as {what} (line {}) but defined differently", loc.line));
                    }
                }
                Ok(())
            }
            t::Typedef::IfaceType { loc, .. } => self.unsupported(*loc, "a typedef of an interface port's type"),
            t::Typedef::Type { ty, name, dims, loc } => {
                if self.early.contains(&(td as *const t::Typedef as usize)) {
                    return Ok(());
                }
                let mut t = if matches!(ty.kind, t::TypeKind::String) && ty.packed.is_empty() {
                    SvType { kind: NetKind::String, signed: false, dims: vec![], fields: None, int_kind: false, enum_names: None, unpacked: vec![], heap: None, param: None, class: None }
                } else {
                    match self.data_type(ty)? {
                        Some(t) => t,
                        None => return self.err(*loc, "expected a type after typedef"),
                    }
                };
                let name = self.name(name, *loc);
                self.qualify_type(&mut t);
                let (_, heap) = self.unpacked_dims(dims, *loc)?;
                if heap.is_some() {
                    if !t.unpacked.is_empty() || t.heap.is_some() {
                        return self.unsupported(*loc, "an array of queues or dynamic arrays");
                    }
                    t.heap = heap;
                    self.types.insert(name, Rc::new(t));
                    return Ok(());
                }
                for d in dims {
                    let mut r = self.dim(d, *loc)?;
                    if let Some(names) = &self.type_scope {
                        qualify_expr(&mut r.0, names);
                        qualify_expr(&mut r.1, names);
                    }
                    t.unpacked.push(r);
                }
                self.types.insert(name, Rc::new(t));
                Ok(())
            }
        }
    }

    /// Identifiers in the dimensions of a type declared in a package or at file scope that
    /// name parameters of that scope, or names it imported, refer to them where they are
    /// declared.
    fn qualify_type(&self, t: &mut SvType) {
        let Some(names) = &self.type_scope else { return };
        if names.is_empty() {
            return;
        }
        fn ty(t: &mut SvType, names: &std::collections::HashMap<String, String>) {
            for d in t.dims.iter_mut().chain(t.unpacked.iter_mut()) {
                qualify_expr(&mut d.0, names);
                qualify_expr(&mut d.1, names);
            }
            if let Some(f) = &mut t.fields {
                let f = Rc::make_mut(f);
                for (_, ft) in &mut f.fields {
                    ty(ft, names);
                }
            }
        }
        ty(t, names);
    }

    /// `name [= data_type]` of a type parameter; the name is a type from here on.
    pub(crate) fn type_param(&mut self, local: bool, name: &str, default: Option<&t::DataType>, loc: t::Loc) -> Result<Item> {
        let name = self.name(name, loc);
        let default = match default {
            Some(d) => Some(Rc::new(self.data_type_req(d)?)),
            None => None,
        };
        let mut t = SvType::vector(NetKind::Reg, false, vec![]);
        t.param = Some(name.clone());
        self.types.insert(name.clone(), Rc::new(t));
        Ok(Item::TypeParam { local, name, default, line: loc.line })
    }

    /// The type of a value parameter.
    pub(crate) fn param_type(&mut self, ty: &t::DataType) -> Result<PType> {
        let svlike = self.is_sv_type(ty) || matches!(ty.kind, t::TypeKind::Int(t::IntKw::Byte | t::IntKw::Shortint | t::IntKw::Longint | t::IntKw::Time));
        if svlike {
            let t = self.data_type_req(ty)?;
            let (kind, signed, range, packed) = t.decl_parts();
            // a parameter has one range: the whole width
            let range = if packed.is_empty() { range } else { Some((minus_one(t.width()), Expr::Num(dec(0)))) };
            if t.int_kind && matches!(kind, NetKind::Int(_) | NetKind::Time) && !matches!(kind, NetKind::Int(32)) {
                // byte, shortint, longint, time: their width
                let range = Some((minus_one(t.width()), Expr::Num(dec(0))));
                return Ok(PType { signed, integer: false, real: false, range, sv: Some(t) });
            }
            let integer = matches!(kind, NetKind::Int(_) | NetKind::Integer) && range.is_none();
            return Ok(PType { signed, integer, real: false, range, sv: Some(t) });
        }
        let mut signed = ty.signed == Some(true);
        let mut integer = false;
        let mut real = false;
        match &ty.kind {
            t::TypeKind::Implicit | t::TypeKind::Int(t::IntKw::Logic | t::IntKw::Bit | t::IntKw::Reg) => {}
            t::TypeKind::Int(t::IntKw::Integer | t::IntKw::Int) => {
                integer = true;
                signed = ty.signed != Some(false);
            }
            t::TypeKind::Real(_) => real = true,
            t::TypeKind::String if ty.packed.is_empty() => {
                // a string parameter (its value is the text, as a vector; typed `string`)
                let st = SvType { kind: NetKind::String, signed: false, dims: vec![], fields: None, int_kind: false, enum_names: None, unpacked: vec![], heap: None, param: None, class: None };
                return Ok(PType { signed: false, integer: false, real: false, range: None, sv: Some(st) });
            }
            _ => return self.unsupported(ty.loc, "a parameter of this type"),
        }
        if ty.packed.len() > 1 {
            // several packed dimensions: one range for the whole width, the type keeps them
            if integer || real {
                return self.err(ty.loc, "packed dimensions after a type that has a width");
            }
            let kind = if matches!(ty.kind, t::TypeKind::Int(t::IntKw::Bit)) { NetKind::Bit } else { NetKind::Reg };
            let t = SvType::vector(kind, signed, self.dims(&ty.packed, ty.loc)?);
            let range = Some((minus_one(t.width()), Expr::Num(dec(0))));
            return Ok(PType { signed, integer: false, real: false, range, sv: Some(t) });
        }
        let range = match ty.packed.as_slice() {
            [] => None,
            [d] => Some(self.dim(d, ty.loc)?),
            _ => unreachable!(),
        };
        Ok(PType { signed, integer, real, range, sv: None })
    }

    /// One `name = value` of a value parameter; `port`: in a parameter port list.
    pub(crate) fn param_assign(&mut self, local: bool, port: bool, pt: &PType, d: &t::Declarator) -> Result<ParamDecl> {
        let name = self.name(&d.name, d.loc);
        if !d.dims.is_empty() {
            return self.unsupported(d.loc, "an unpacked array parameter");
        }
        // a parameter port may leave out its default (IEEE 1800 6.20.1); its instances must
        // override it
        let value = match &d.init {
            Some(v) => Some(self.expr(v)?),
            None if port && !local => None,
            None => return self.err(d.loc, format!("parameter '{name}' needs a value (only a parameter port may leave it out)")),
        };
        self.decl_type = pt.sv.clone().map(Rc::new);
        self.declare_var(&name, &[]);
        let ty = self.decl_type.clone();
        Ok(ParamDecl { local, signed: pt.signed, range: pt.range.clone(), integer: pt.integer, real: pt.real, ty, name, value, line: d.loc.line })
    }

    /// A declaration head: direction, net type, `var`, data type, strength and delay.
    /// Records the type for the declarators that follow (`decl_type`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decl_head(
        &mut self,
        dir: Option<Dir>,
        net: Option<t::NetTypeKw>,
        var: bool,
        ty: &t::DataType,
        strength: Option<&t::Strength>,
        delay: Option<&t::Delay>,
        loc: t::Loc,
    ) -> Result<Decl> {
        let line = loc.line;
        self.decl_type = None;
        let strength = strength.and_then(drive_strength);
        let delay = match delay {
            Some(d) => Some(self.delay(d)?),
            None => None,
        };
        if self.is_sv_type(ty) {
            let named = matches!(ty.kind, t::TypeKind::Named(_));
            let t = self.data_type_req(ty)?;
            let (kind, signed, range, packed) = t.decl_parts();
            let type_param = t.param.clone();
            let ty = Rc::new(t);
            self.decl_type = Some(ty.clone());
            let netk = net.map(net_type).unwrap_or_default();
            let range_written = range.is_some() && !named;
            return Ok(Decl { dir, kind, net: netk, explicit_kind: true, real_net: false, strength, delay, signed, range, range_written, packed, type_param, ty: Some(ty), names: vec![], automatic: false, static_: false, line });
        }
        let mut kind = NetKind::Wire;
        let mut explicit_kind = false;
        let mut real_net = false;
        let mut signed = false;
        let netk = match net {
            Some(t::NetTypeKw::Interconnect) => return self.unsupported(loc, "an interconnect net"),
            Some(n) => {
                explicit_kind = true;
                match &ty.kind {
                    t::TypeKind::Implicit | t::TypeKind::Int(t::IntKw::Logic | t::IntKw::Reg) => {}
                    t::TypeKind::Real(_) => {
                        kind = NetKind::Real;
                        real_net = true;
                    }
                    // 4-state integer types (6.7.1): a vector net of their width
                    t::TypeKind::Int(k @ (t::IntKw::Integer | t::IntKw::Time)) if ty.packed.is_empty() => {
                        let integer = *k == t::IntKw::Integer;
                        let w = if integer { 31 } else { 63 };
                        let mut d = self.decl_head(dir, net, var, &t::DataType { kind: t::TypeKind::Implicit, signed: Some(ty.signed.unwrap_or(integer)), packed: vec![], loc: ty.loc }, None, None, loc)?;
                        d.range = Some((Expr::Num(dec(w)), Expr::Num(dec(0))));
                        d.strength = strength;
                        d.delay = delay;
                        let mut t = SvType::vector(NetKind::Wire, d.signed, vec![d.range.clone().unwrap()]);
                        t.int_kind = true;
                        let t = Rc::new(t);
                        self.decl_type = Some(t.clone());
                        d.ty = Some(t);
                        return Ok(d);
                    }
                    _ => return self.unsupported(ty.loc, "a net of this data type (only 4-state types)"),
                }
                net_type(n)
            }
            None => {
                let k = match &ty.kind {
                    t::TypeKind::Implicit => None,
                    t::TypeKind::Int(t::IntKw::Logic) if !var && matches!(dir, Some(Dir::Input | Dir::Inout)) => Some(NetKind::Wire),
                    t::TypeKind::Int(t::IntKw::Logic | t::IntKw::Reg) => Some(NetKind::Reg),
                    t::TypeKind::Int(t::IntKw::Bit) => Some(NetKind::Bit),
                    t::TypeKind::Int(t::IntKw::Integer) => Some(NetKind::Integer),
                    t::TypeKind::Int(t::IntKw::Time) => Some(NetKind::Time),
                    t::TypeKind::Int(t::IntKw::Int) => Some(NetKind::Int(32)),
                    t::TypeKind::Int(t::IntKw::Byte) => Some(NetKind::Int(8)),
                    t::TypeKind::Int(t::IntKw::Shortint) => Some(NetKind::Int(16)),
                    t::TypeKind::Int(t::IntKw::Longint) => Some(NetKind::Int(64)),
                    t::TypeKind::Real(_) => Some(NetKind::Real),
                    t::TypeKind::Event => Some(NetKind::Event),
                    t::TypeKind::String => Some(NetKind::String),
                    _ => return self.unsupported(ty.loc, "this data type"),
                };
                if let Some(k) = k {
                    kind = k;
                    explicit_kind = true;
                    if matches!(k, NetKind::Integer | NetKind::Int(_)) {
                        signed = true;
                    }
                }
                NetType::Wire
            }
        };
        if let Some(s) = ty.signed {
            signed = s;
        }
        let mut pd = self.dims(&ty.packed, ty.loc)?.into_iter();
        let range = pd.next();
        let packed: Vec<Range> = pd.collect();
        // the type (assignment patterns, $left & co.)
        if !matches!(kind, NetKind::Real | NetKind::Event | NetKind::Genvar) {
            let iw = match kind {
                NetKind::Integer => Some(32),
                NetKind::Int(n) => Some(n),
                NetKind::Time => Some(64),
                _ => None,
            };
            let mut t = SvType::vector(kind, signed, vec![]);
            if let Some(w) = iw.filter(|_| range.is_none()) {
                t.dims.push((Expr::Num(dec(w as u64 - 1)), Expr::Num(dec(0))));
                t.int_kind = true;
            }
            t.dims.extend(range.iter().cloned());
            t.dims.extend(packed.iter().cloned());
            self.decl_type = Some(Rc::new(t));
        }
        // `var` without a type: a logic variable (not a net, also as a port)
        if var && kind == NetKind::Wire {
            kind = NetKind::Reg;
            explicit_kind = true;
        }
        let ty = self.decl_type.clone();
        let range_written = range.is_some();
        Ok(Decl { dir, kind, net: netk, explicit_kind, real_net, strength, delay, signed, range, range_written, packed, type_param: None, ty, names: Vec::new(), automatic: false, static_: false, line })
    }

    /// The declarators of a declaration (after `decl_head`).
    pub(crate) fn declarators(&mut self, v: &[t::Declarator]) -> Result<Vec<Declarator>> {
        let mut out = Vec::new();
        for d in v {
            let name = self.name(&d.name, d.loc);
            let (dims, heap) = self.declarator_dims(&d.dims, d.loc)?;
            let init = match &d.init {
                Some(e) => Some(self.expr(e)?),
                None => None,
            };
            self.declare_var_heap(&name, &dims, heap.as_ref());
            out.push(Declarator { name, dims, heap, init, line: d.loc.line });
        }
        Ok(out)
    }

    /// The unpacked dimensions of a declarator with those of an array typedef before them, or
    /// the declarator's (or the typedef's) queue or dynamic dimension.
    pub(crate) fn declarator_dims(&mut self, v: &[t::Dim], loc: t::Loc) -> Result<(Vec<Range>, Option<HeapKind>)> {
        let (mut dims, mut heap) = self.unpacked_dims(v, loc)?;
        if let Some(t) = &self.decl_type {
            if let Some(h) = &t.heap {
                if !dims.is_empty() || heap.is_some() {
                    return self.unsupported(loc, "an array of queues or dynamic arrays");
                }
                heap = Some(h.clone());
            }
            if heap.is_some() && !t.unpacked.is_empty() {
                return self.unsupported(loc, "a queue or dynamic array of unpacked arrays");
            }
            dims.splice(0..0, t.unpacked.iter().cloned());
        }
        Ok((dims, heap))
    }

    /// `declare_var` of a queue or dynamic array (`heap`).
    pub(crate) fn declare_var_heap(&mut self, name: &str, unpacked: &[Range], heap: Option<&HeapKind>) {
        self.declare_var(name, unpacked);
        if let (Some(h), Some(v)) = (heap, self.vars.get_mut(name)) {
            let mut t = (*v.0).clone();
            t.heap = Some(h.clone());
            v.0 = Rc::new(t);
        }
    }

    /// Note the type of a declared name (struct members are resolved through it).
    pub(crate) fn declare_var(&mut self, name: &str, unpacked: &[Range]) {
        match &self.decl_type {
            Some(t) => {
                self.vars.insert(name.to_string(), (t.clone(), unpacked.to_vec()));
            }
            None => {
                self.vars.remove(name);
            }
        }
    }

    /// `v.first()`, `.last()`, `.num()`, `.next(N)`, `.prev(N)`, `.name()` of an enum variable.
    pub(crate) fn enum_method(&mut self, name: String, m: &str, arg: Option<&t::Expr>, loc: t::Loc) -> Result<Expr> {
        let names = self.vars[&name].0.enum_names.clone().unwrap();
        let arg = match arg {
            Some(a) => Some(self.expr(a)?),
            None => None,
        };
        let n = names.len();
        let v = Expr::Ident(name.clone());
        let chain = |val: &dyn Fn(usize) -> Expr, dflt: Expr| {
            (0..n).rev().fold(dflt, |acc, i| {
                let c = Expr::Binary(BinOp::CaseEq, Box::new(v.clone()), Box::new(Expr::Ident(names[i].clone())));
                Expr::Cond(Box::new(c), Box::new(val(i)), Box::new(acc))
            })
        };
        let x = Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: "x".into(), fill: true });
        Ok(match m {
            "first" => Expr::Ident(names[0].clone()),
            "last" => Expr::Ident(names[n - 1].clone()),
            "num" => num(n as i64),
            "next" | "prev" => {
                let k = match &arg {
                    None => 1,
                    Some(e) => match lit(e) {
                        Some(k) if k >= 0 => k as usize,
                        Some(_) => return self.err(loc, format!(".{m}() needs a count of at least 0")),
                        // a count known only in elaboration or at run time: the constant `n`
                        // steps from each value
                        None => {
                            if super::stmt::side_effects(e) || n > 64 {
                                return self.unsupported(loc, &format!("this count of .{m}()"));
                            }
                            let bx = |e: Expr| Box::new(e);
                            let r = Expr::Binary(BinOp::Mod, bx(e.clone()), bx(num(n as i64)));
                            let t = self.vars[&name].0.clone();
                            let val = |i: usize| {
                                let pos = if m == "next" {
                                    Expr::Binary(BinOp::Add, bx(num(i as i64)), bx(r.clone()))
                                } else {
                                    Expr::Binary(BinOp::Sub, bx(num((i + n) as i64)), bx(r.clone()))
                                };
                                let pos = Expr::Binary(BinOp::Mod, bx(pos), bx(num(n as i64)));
                                (0..n).rev().fold(x.clone(), |acc, j| {
                                    let c = Expr::Binary(BinOp::Eq, bx(pos.clone()), bx(num(j as i64)));
                                    Expr::Cond(bx(c), bx(Expr::Ident(names[j].clone())), bx(acc))
                                })
                            };
                            return Ok(cast(t.width(), chain(&val, x.clone()), u64::from(t.signed)));
                        }
                    },
                };
                let step = if m == "next" { k % n } else { n - k % n };
                let t = self.vars[&name].0.clone();
                cast(t.width(), chain(&|i| Expr::Ident(names[(i + step) % n].clone()), x), u64::from(t.signed))
            }
            _ => Expr::SysCall("$__str".into(), vec![chain(&|i| Expr::Str(names[i].rsplit("::").next().unwrap().to_string()), Expr::Str(String::new()))]),
        })
    }

    /// The (left, right) bounds of every dimension of a declared name: unpacked first.
    pub(crate) fn var_dims(&self, name: &str) -> Option<Vec<(Expr, Expr)>> {
        let (t, unpacked) = self.vars.get(name)?;
        let mut dims = unpacked.clone();
        dims.extend(t.dims.iter().cloned());
        if t.has_struct() {
            dims.push((sub(t.leaf_width(), num(1)), num(0)));
        }
        Some(dims)
    }

    /// `$left(v)`, `$size(v, 2)`, ... of a declared name.
    pub(crate) fn array_query(&mut self, f: &str, name: &str, dim: Option<&t::Expr>, loc: t::Loc) -> Result<Expr> {
        let name = self.name(name, loc);
        let k = match dim {
            Some(e) => {
                let e = self.expr(e)?;
                match lit(&e) {
                    Some(k) => k,
                    None => return self.err(loc, format!("{f}: the dimension must be a constant")),
                }
            }
            None => 1,
        };
        if f == "$size" && self.vars[&name].0.kind == NetKind::String && self.vars[&name].1.is_empty() {
            return Ok(Expr::Call(format!("{name}.len"), Vec::new()));
        }
        let unpacked = self.vars[&name].1.len();
        let dims = self.var_dims(&name).unwrap();
        let x = Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: "x".into(), fill: true });
        match f {
            "$dimensions" => return Ok(num(dims.len() as i64)),
            "$unpacked_dimensions" => return Ok(num(unpacked as i64)),
            _ => {}
        }
        let Some((l, r)) = (k >= 1).then(|| dims.get(k as usize - 1)).flatten().cloned() else { return Ok(x) };
        let cmp = |op: BinOp, a: &Expr, b: &Expr, t: Expr, e: Expr| match (lit(a), lit(b)) {
            (Some(x), Some(y)) => {
                let c = match op {
                    BinOp::Ge => x >= y,
                    _ => x <= y,
                };
                if c {
                    t
                } else {
                    e
                }
            }
            _ => Expr::Cond(Box::new(Expr::Binary(op, Box::new(a.clone()), Box::new(b.clone()))), Box::new(t), Box::new(e)),
        };
        Ok(match f {
            "$left" => l,
            "$right" => r,
            "$low" => cmp(BinOp::Le, &l, &r, l.clone(), r.clone()),
            "$high" => cmp(BinOp::Ge, &l, &r, l.clone(), r.clone()),
            "$size" => dim_size(&(l, r)),
            _ => cmp(BinOp::Ge, &l, &r, num(1), num(-1)),
        })
    }
}

/// The definition of a forward-declared type later in the scope.
/// A class named `name` is declared in `rest` (or in a generate block there).
fn find_class(rest: &[&t::Item], name: &str) -> bool {
    rest.iter().any(|it| match it {
        t::Item::Class(c) => c.name == name,
        t::Item::Generate(v, _) => find_class(&v.iter().collect::<Vec<_>>(), name),
        _ => false,
    })
}

fn find_typedef<'a>(rest: &[&'a t::Item], name: &str) -> Option<&'a t::Typedef> {
    for it in rest {
        match it {
            t::Item::Typedef(td @ t::Typedef::Type { name: n, .. }) if n == name => return Some(td),
            t::Item::Generate(v, _) => {
                let r: Vec<&t::Item> = v.iter().collect();
                if let Some(td) = find_typedef(&r, name) {
                    return Some(td);
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn net_type(n: t::NetTypeKw) -> NetType {
    use t::NetTypeKw as N;
    match n {
        N::Tri0 => NetType::Tri0,
        N::Tri1 => NetType::Tri1,
        N::Supply0 => NetType::Supply0,
        N::Supply1 => NetType::Supply1,
        N::Wand | N::Triand => NetType::Wand,
        N::Wor | N::Trior => NetType::Wor,
        N::Trireg => NetType::Trireg,
        _ => NetType::Wire,
    }
}

/// `(strength0, strength1)`: levels 0 = highz .. 7 = supply, a missing one strong; charge
/// strengths give None.
pub(crate) fn drive_strength(s: &t::Strength) -> Option<(u8, u8)> {
    let t::Strength::Drive(a, b) = s else { return None };
    let (mut s0, mut s1) = (6u8, 6u8);
    for k in std::iter::once(*a).chain(*b) {
        let s = k.as_str();
        let lvl = match &s[..s.len() - 1] {
            "supply" => 7,
            "strong" => 6,
            "pull" => 5,
            "weak" => 3,
            "highz" => 0,
            _ => continue,
        };
        if s.ends_with('0') {
            s0 = lvl;
        } else {
            s1 = lvl;
        }
    }
    Some((s0, s1))
}
