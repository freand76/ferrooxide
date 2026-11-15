//! Data types: typedefs, declaration heads, parameter types, struct member chains, enum
//! methods, array queries, assignment patterns and casts (what `svtypes.rs` does in the old
//! parser).

use super::Lower;
use crate::ast::*;
use crate::parser::dec;
use crate::svtypes::{add, builtin, cast, descending, dim_pos, dim_size, lit, minus_one, mul, num, qualify_expr, sub, Struct, SvType};
use crate::syntax::tree as t;
use crate::Result;
use std::rc::Rc;

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
                    let Some(ft) = self.data_type(&m.ty)? else {
                        return self.err(m.loc, "expected a member type");
                    };
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
                Ok(Some(SvType { kind, signed, dims, fields: Some(Rc::new(Struct { union: st.union, fields })), int_kind: false, enum_names: None, unpacked: vec![], param: None }))
            }
            t::TypeKind::Enum(et) => {
                if !ty.packed.is_empty() {
                    return self.unsupported(ty.loc, "packed dimensions after an enum");
                }
                let int32 = |signed: bool| SvType { kind: NetKind::Int(32), signed, dims: vec![(num(31), num(0))], fields: None, int_kind: true, enum_names: None, unpacked: vec![], param: None };
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
                    if it.range.is_some() {
                        return self.unsupported(it.loc, "an enum name range");
                    }
                    let name = self.name(&it.name, it.loc);
                    let value = match &it.value {
                        Some(e) => self.expr(e)?,
                        None => match &prev {
                            Some(p) => add(Expr::Ident(p.clone()), num(1)),
                            None => num(0),
                        },
                    };
                    self.pending.push(Item::Param(ParamDecl { local: true, signed, range: range.clone(), integer: false, real: false, name: name.clone(), value, line: it.loc.line }));
                    names.push(name.clone());
                    prev = Some(name);
                }
                let mut t = base;
                t.enum_names = Some(Rc::new(names));
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
                Ok(Some(SvType { kind, signed, dims, fields: None, int_kind: iw.is_some(), enum_names: None, unpacked: vec![], param: None }))
            }
            t::TypeKind::Real(t::RealKw::Real | t::RealKw::Realtime) => {
                let (kind, signed, iw) = builtin("real").unwrap();
                let mut dims = vec![(num(iw.unwrap() as i64 - 1), num(0))];
                dims.extend(self.dims(&ty.packed, ty.loc)?);
                Ok(Some(SvType { kind, signed, dims, fields: None, int_kind: true, enum_names: None, unpacked: vec![], param: None }))
            }
            t::TypeKind::Named(n) => {
                if n.params.is_some() || n.scopes.iter().any(|s| s.params.is_some()) || n.scopes.len() > 1 {
                    return self.unsupported(ty.loc, "a parameterized class type");
                }
                if let Some(s) = n.scopes.first() {
                    // `pkg::T`: the package's type becomes visible here
                    let Some(p) = self.packages.get(&s.name).cloned() else { return Ok(None) };
                    let Some(t) = p.types.get(&n.name) else { return Ok(None) };
                    self.types.insert(n.name.clone(), t.clone());
                    self.use_package(&s.name);
                }
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
                    }
                }
                if let Some(t) = self.types.get(name) {
                    let (ok, what) = match kind {
                        Some(Kw::Enum) => (t.enum_names.is_some(), "enum"),
                        Some(Kw::Struct) => (t.fields.as_ref().is_some_and(|f| !f.union), "struct"),
                        Some(Kw::Union) => (t.fields.as_ref().is_some_and(|f| f.union), "union"),
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
                    SvType { kind: NetKind::String, signed: false, dims: vec![], fields: None, int_kind: false, enum_names: None, unpacked: vec![], param: None }
                } else {
                    match self.data_type(ty)? {
                        Some(t) => t,
                        None => return self.err(*loc, "expected a type after typedef"),
                    }
                };
                let name = self.name(name, *loc);
                self.qualify_type(&mut t);
                for d in dims {
                    if matches!(d, t::Dim::Unsized | t::Dim::Queue(_) | t::Dim::Assoc(_)) {
                        return self.err(*loc, "dynamic arrays and queues in typedefs are not supported");
                    }
                    let mut r = self.dim(d, *loc)?;
                    if let Some((scope, names)) = &self.type_scope {
                        qualify_expr(&mut r.0, scope, names);
                        qualify_expr(&mut r.1, scope, names);
                    }
                    t.unpacked.push(r);
                }
                self.types.insert(name, Rc::new(t));
                Ok(())
            }
        }
    }

    /// Identifiers in the dimensions of a type declared in a package or at file scope that
    /// name parameters of that scope refer to them there.
    fn qualify_type(&self, t: &mut SvType) {
        let Some((scope, names)) = &self.type_scope else { return };
        if names.is_empty() {
            return;
        }
        fn ty(t: &mut SvType, scope: &str, names: &std::collections::HashSet<String>) {
            for d in t.dims.iter_mut().chain(t.unpacked.iter_mut()) {
                qualify_expr(&mut d.0, scope, names);
                qualify_expr(&mut d.1, scope, names);
            }
            if let Some(f) = &mut t.fields {
                let f = Rc::make_mut(f);
                for (_, ft) in &mut f.fields {
                    ty(ft, scope, names);
                }
            }
        }
        ty(t, scope, names);
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
            t::TypeKind::Real(t::RealKw::Real | t::RealKw::Realtime) => real = true,
            _ => return self.unsupported(ty.loc, "a parameter of this type"),
        }
        let range = match ty.packed.as_slice() {
            [] => None,
            [d] => Some(self.dim(d, ty.loc)?),
            _ => return self.unsupported(ty.loc, "a parameter with several packed dimensions"),
        };
        Ok(PType { signed, integer, real, range, sv: None })
    }

    /// One `name = value` of a value parameter.
    pub(crate) fn param_assign(&mut self, local: bool, pt: &PType, d: &t::Declarator) -> Result<ParamDecl> {
        let name = self.name(&d.name, d.loc);
        if !d.dims.is_empty() {
            return self.unsupported(d.loc, "an unpacked array parameter");
        }
        let Some(v) = &d.init else {
            return self.unsupported(d.loc, "a parameter without a value");
        };
        let value = self.expr(v)?;
        let value = match &pt.sv {
            Some(t) => self.resolve_pattern(value, t, d.loc)?,
            None => value,
        };
        self.decl_type = pt.sv.clone().map(Rc::new);
        self.declare_var(&name, &[]);
        Ok(ParamDecl { local, signed: pt.signed, range: pt.range.clone(), integer: pt.integer, real: pt.real, name, value, line: d.loc.line })
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
            let t = self.data_type_req(ty)?;
            let (kind, signed, range, packed) = t.decl_parts();
            let type_param = t.param.clone();
            self.decl_type = Some(Rc::new(t));
            let netk = net.map(net_type).unwrap_or_default();
            return Ok(Decl { dir, kind, net: netk, explicit_kind: true, real_net: false, strength, delay, signed, range, packed, type_param, names: vec![], line });
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
                    t::TypeKind::Real(t::RealKw::Real | t::RealKw::Realtime) => {
                        kind = NetKind::Real;
                        real_net = true;
                    }
                    _ => return self.unsupported(ty.loc, "a net of this data type"),
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
                    t::TypeKind::Real(t::RealKw::Real | t::RealKw::Realtime) => Some(NetKind::Real),
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
        Ok(Decl { dir, kind, net: netk, explicit_kind, real_net, strength, delay, signed, range, packed, type_param: None, names: Vec::new(), line })
    }

    /// The declarators of a declaration (after `decl_head`).
    pub(crate) fn declarators(&mut self, v: &[t::Declarator]) -> Result<Vec<Declarator>> {
        let mut out = Vec::new();
        for d in v {
            let name = self.name(&d.name, d.loc);
            let mut dims = self.dims(&d.dims, d.loc)?;
            // an array typedef: its dimensions come before the declaration's own
            if let Some(t) = &self.decl_type {
                dims.splice(0..0, t.unpacked.iter().cloned());
            }
            let mut init = match &d.init {
                Some(e) => Some(self.expr(e)?),
                None => None,
            };
            if let (Some(e), Some(t)) = (&init, self.decl_type.clone()) {
                if t.unpacked.is_empty() {
                    init = Some(self.resolve_pattern(e.clone(), &t, d.loc)?);
                }
            }
            self.declare_var(&name, &dims);
            out.push(Declarator { name, dims, init, line: d.loc.line });
        }
        Ok(out)
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

    /// A reference to a variable with a struct type: indexes, members and selects become one
    /// `[off +: w]` select of the flat vector. Returns the expression and the ops left over.
    pub(crate) fn struct_access<'a>(&mut self, name: String, ops: &'a [Op<'a>], lvalue: bool, loc: t::Loc) -> Result<(Expr, &'a [Op<'a>])> {
        let (t, unpacked) = self.vars[&name].clone();
        let mut base = Expr::Ident(name);
        let mut k = 0;
        for _ in 0..unpacked.len() {
            match ops.get(k) {
                Some(Op::Index(i)) => {
                    let i = self.expr(i)?;
                    base = Expr::Index(Box::new(base), Box::new(i));
                    k += 1;
                }
                Some(Op::Range(..)) => return self.unsupported(loc, "a slice of an unpacked struct array"),
                _ => {
                    self.last_type = None;
                    return Ok((base, &ops[k..]));
                }
            }
        }
        let mut cur: SvType = (*t).clone();
        let mut di = 0;
        let mut off = num(0);
        let mut width = cur.width();
        let mut touched = false;
        let mut whole = true;
        loop {
            match ops.get(k) {
                Some(op @ (Op::Index(_) | Op::Range(..))) if di < cur.dims.len() => {
                    let r = cur.dims[di].clone();
                    let elem = cur.width_from(di + 1);
                    k += 1;
                    match op {
                        Op::Index(a) => {
                            let a = self.expr(a)?;
                            off = add(off, mul(dim_pos(&r, a), elem.clone()));
                            width = elem;
                            di += 1;
                            touched = true;
                            continue;
                        }
                        Op::Range(t::RangeKind::Colon, a, b) => {
                            let a = self.expr(a)?;
                            let b = self.expr(b)?;
                            off = add(off, mul(dim_pos(&r, b.clone()), elem.clone()));
                            width = mul(dim_size(&(a, b)), elem);
                            whole = false;
                        }
                        Op::Range(kind, a, n) => {
                            let up = *kind == t::RangeKind::Plus;
                            let a = self.expr(a)?;
                            let n = self.expr(n)?;
                            let low = match (up, descending(&r)) {
                                (true, true) | (false, false) => a,
                                (true, false) => sub(add(a, n.clone()), num(1)),
                                (false, true) => add(sub(a, n.clone()), num(1)),
                            };
                            off = add(off, mul(dim_pos(&r, low), elem.clone()));
                            width = mul(n, elem);
                            whole = false;
                        }
                        _ => unreachable!(),
                    }
                    touched = true;
                    break;
                }
                Some(Op::Member(m, mloc)) if di == cur.dims.len() && cur.fields.is_some() => {
                    let st = cur.fields.clone().unwrap();
                    let Some(fk) = st.fields.iter().position(|f| f.0 == *m) else {
                        return self.err(*mloc, format!("no member '{m}' in the struct"));
                    };
                    k += 1;
                    if !st.union {
                        for f in &st.fields[fk + 1..] {
                            off = add(off, f.1.width());
                        }
                    }
                    cur = st.fields[fk].1.clone();
                    di = 0;
                    width = cur.width();
                    touched = true;
                    continue;
                }
                _ => break,
            }
        }
        self.last_type = if whole {
            let mut rest = cur.clone();
            rest.dims.drain(..di);
            if di > 0 {
                rest.int_kind = false;
            }
            Some(rest)
        } else {
            None
        };
        if !touched {
            return Ok((base, &ops[k..]));
        }
        let e = Expr::IndexedRange(Box::new(base), Box::new(off), Box::new(width), true);
        // a whole signed member reads as signed
        if !lvalue && whole && cur.signed && di == 0 {
            return Ok((Expr::SysCall("$signed".into(), vec![e]), &ops[k..]));
        }
        Ok((e, &ops[k..]))
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
                        _ => return self.err(loc, format!(".{m}() needs a constant count")),
                    },
                };
                let step = if m == "next" { k % n } else { n - k % n };
                let t = self.vars[&name].0.clone();
                cast(t.width(), chain(&|i| Expr::Ident(names[(i + step) % n].clone()), x), u64::from(t.signed))
            }
            _ => Expr::SysCall("$__str".into(), vec![chain(&|i| Expr::Str(names[i].clone()), Expr::Str(String::new()))]),
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

    /// Replace assignment patterns in `e` (at its top, or in a conditional's branches) by
    /// concatenations for type `t`.
    pub(crate) fn resolve_pattern(&self, e: Expr, t: &SvType, loc: t::Loc) -> Result<Expr> {
        match e {
            Expr::SysCall(n, items) if n == "$__pattern" => self.build_pattern(items, t, loc),
            Expr::Cond(c, a, b) => Ok(Expr::Cond(c, Box::new(self.resolve_pattern(*a, t, loc)?), Box::new(self.resolve_pattern(*b, t, loc)?))),
            e => Ok(e),
        }
    }

    fn build_pattern(&self, items: Vec<Expr>, t: &SvType, loc: t::Loc) -> Result<Expr> {
        let mut pos = Vec::new();
        let mut named = Vec::new();
        let mut default = None;
        for it in items {
            match it {
                Expr::SysCall(n, mut kv) if n == "$__named" => {
                    let v = kv.pop().unwrap();
                    let Some(Expr::Str(k)) = kv.pop() else { unreachable!() };
                    if k == "default" {
                        default = Some(v);
                    } else {
                        named.push((k, v));
                    }
                }
                v => pos.push(v),
            }
        }
        let parts: Vec<(Option<String>, SvType)> = if !t.dims.is_empty() && !t.int_kind {
            let r = &t.dims[0];
            let n = match (lit(&r.0), lit(&r.1)) {
                (Some(a), Some(b)) => (a - b).unsigned_abs() as usize + 1,
                _ => return self.err(loc, "an assignment pattern needs constant dimensions"),
            };
            let mut el = t.clone();
            el.dims.remove(0);
            (0..n).map(|_| (None, el.clone())).collect()
        } else if let Some(s) = &t.fields {
            if s.union {
                return self.err(loc, "assignment patterns for unions are not supported");
            }
            s.fields.iter().map(|(n, ft)| (Some(n.clone()), ft.clone())).collect()
        } else {
            return match default {
                Some(v) => Ok(cast(t.width(), v, 0)),
                None => Ok(Expr::Concat(pos.into_iter().map(|v| cast(num(1), v, 0)).collect())),
            };
        };
        if !pos.is_empty() && pos.len() != parts.len() {
            return self.err(loc, format!("assignment pattern has {} values for {} elements", pos.len(), parts.len()));
        }
        let mut out = Vec::new();
        for (k, (name, pt)) in parts.iter().enumerate() {
            let v = if let Some(v) = pos.get(k) {
                v.clone()
            } else if let Some((_, v)) = named.iter().find(|(n, _)| Some(n) == name.as_ref()) {
                v.clone()
            } else if let Some(v) = &default {
                v.clone()
            } else {
                return self.err(loc, format!("assignment pattern has no value for member {}", name.clone().unwrap_or_default()));
            };
            let v = self.resolve_pattern(v, pt, loc)?;
            out.push(match v {
                Expr::Concat(_) => v,
                v => cast(pt.width(), v, 0),
            });
        }
        Ok(Expr::Concat(out))
    }
}

/// The definition of a forward-declared type later in the scope.
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
