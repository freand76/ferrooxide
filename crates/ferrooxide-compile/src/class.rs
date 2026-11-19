//! Classes: objects on the simulator's heap, referred to by 64-bit handles (0: `null`).
//!
//! An object holds its properties flat (the base class's first), as an unpacked struct is
//! stored; `Instr::Obj` reads and writes a property at its offset. Methods are inlined like
//! functions, with an implicit input `this` (the handle). A constructor first runs the base
//! class's constructor (`super.new(..)` where the body starts with it), then the class's
//! property initializers, then its body. Objects are shared and never freed.

use crate::types::{DynKind, Type};
use crate::*;
use rt::ObjOp;

/// A class of the design (`Program::classes` has the same index).
pub(crate) struct ClassInfo<'a> {
    pub name: String,
    pub decl: &'a ClassDecl,
    pub base: Option<u32>,
    /// the scope methods are compiled in (its parent is the scope declaring the class)
    pub scope: usize,
    /// properties with those of the base classes first
    pub props: Vec<Prop>,
    /// property initializers run in this scope, `this` being `init_this`
    pub init_scope: usize,
    pub init_this: u32,
}

#[derive(Clone)]
pub(crate) struct Prop {
    pub name: String,
    pub off: u32,
    /// the type (of an element, for an array)
    pub ty: Type,
    /// a fixed-size array: (lowest index, number of elements) per dimension, stored flat
    /// (row-major)
    pub dims: Vec<(i64, u32)>,
}

impl Prop {
    /// The number of elements (1 for a scalar).
    fn count(&self) -> u32 {
        self.dims.iter().map(|d| d.1).product()
    }

    /// Its width in the object.
    fn size(&self) -> u32 {
        self.ty.width() * self.count()
    }
}

#[derive(Default)]
pub(crate) struct Classes<'a> {
    pub list: Vec<ClassInfo<'a>>,
    /// (declaring scope, name) -> class
    pub by_name: HashMap<(usize, String), u32>,
    /// a class's method scope -> the class
    pub of_scope: HashMap<usize, u32>,
}

impl<'a> Compiler<'a> {
    /// Declare a class in scope `sid`: its properties' layout and start values.
    pub(crate) fn declare_class(&mut self, c: &'a ClassDecl, sid: usize) -> R<()> {
        self.line = c.line;
        let base = match &c.base {
            Some(b) => match self.find_class(b, sid) {
                None => return self.err(format!("unknown base class '{b}'")),
                Some(x) if x as usize >= self.classes.list.len() => {
                    return self.err(format!("base class '{b}' declared after class '{}' is not supported", c.name))
                }
                x => x,
            },
            None => None,
        };
        let id = self.classes.list.len() as u32;
        let path = format!("{}.{}", self.scopes[sid].name, c.name);
        let file = self.file.clone();
        let scope = self.new_scope(path.clone(), Some(sid), file.clone());
        // parameters and enum constants: the base class's, then its own (in its scope, where
        // the methods find them first); `C::P` reaches them like a package's
        if let Some(b) = base {
            let bs = self.classes.list[b as usize].scope;
            let inherited: Vec<(String, Sym)> =
                self.scopes[bs].syms.iter().filter(|(_, s)| matches!(s, Sym::Const(_) | Sym::Real(_))).map(|(k, s)| (k.clone(), s.clone())).collect();
            self.scopes[scope].syms.extend(inherited);
        }
        for p in &c.consts {
            self.line = p.line;
            let v = self.param_value(p, None, scope)?;
            self.insert_param(p, v, scope)?;
        }
        self.hier.pkg_scopes.entry(c.name.clone()).or_insert(scope);
        let mut props = base.map(|b| self.classes.list[b as usize].props.clone()).unwrap_or_default();
        let mut width = props.last().map(|p| p.off + p.size()).unwrap_or(0);
        // (4-state properties start as x)
        let mut xs: Vec<(u32, u32)> = Vec::new();
        for d in &c.props {
            self.line = d.line;
            let base_ty = self.prop_type(d, scope)?;
            let four = !matches!(d.kind, NetKind::Bit | NetKind::Int(_) | NetKind::Real | NetKind::String | NetKind::Class);
            for n in &d.names {
                // a queue, dynamic or associative array: a handle (0: empty)
                let ty = match &n.heap {
                    Some(h) => self.dyn_prop_type(d, base_ty, h, scope)?,
                    None => base_ty,
                };
                // a fixed-size array
                let mut dims = Vec::new();
                for (a, b) in &n.dims {
                    if n.heap.is_some() {
                        return self.err("an array of queues or dynamic arrays as a class property is not supported");
                    }
                    let (a, b) = (self.const_int(a, scope)?, self.const_int(b, scope)?);
                    dims.push((a.min(b), ((a - b).abs() + 1) as u32));
                }
                let p = Prop { name: n.name.clone(), off: width, ty, dims };
                if four && n.heap.is_none() {
                    xs.push((width, p.size()));
                }
                width += p.size();
                props.push(p);
            }
        }
        let n = rt::nw(width.max(1));
        let mut init = match base {
            // the base class's start values, widened
            Some(b) => {
                let bw = self.prog.classes[b as usize].width.max(1);
                let (bn, bi) = (rt::nw(bw), &self.prog.classes[b as usize].init);
                let mut v = vec![0u64; 2 * n];
                v[..bn].copy_from_slice(&bi[..bn]);
                v[n..n + bn].copy_from_slice(&bi[bn..2 * bn]);
                v
            }
            None => vec![0u64; 2 * n],
        };
        for (off, w) in xs {
            for k in 0..w {
                let b = off + k;
                init[(b / 64) as usize] |= 1 << (b % 64);
                init[n + (b / 64) as usize] |= 1 << (b % 64);
            }
        }
        self.prog.classes.push(rt::ClassDef { name: c.name.clone(), width, init });
        let init_scope = self.new_scope(format!("{path}.$init"), Some(scope), file);
        let init_this = self.new_signal(format!("{path}.$init.this"), 64, false, 0, 63, 0, 0, SigKind::Reg);
        self.prog.names.pop();
        self.sigs.types[init_this as usize] = Type::Class(id);
        self.scopes[init_scope].syms.insert("this".into(), Sym::Sig(init_this));
        self.classes.list.push(ClassInfo { name: c.name.clone(), decl: c, base, scope, props, init_scope, init_this });
        self.classes.by_name.insert((sid, c.name.clone()), id);
        self.classes.of_scope.insert(scope, id);
        Ok(())
    }

    /// The type of a class property.
    fn prop_type(&mut self, d: &Decl, sid: usize) -> R<Type> {
        if d.ty.as_ref().is_some_and(|t| t.has_struct() || t.param.is_some()) {
            return self.err("a class property of a struct type or a type parameter is not supported");
        }
        Ok(match d.kind {
            NetKind::Real => Type::Real,
            NetKind::String => Type::Str,
            NetKind::Class => Type::Class(self.class_of_decl(d, sid)?),
            NetKind::Event => return self.err("an event as a class property is not supported"),
            NetKind::Integer => Type::vec(32, true),
            NetKind::Int(n) if d.range.is_none() => Type::vec(n, d.signed),
            NetKind::Time if d.range.is_none() => Type::vec(64, false),
            _ => {
                let (m, l) = self.range_vals(&d.range, sid)?;
                let mut w = ((m - l).abs() + 1) as u32;
                for (a, b) in &d.packed {
                    w *= ((self.const_int(a, sid)? - self.const_int(b, sid)?).abs() + 1) as u32;
                }
                Type::vec(w, d.signed)
            }
        })
    }

    /// The type of a property that is a queue, dynamic or associative array of `elem`.
    fn dyn_prop_type(&mut self, d: &Decl, elem: Type, h: &HeapKind, sid: usize) -> R<Type> {
        let (w, signed) = match elem {
            Type::Vec { w, signed } => (w, signed),
            _ => (64, false),
        };
        let kind = self.dyn_kind(h, sid)?;
        let elem = self.elem_for(d.kind, w, signed, d.ty.as_deref(), sid)?;
        if elem.leaf == crate::types::Leaf::Str || matches!(kind, DynKind::Assoc(rt::KeyKind::Str)) {
            self.intern(b"");
        }
        self.sigs.has_dyn = true;
        Ok(Type::Dyn { kind, elem })
    }

    /// The class a declaration of a class type names.
    pub(crate) fn class_of_decl(&self, d: &Decl, sid: usize) -> R<u32> {
        let name = d.ty.as_ref().and_then(|t| t.class.clone()).unwrap_or_default();
        self.find_class(&name, sid).ok_or_else(|| self.mkerr(format!("unknown class '{name}'")))
    }

    /// The class named `name` visible from scope `sid`.
    pub(crate) fn find_class(&self, name: &str, mut sid: usize) -> Option<u32> {
        // `P::C`: class C of package P
        if let Some((p, n)) = name.split_once("::") {
            return self.classes.by_name.get(&(*self.hier.pkg_scopes.get(p)?, n.to_string())).copied();
        }
        loop {
            if let Some(&c) = self.classes.by_name.get(&(sid, name.to_string())) {
                return Some(c);
            }
            for &p in &self.scopes[sid].imports {
                if let Some(&c) = self.classes.by_name.get(&(p, name.to_string())) {
                    return Some(c);
                }
            }
            sid = self.scopes[sid].parent?;
        }
    }

    /// The class whose method (or property initializer) scope `sid` is in.
    pub(crate) fn class_of_scope(&self, mut sid: usize) -> Option<u32> {
        loop {
            if let Some(&c) = self.classes.of_scope.get(&sid) {
                return Some(c);
            }
            sid = self.scopes[sid].parent?;
        }
    }

    /// A method of class `c` or of a base class: (method, the class declaring it).
    pub(crate) fn find_method(&self, mut c: u32, name: &str) -> Option<(&'a SubroutineDecl, u32)> {
        loop {
            let ci = &self.classes.list[c as usize];
            if let Some(f) = ci.decl.methods.iter().find(|f| f.name == name) {
                return Some((f, c));
            }
            c = ci.base?;
        }
    }

    /// A parameter or enum constant `name` of class `c` (or of a base class).
    pub(crate) fn class_const(&self, c: u32, name: &str) -> Option<Sym> {
        let s = self.classes.list[c as usize].scope;
        match self.scopes[s].syms.get(name) {
            Some(x @ (Sym::Const(_) | Sym::Real(_))) => Some(x.clone()),
            _ => None,
        }
    }

    /// The property `name` of class `c`.
    pub(crate) fn prop(&self, c: u32, name: &str) -> R<Prop> {
        let ci = &self.classes.list[c as usize];
        match ci.props.iter().rev().find(|p| p.name == name) {
            Some(p) => Ok(p.clone()),
            None => self.err(format!("class '{}' has no property '{name}'", ci.name)),
        }
    }

    /// Property `name` of class `c`, which is not an array.
    fn scalar_prop(&self, c: u32, name: &str) -> R<Prop> {
        let p = self.prop(c, name)?;
        if !p.dims.is_empty() {
            return self.err(format!("the whole array property '{name}' is not supported here (only its elements)"));
        }
        Ok(p)
    }

    /// An element of a fixed-size array property (`obj.a[i][j]`): (the handle expression,
    /// the property, an index per dimension); None if `e` is no such element.
    fn prop_elem<'e>(&mut self, e: &'e Expr, sid: usize) -> R<Option<(&'e Expr, Prop, Vec<&'e Expr>)>> {
        let mut idx = Vec::new();
        let mut cur = e;
        while let Expr::Index(a, i) = cur {
            idx.push(&**i);
            cur = a;
        }
        let Expr::Member(b, m) = cur else { return Ok(None) };
        if idx.is_empty() {
            return Ok(None);
        }
        let Some(c) = self.handle_class(b, sid)? else { return Ok(None) };
        if self.class_const(c, m).is_some() {
            return Ok(None);
        }
        let p = self.prop(c, m)?;
        if p.dims.is_empty() {
            return Ok(None);
        }
        if idx.len() != p.dims.len() {
            return self.err(format!("'{m}' has {} dimensions: a select of an element or a part of the array is not supported", p.dims.len()));
        }
        idx.reverse();
        Ok(Some((&**b, p, idx)))
    }

    /// The flat element index (32-bit signed) of indices `idx` of array property `p`.
    fn prop_index(&mut self, p: &Prop, idx: &[&Expr], sid: usize) -> R<u32> {
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = Expr::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
            if v < 0 {
                Expr::Unary(UnOp::Neg, b(n))
            } else {
                n
            }
        };
        let mut lin: Option<Expr> = None;
        for (k, &(lo, n)) in p.dims.iter().enumerate() {
            let term = Expr::Binary(BinOp::Sub, b(idx[k].clone()), b(num(lo)));
            lin = Some(match lin {
                None => term,
                Some(l) => Expr::Binary(BinOp::Add, b(Expr::Binary(BinOp::Mul, b(l), b(num(n as i64)))), b(term)),
            });
        }
        self.expr(&lin.unwrap(), sid, 32, true)
    }

    /// The class of a handle expression (`super`: the base class of the method's class).
    pub(crate) fn handle_class(&mut self, e: &Expr, sid: usize) -> R<Option<u32>> {
        if let Expr::Ident(n) = e {
            if n == "super" {
                let Some(c) = self.class_of_scope(sid) else { return self.err("`super` outside a class") };
                return match self.classes.list[c as usize].base {
                    Some(b) => Ok(Some(b)),
                    None => self.err(format!("class '{}' has no base class", self.classes.list[c as usize].name)),
                };
            }
        }
        if !matches!(e, Expr::Ident(_) | Expr::Hier(_) | Expr::Member(..) | Expr::MethodCall(..) | Expr::Index(..) | Expr::Call(..)) {
            return Ok(None);
        }
        Ok(match self.type_of(e, sid)? {
            Type::Class(c) => Some(c),
            _ => None,
        })
    }

    /// The slot of a handle's value (`super`: `this`).
    pub(crate) fn handle_value(&mut self, e: &Expr, sid: usize) -> R<u32> {
        let e = match e {
            Expr::Ident(n) if n == "super" => &Expr::Ident("this".into()),
            e => e,
        };
        self.expr(e, sid, 64, false)
    }

    /// The type of a property access or a method call, None for other expressions.
    pub(crate) fn obj_type_of(&mut self, e: &Expr, sid: usize) -> R<Option<Type>> {
        match e {
            Expr::Null | Expr::New(_) => Ok(Some(Type::Class(NONE))),
            Expr::NewCopy(x) => self.type_of(x, sid).map(Some),
            Expr::NewTyped(c, _) => Ok(Some(Type::Class(self.named_class(c, sid)?))),
            Expr::StaticCall(cn, m, _) => {
                let (f, owner) = self.static_method(cn, m, sid)?;
                if f.is_task || f.ret_kind == NetKind::Int(0) {
                    return self.err(format!("'{m}' gives no value"));
                }
                let scope = self.classes.list[owner as usize].scope;
                self.func_ret_type(f, scope).map(Some)
            }
            Expr::Member(b, m) => match self.handle_class(b, sid)? {
                Some(c) => Ok(Some(match self.class_const(c, m) {
                    Some(Sym::Const(k)) => Type::vec(k.w, k.signed),
                    Some(Sym::Real(_)) => Type::Real,
                    _ => self.scalar_prop(c, m)?.ty,
                })),
                None => Ok(None),
            },
            Expr::Index(..) => Ok(self.prop_elem(e, sid)?.map(|(_, p, _)| p.ty)),
            Expr::MethodCall(o, m, _) => {
                let Some(c) = self.handle_class(o, sid)? else {
                    // (a method of an array property: `heap.rs`)
                    if let Type::Dyn { .. } = self.type_of(o, sid)? {
                        return Ok(None);
                    }
                    return self.err(format!("'.{m}()' of a value that is not an object"));
                };
                let Some((f, owner)) = self.find_method(c, m) else {
                    return self.err(format!("class '{}' has no method '{m}'", self.classes.list[c as usize].name));
                };
                if f.is_task || f.ret_kind == NetKind::Int(0) {
                    return self.err(format!("'{m}' gives no value"));
                }
                let scope = self.classes.list[owner as usize].scope;
                self.func_ret_type(f, scope).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// A property read or a method call: (slot, type); None for other expressions.
    pub(crate) fn obj_value(&mut self, e: &Expr, sid: usize) -> R<Option<(u32, Type)>> {
        match e {
            Expr::Null => Ok(Some((self.const_slot(&Const::zero(64)), Type::Class(NONE)))),
            Expr::New(_) => self.err("`new` needs a class handle to assign to"),
            Expr::NewTyped(c, args) => {
                let c = self.named_class(c, sid)?;
                Ok(Some((self.construct(c, args, sid)?, Type::Class(c))))
            }
            Expr::StaticCall(cn, m, args) => {
                let t = self.obj_type_of(e, sid)?.unwrap();
                let (r, _, _) = self.static_call(cn, m, args, sid)?;
                Ok(Some((r.unwrap(), t)))
            }
            Expr::NewCopy(x) => {
                let Some(c) = self.handle_class(x, sid)? else { return self.err("`new` copies an object") };
                let h = self.handle_value(x, sid)?;
                let d = self.tmp(64);
                self.emit(Instr::Obj { op: ObjOp::Copy, d, a: h, b: NONE, off: 0, w: 0 });
                // array properties are values: the copy gets arrays of its own (8.12)
                let props = self.classes.list[c as usize].props.clone();
                for p in props {
                    if let Type::Dyn { kind, elem } = p.ty {
                        let t = self.tmp(64);
                        self.emit(Instr::Obj { op: ObjOp::Get, d: t, a: d, b: NONE, off: p.off, w: 64 });
                        let n = self.dyn_copy(t, kind, elem);
                        self.emit(Instr::Obj { op: ObjOp::Set, d: NONE, a: d, b: n, off: p.off, w: 64 });
                    }
                }
                Ok(Some((d, Type::Class(c))))
            }
            Expr::Member(b, m) => {
                let Some(c) = self.handle_class(b, sid)? else { return Ok(None) };
                // a parameter or enum constant of the class
                match self.class_const(c, m) {
                    Some(Sym::Const(k)) => return Ok(Some((self.const_slot(&k), Type::vec(k.w, k.signed)))),
                    Some(Sym::Real(r)) => return Ok(Some((self.const_slot(&Const::from_i64(r.to_bits() as i64, 64, false)), Type::Real))),
                    _ => {}
                }
                let p = self.scalar_prop(c, m)?;
                let h = self.handle_value(b, sid)?;
                let w = p.ty.width();
                let d = self.tmp(w);
                self.emit(Instr::Obj { op: ObjOp::Get, d, a: h, b: NONE, off: p.off, w });
                Ok(Some((d, p.ty)))
            }
            Expr::Index(..) => {
                let Some((b, p, idx)) = self.prop_elem(e, sid)? else { return Ok(None) };
                let h = self.handle_value(b, sid)?;
                let ix = self.prop_index(&p, &idx, sid)?;
                let w = p.ty.width();
                let d = self.tmp(w);
                self.emit(Instr::Obj { op: ObjOp::GetAt(p.count()), d, a: h, b: ix, off: p.off, w });
                Ok(Some((d, p.ty)))
            }
            Expr::MethodCall(o, m, args) => {
                let Some(t) = self.obj_type_of(e, sid)? else { return Ok(None) };
                let (r, _, _) = self.method_call(o, m, args, sid)?;
                Ok(Some((r.unwrap(), t)))
            }
            _ => Ok(None),
        }
    }

    /// A handle value for a target of class `c` (`new(args)` makes an object of class `c`).
    pub(crate) fn handle_for(&mut self, e: &Expr, c: u32, sid: usize) -> R<u32> {
        match e {
            Expr::New(args) => self.construct(c, args, sid),
            Expr::Null => Ok(self.const_slot(&Const::zero(64))),
            _ => match self.type_of(e, sid)? {
                Type::Class(x) if x == c || x == NONE || self.derives(x, c) => self.handle_value(e, sid),
                Type::Class(x) => self.err(format!(
                    "an object of class '{}' cannot be assigned to a handle of class '{}'",
                    self.classes.list[x as usize].name, self.classes.list[c as usize].name
                )),
                _ => self.err(format!("a value that is not an object cannot be assigned to a handle of class '{}'", self.classes.list[c as usize].name)),
            },
        }
    }

    /// The class named `name` (of `C::new`).
    fn named_class(&self, name: &str, sid: usize) -> R<u32> {
        match self.find_class(name, sid) {
            Some(c) => Ok(c),
            None => self.err(format!("unknown class '{name}'")),
        }
    }

    /// Class `c` is `base` or derives from it.
    pub(crate) fn derives(&self, mut c: u32, base: u32) -> bool {
        loop {
            if c == base {
                return true;
            }
            match self.classes.list[c as usize].base {
                Some(b) => c = b,
                None => return false,
            }
        }
    }

    /// `new(args)` for class `c`: the slot of the new object's handle.
    fn construct(&mut self, c: u32, args: &[Expr], sid: usize) -> R<u32> {
        let ci = &self.classes.list[c as usize];
        if ci.decl.abstract_ {
            return self.err(format!("cannot create an object of virtual class '{}'", ci.name));
        }
        let h = self.tmp(64);
        self.emit(Instr::Obj { op: ObjOp::New, d: h, a: NONE, b: NONE, off: 0, w: c });
        self.run_ctor(c, h, args, sid)?;
        Ok(h)
    }

    /// The constructor of class `c` on the object at slot `h`.
    fn run_ctor(&mut self, c: u32, h: u32, args: &[Expr], sid: usize) -> R<()> {
        let ci = &self.classes.list[c as usize];
        match ci.decl.methods.iter().find(|f| f.name == "new") {
            Some(f) => {
                let scope = ci.scope;
                self.call_resolved(f, scope, "new", Some(h), args, sid)?;
            }
            None => {
                if !args.is_empty() {
                    return self.err(format!("class '{}' has no constructor with arguments", ci.name));
                }
                self.ctor_prologue(c, h, None, sid)?;
            }
        }
        Ok(())
    }

    /// What a constructor of class `c` does before its body: the base class's constructor
    /// (with `args` from `super.new(args)` in scope `sid`, else those of `extends B(args)`),
    /// then the property initializers.
    pub(crate) fn ctor_prologue(&mut self, c: u32, h: u32, args: Option<&[Expr]>, sid: usize) -> R<()> {
        let ci = &self.classes.list[c as usize];
        let (base, init_scope, init_this, decl) = (ci.base, ci.init_scope, ci.init_this, ci.decl);
        if let Some(b) = base {
            match (args, &decl.base_args) {
                (Some(a), _) => self.run_ctor(b, h, a, sid)?,
                (None, Some(a)) => self.run_ctor(b, h, a, init_scope)?,
                (None, None) => self.run_ctor(b, h, &[], sid)?,
            }
        }
        if decl.props.iter().any(|d| d.names.iter().any(|n| n.init.is_some())) {
            self.emit(Instr::Copy { d: self.sig(init_this).slot, s: h, w: 64 });
            for d in &decl.props {
                for n in &d.names {
                    if let Some(init) = &n.init {
                        let lhs = Expr::Member(Box::new(Expr::Ident("this".into())), n.name.clone());
                        self.prop_assign(&lhs, init, init_scope)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The body of constructor `f` starts with `super.new(..)`.
    pub(crate) fn calls_super_new(f: &SubroutineDecl) -> bool {
        let mut s = &f.body;
        loop {
            match s {
                Stmt::Block { stmts, fork: None, .. } if !stmts.is_empty() => s = &stmts[0],
                Stmt::MethodCall { obj: Expr::Ident(o), name, .. } => return o == "super" && name == "new",
                _ => return false,
            }
        }
    }

    /// `obj.m(args)`: (result slot, width, signed).
    pub(crate) fn method_call(&mut self, obj: &Expr, m: &str, args: &[Expr], sid: usize) -> R<(Option<u32>, u32, bool)> {
        let is_super = matches!(obj, Expr::Ident(n) if n == "super");
        if is_super && m == "new" {
            // the base part of the object being constructed, then this class's initializers
            let Some(c) = self.class_of_scope(sid) else { return self.err("`super` outside a class") };
            let h = self.handle_value(obj, sid)?;
            self.ctor_prologue(c, h, Some(args), sid)?;
            return Ok((None, 0, false));
        }
        let Some(c) = self.handle_class(obj, sid)? else { return self.err(format!("'.{m}()' of a value that is not an object")) };
        let Some((f, owner)) = self.find_method(c, m) else {
            return self.err(format!("class '{}' has no method '{m}'", self.classes.list[c as usize].name));
        };
        if m == "new" {
            return self.err("`new` is called only through `super.new`");
        }
        // a static method needs no object (8.10)
        if self.classes.list[owner as usize].decl.statics.iter().any(|s| s == m) {
            let scope = self.classes.list[owner as usize].scope;
            return self.call_resolved(f, scope, m, None, args, sid);
        }
        let h = self.handle_value(obj, sid)?;
        // (a task may wait: the handle must survive other processes' temporaries)
        let h = if f.is_task {
            let p = self.ptmp(64);
            self.emit(Instr::Copy { d: p, s: h, w: 64 });
            p
        } else {
            h
        };
        let scope = self.classes.list[owner as usize].scope;
        if is_super || !self.is_virtual(owner, m) {
            return self.call_resolved(f, scope, m, Some(h), args, sid);
        }
        self.virtual_call(c, (f, owner), m, h, args, sid)
    }

    /// A dotted name `a.b.c.m` whose prefix `a.b.c` is a class handle in another scope (the
    /// front end knows only its own scope's handles): (the handle, `m`).
    pub(crate) fn hier_handle(&self, name: &str, sid: usize) -> Option<(Expr, String)> {
        if self.classes.list.is_empty() {
            return None;
        }
        let (base, m) = name.rsplit_once('.')?;
        let parts: Vec<String> = base.split('.').map(String::from).collect();
        if parts.len() < 2 {
            return None;
        }
        match self.lookup_hier(&parts, sid)? {
            Sym::Sig(s) if matches!(self.sig_type(s), Type::Class(_)) => Some((Expr::Hier(parts), m.to_string())),
            _ => None,
        }
    }

    /// `inst.obj.p` (a hierarchical name that resolves to nothing else) as a member access.
    pub(crate) fn hier_member(&self, e: &Expr, sid: usize) -> Option<Expr> {
        let Expr::Hier(p) = e else { return None };
        if self.lookup_hier(p, sid).is_some() {
            return None;
        }
        let (obj, m) = self.hier_handle(&p.join("."), sid)?;
        Some(Expr::Member(Box::new(obj), m))
    }

    /// Static method `m` of the class named `cn` (or of a base class): (method, the class
    /// declaring it).
    fn static_method(&self, cn: &str, m: &str, sid: usize) -> R<(&'a SubroutineDecl, u32)> {
        let c = self.named_class(cn, sid)?;
        let Some((f, owner)) = self.find_method(c, m) else {
            return self.err(format!("class '{cn}' has no method '{m}'"));
        };
        if !self.classes.list[owner as usize].decl.statics.iter().any(|s| s == m) {
            return self.err(format!("'{cn}::{m}' is not a static method (it needs an object)"));
        }
        Ok((f, owner))
    }

    /// `C::m(args)`: a call of a static method, without an object.
    pub(crate) fn static_call(&mut self, cn: &str, m: &str, args: &[Expr], sid: usize) -> R<(Option<u32>, u32, bool)> {
        let (f, owner) = self.static_method(cn, m, sid)?;
        let scope = self.classes.list[owner as usize].scope;
        self.call_resolved(f, scope, m, None, args, sid)
    }

    /// Method `m` of class `c` is virtual: declared so there or in a base class.
    fn is_virtual(&self, mut c: u32, m: &str) -> bool {
        loop {
            let ci = &self.classes.list[c as usize];
            if ci.decl.virtuals.iter().any(|v| v == m) {
                return true;
            }
            match ci.base {
                Some(b) => c = b,
                None => return false,
            }
        }
    }

    /// A call of virtual method `m` through a handle of class `c`: the implementation of the
    /// object's class, chosen at run time among those of `c` and the classes derived from it.
    fn virtual_call(
        &mut self,
        c: u32,
        dflt: (&'a SubroutineDecl, u32),
        m: &str,
        h: u32,
        args: &[Expr],
        sid: usize,
    ) -> R<(Option<u32>, u32, bool)> {
        // the classes derived from c by the implementation they use
        let mut impls: Vec<(u32, Vec<u32>)> = Vec::new();
        for d in 0..self.classes.list.len() as u32 {
            if d == c || !self.derives(d, c) {
                continue;
            }
            let (_, owner) = self.find_method(d, m).unwrap();
            if owner == dflt.1 {
                continue;
            }
            match impls.iter_mut().find(|x| x.0 == owner) {
                Some(x) => x.1.push(d),
                None => impls.push((owner, vec![d])),
            }
        }
        if impls.is_empty() {
            let scope = self.classes.list[dflt.1 as usize].scope;
            return self.call_resolved(dflt.0, scope, m, Some(h), args, sid);
        }
        let cls = self.tmp(32);
        self.emit(Instr::Obj { op: ObjOp::Class, d: cls, a: h, b: NONE, off: 0, w: 0 });
        let ret_t = if dflt.0.is_task || dflt.0.ret_kind == NetKind::Int(0) {
            None
        } else {
            Some(self.func_ret_type(dflt.0, self.classes.list[dflt.1 as usize].scope)?)
        };
        let res = ret_t.map(|t| (self.tmp(t.width()), t));
        let mut ends = Vec::new();
        let result = |me: &mut Self, r: (Option<u32>, u32, bool)| {
            if let (Some((d, t)), Some(s)) = (res, r.0) {
                me.emit(Instr::Copy { d, s, w: t.width() });
            }
        };
        for (owner, ids) in impls {
            let mut any = NONE;
            for id in ids {
                let k = self.const_slot(&Const::from_i64(id as i64, 32, false));
                let e = self.tmp(1);
                self.emit(Instr::Cmp { op: rt::CmpOp::Eq, d: e, a: cls, b: k, w: 32 });
                any = if any == NONE {
                    e
                } else {
                    let o = self.tmp(1);
                    self.emit(Instr::Bin { op: rt::BinOp::Or, d: o, a: any, b: e, w: 1 });
                    o
                };
            }
            let skip = self.emit(Instr::Jz { c: any, w: 1, to: 0 });
            let (f, _) = self.find_method(owner, m).unwrap();
            let scope = self.classes.list[owner as usize].scope;
            let r = self.call_resolved(f, scope, m, Some(h), args, sid)?;
            result(self, r);
            ends.push(self.emit(Instr::Jmp { to: 0 }));
            let here = self.here();
            self.patch(skip, here);
        }
        let scope = self.classes.list[dflt.1 as usize].scope;
        let r = self.call_resolved(dflt.0, scope, m, Some(h), args, sid)?;
        result(self, r);
        let end = self.here();
        for j in ends {
            self.patch(j, end);
        }
        Ok(match res {
            Some((d, t)) => (Some(d), t.width(), t.signed()),
            None => (None, 0, false),
        })
    }

    /// `lhs = rhs` where lhs is a property (`obj.p`); false if it is not one.
    pub(crate) fn prop_assign(&mut self, lhs: &Expr, rhs: &Expr, sid: usize) -> R<bool> {
        // `inst.obj.p = v`
        if let Some(m) = self.hier_member(lhs, sid) {
            return self.prop_assign(&m, rhs, sid);
        }
        // an element of an array property
        if let Some((b, p, idx)) = self.prop_elem(lhs, sid)? {
            let v = match p.ty {
                Type::Class(pc) => self.handle_for(rhs, pc, sid)?,
                t => self.value_for(rhs, sid, t.width(), t, None)?,
            };
            let h = self.handle_value(b, sid)?;
            let ix = self.prop_index(&p, &idx, sid)?;
            self.emit(Instr::Obj { op: ObjOp::SetAt(p.count()), d: ix, a: h, b: v, off: p.off, w: p.ty.width() });
            return Ok(true);
        }
        let Expr::Member(b, m) = lhs else { return Ok(false) };
        let Some(c) = self.handle_class(b, sid)? else { return Ok(false) };
        let p = self.scalar_prop(c, m)?;
        let v = match p.ty {
            Type::Class(pc) => self.handle_for(rhs, pc, sid)?,
            t => self.value_for(rhs, sid, t.width(), t, None)?,
        };
        let h = self.handle_value(b, sid)?;
        self.emit(Instr::Obj { op: ObjOp::Set, d: NONE, a: h, b: v, off: p.off, w: p.ty.width() });
        Ok(true)
    }

    /// The assignment target `e` contains a property (`obj.p[3:0]`, ...), which is not
    /// supported as a part.
    pub(crate) fn has_prop(&mut self, e: &Expr, sid: usize) -> R<bool> {
        if let Some(m) = self.hier_member(e, sid) {
            return self.has_prop(&m, sid);
        }
        Ok(match e {
            Expr::Member(b, _) => self.handle_class(b, sid)?.is_some() || self.has_prop(b, sid)?,
            Expr::Index(a, _) | Expr::Range(a, ..) | Expr::IndexedRange(a, ..) => self.has_prop(a, sid)?,
            Expr::Concat(v) => {
                for x in v {
                    if self.has_prop(x, sid)? {
                        return Ok(true);
                    }
                }
                false
            }
            _ => false,
        })
    }
}
