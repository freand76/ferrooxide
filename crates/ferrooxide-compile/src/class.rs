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
    /// the interface classes it implements (of an interface class: extends)
    pub ifaces: Vec<u32>,
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
    /// per dimension: declared with a descending range (`[4:2]`)
    pub desc: Vec<bool>,
    /// the enum of the property (of an element; NONE: not an enum)
    pub enum_id: u32,
    /// the layout of a struct property (member chains: `Compiler::struct_walk`)
    pub layout: Option<Rc<crate::types::Packed>>,
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
    /// specializations of parameterized classes: (class, parameter values) -> class, by
    /// the values as written and as evaluated in the class
    pub specs: HashMap<String, u32>,
    pub spec_count: u32,
    /// the scope declaring each class
    pub decl_scope: HashMap<u32, usize>,
    /// names of specializations of classes still being declared (`C #(T)` inside C): their
    /// scopes (`Hier::pkg_scopes`) once the classes are declared
    pub pending_names: Vec<(String, u32)>,
}

/// The scope of a class that is reserved but not yet declared (`ClassInfo::scope`).
const UNDECLARED: usize = usize::MAX;

impl<'a> Compiler<'a> {
    /// The signal that object property writes touch (`Program::obj_sig`), made on first use.
    pub(crate) fn obj_sig(&mut self) -> u32 {
        if let Some(s) = self.prog.obj_sig {
            return s;
        }
        let s = self.new_signal("$objects".into(), 1, false, 0, 0, 0, 0, SigKind::Reg);
        self.prog.names.pop();
        self.prog.obj_sig = Some(s);
        s
    }

    /// The id of a class to be declared (`declare_class`): a placeholder until then.
    pub(crate) fn reserve_class(&mut self, c: &'a ClassDecl, name: &str, sid: usize) -> u32 {
        let id = self.classes.list.len() as u32;
        self.classes.decl_scope.insert(id, sid);
        self.classes.list.push(ClassInfo { name: name.to_string(), decl: c, base: None, ifaces: vec![], scope: UNDECLARED, props: vec![], init_scope: 0, init_this: 0 });
        self.prog.classes.push(rt::ClassDef { name: name.to_string(), width: 0, init: vec![] });
        id
    }

    /// Declare class `c` as `name` in scope `sid` into the reserved class `id`: its
    /// parameters (with `overrides` for a specialization), its properties' layout and start
    /// values.
    pub(crate) fn declare_class(&mut self, c: &'a ClassDecl, sid: usize, name: &str, overrides: &[(Option<String>, PVal)], id: u32) -> R<()> {
        self.line = c.line;
        let path = format!("{}.{name}", self.scopes[sid].name);
        let file = self.file.clone();
        let scope = self.new_scope(path.clone(), Some(sid), file.clone());
        // (names of the class in its declarations are this class: `class_of_scope`)
        self.classes.of_scope.insert(scope, id);
        // the parameter ports, then the specializations the class uses (their values may
        // use the parameters)
        self.eval_param_items(&c.params, overrides, scope)?;
        for s in &c.specs {
            if let Item::ClassSpec { name, class, args, line } = s {
                self.line = *line;
                self.class_spec(name, class, args, scope)?;
            }
        }
        self.line = c.line;
        let base = match &c.base {
            Some(b) => match self.find_class(b, scope) {
                None => return self.err(format!("unknown base class '{b}'")),
                Some(x) if self.classes.list[x as usize].scope == UNDECLARED => {
                    return self.err(format!("base class '{b}' declared after class '{}' is not supported", c.name))
                }
                x => x,
            },
            None => None,
        };
        let mut ifaces = Vec::new();
        for i in &c.interfaces {
            match self.find_class(i, scope) {
                None => return self.err(format!("unknown interface class '{i}'")),
                Some(x) if self.classes.list[x as usize].scope == UNDECLARED => {
                    return self.err(format!("interface class '{i}' is used before its declaration"))
                }
                Some(x) => ifaces.push(x),
            }
        }
        // parameters and enum constants: the base class's (unless the class has its own of
        // that name), then its own (in its scope, where the methods find them first); `C::P`
        // reaches them like a package's
        if let Some(b) = base {
            let bs = self.classes.list[b as usize].scope;
            // (and the static properties of a parameterized base, declared in its scope)
            let statics: HashSet<&str> = self.base_static_props(b);
            let inherited: Vec<(String, Sym)> = self.scopes[bs]
                .syms
                .iter()
                .filter(|(k, s)| matches!(s, Sym::Const(_) | Sym::Real(_)) || statics.contains(k.as_str()))
                .map(|(k, s)| (k.clone(), s.clone()))
                .collect();
            for (k, s) in inherited {
                self.scopes[scope].syms.entry(k).or_insert(s);
            }
        }
        for p in &c.consts {
            self.line = p.line;
            let v = self.param_value(p, None, scope)?;
            self.insert_param(p, v, scope)?;
        }
        self.hier.pkg_scopes.entry(name.to_string()).or_insert(scope);
        self.declarations(&c.static_props, scope, false, &HashMap::new(), None)?;
        let mut props = base.map(|b| self.classes.list[b as usize].props.clone()).unwrap_or_default();
        let mut width = props.last().map(|p| p.off + p.size()).unwrap_or(0);
        // (4-state properties start as x)
        let mut xs: Vec<(u32, u32)> = Vec::new();
        // (64 bits at each of these offsets start as 0: real and string struct members)
        let mut zeros: Vec<u32> = Vec::new();
        for d in &c.props {
            self.line = d.line;
            let (base_ty, layout) = self.prop_type(d, scope)?;
            let enum_id = match d.ty.enum_names.clone().filter(|_| d.ty.dims.len() <= 1) {
                Some(n) => self.enum_id(&n, scope),
                None => NONE,
            };
            let four = !matches!(d.kind(), NetKind::Bit | NetKind::Int(_) | NetKind::Real | NetKind::String | NetKind::Class);
            for n in &d.names {
                // a queue, dynamic or associative array: a handle (0: empty)
                let ty = match &n.heap {
                    Some(h) => self.dyn_prop_type(d, base_ty, h, scope)?,
                    None => base_ty,
                };
                // a fixed-size array
                let mut dims = Vec::new();
                let mut desc = Vec::new();
                for (a, b) in &n.dims {
                    if n.heap.is_some() {
                        return self.err("an array of queues or dynamic arrays as a class property is not supported");
                    }
                    let (a, b) = (self.const_int(a, scope)?, self.const_int(b, scope)?);
                    dims.push((a.min(b), ((a - b).abs() + 1) as u32));
                    desc.push(a > b);
                }
                if layout.is_some() && (n.heap.is_some() || !n.dims.is_empty()) {
                    return self.err("an array of structs as a class property is not supported");
                }
                let p = Prop { name: n.name.clone(), off: width, ty, dims, desc, enum_id, layout: layout.clone() };
                if four && n.heap.is_none() {
                    xs.push((width, p.size()));
                }
                // (real and string members of a struct start as 0.0 and the empty string)
                if let Some(l) = &layout {
                    let mut zero = Vec::new();
                    crate::types::value_members(l, 0, &mut zero);
                    if !zero.is_empty() {
                        self.sigs.has_struct_values = true;
                        self.intern(b"");
                    }
                    zeros.extend(zero.into_iter().map(|z| width + z as u32));
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
        for b in zeros.into_iter().flat_map(|z| z..z + 64) {
            init[(b / 64) as usize] &= !(1 << (b % 64));
            init[n + (b / 64) as usize] &= !(1 << (b % 64));
        }
        self.prog.classes[id as usize] = rt::ClassDef { name: name.to_string(), width, init };
        let init_scope = self.new_scope(format!("{path}.$init"), Some(scope), file);
        let init_this = self.new_signal(format!("{path}.$init.this"), 64, false, 0, 63, 0, 0, SigKind::Reg);
        self.prog.names.pop();
        self.sigs.types[init_this as usize] = Type::Class(id);
        self.scopes[init_scope].syms.insert("this".into(), Sym::Sig(init_this));
        self.classes.list[id as usize] = ClassInfo { name: name.to_string(), decl: c, base, ifaces: ifaces.clone(), scope, props, init_scope, init_this };
        self.check_interfaces(id)?;
        self.classes.by_name.insert((sid, name.to_string()), id);
        self.classes.of_scope.insert(scope, id);
        for (n, _) in self.classes.pending_names.iter().filter(|(_, c)| *c == id) {
            self.hier.pkg_scopes.insert(n.clone(), scope);
        }
        self.classes.pending_names.retain(|(_, c)| *c != id);
        Ok(())
    }

    /// The names of the static properties of parameterized classes that class `c` (and its
    /// base classes) declare in their scopes.
    fn base_static_props(&self, mut c: u32) -> HashSet<&'a str> {
        let mut out = HashSet::new();
        loop {
            let ci = &self.classes.list[c as usize];
            for it in &ci.decl.static_props {
                if let Item::Decl(d) = it {
                    out.extend(d.names.iter().map(|n| n.name.as_str()));
                }
            }
            match ci.base {
                Some(b) => c = b,
                None => return out,
            }
        }
    }

    /// `C #(args)` used in scope `sid` as class `name` (`Item::ClassSpec`): the class with
    /// these parameter values, declared where C is (once for equal values).
    pub(crate) fn class_spec(&mut self, name: &str, class: &str, args: &'a [Conn], sid: usize) -> R<()> {
        let Some(g) = self.find_class(class, sid) else { return self.err(format!("unknown class '{class}'")) };
        let (decl, dscope) = (self.classes.list[g as usize].decl, self.classes.decl_scope[&g]);
        if decl.params.is_empty() {
            return self.err(format!("class '{class}' has no parameters"));
        }
        let mut ovs = Vec::new();
        for a in args {
            ovs.push(match a {
                Conn::Positional(Some(e)) => (None, self.pval(e, sid)?),
                Conn::Positional(None) => (None, PVal::Default),
                Conn::Named(n, Some(e)) => (Some(n.clone()), self.pval(e, sid)?),
                Conn::Named(_, None) => continue,
                Conn::Wildcard => return self.err("`.*` in class parameters"),
            });
        }
        let written = format!("{g}#{:?}", ovs.iter().map(|(n, v)| (n.clone(), Self::pval_key(v))).collect::<Vec<_>>());
        let id = match self.classes.specs.get(&written) {
            Some(&id) => id,
            None => {
                self.classes.spec_count += 1;
                // (`C#sN`: not a name of the lowering's specializations, `C#N`)
                let own = format!("{}#s{}", decl.name, self.classes.spec_count);
                let id = self.reserve_class(decl, &own, dscope);
                // (the class may use this specialization itself: `typedef C #(T) this_type;`)
                self.classes.specs.insert(written.clone(), id);
                self.at(0, |c| c.declare_class(decl, dscope, &own, &ovs, id))?;
                // the same parameter values as the class itself or another specialization:
                // that class (the new one goes unused)
                let key = self.spec_key(g, id);
                let generic = self.classes.list[g as usize].scope != UNDECLARED;
                let id = match self.classes.specs.get(&key) {
                    Some(&other) => other,
                    None if generic && key == self.spec_key(g, g) => g,
                    None => id,
                };
                self.classes.specs.insert(key, id);
                self.classes.specs.insert(written, id);
                id
            }
        };
        self.classes.by_name.insert((sid, name.to_string()), id);
        let cs = self.classes.list[id as usize].scope;
        if cs == UNDECLARED {
            self.classes.pending_names.push((name.to_string(), id));
        } else {
            self.hier.pkg_scopes.insert(name.to_string(), cs);
        }
        Ok(())
    }

    /// A parameter value as text, for the key of a specialization.
    fn pval_key(v: &PVal) -> String {
        match v {
            PVal::Int(c) | PVal::Str(c) => format!("{c:?}"),
            PVal::Default => "default".into(),
            PVal::Real(r) => format!("{}", r.to_bits()),
            PVal::Type(t) => format!("{t:?}"),
            PVal::Array(w) => format!("{w:?}"),
        }
    }

    /// The key of class `id` (a specialization of `g`, or `g` itself) by the values its
    /// parameter ports have.
    fn spec_key(&self, g: u32, id: u32) -> String {
        let c = &self.classes.list[id as usize];
        let mut k = format!("{g}=");
        for p in &c.decl.params {
            let n = match p {
                Item::Param(p) => &p.name,
                Item::TypeParam { name, .. } => name,
                _ => continue,
            };
            let v = match self.scopes[c.scope].syms.get(n) {
                Some(Sym::Const(c)) => format!("{c:?}"),
                Some(Sym::Real(r)) => format!("{}", r.to_bits()),
                Some(Sym::Type(t)) => format!("{t:?}"),
                _ => "?".into(),
            };
            k.push_str(&format!("{n}={v};"));
        }
        k
    }

    /// The type of a class property, and its layout for a struct.
    fn prop_type(&mut self, d: &Decl, sid: usize) -> R<(Type, Option<Rc<crate::types::Packed>>)> {
        // a struct type (also given by a type parameter): stored flat
        if d.ty.has_struct() || d.ty.param.is_some() {
            let r = self.resolve_type(&d.ty, sid)?;
            if r.has_struct() {
                if r.heap.is_some() || !r.unpacked.is_empty() {
                    return self.err("an array of structs as a class property is not supported");
                }
                let l = crate::types::packed_of(&r);
                self.sigs.has_structs = true;
                return Ok((Type::vec(l.width() as u32, l.signed), Some(Rc::new(l))));
            }
            if d.ty.param.is_some() {
                return Ok((self.resolved_type(&r, sid)?, None));
            }
        }
        Ok((self.prop_type_plain(d, sid)?, None))
    }

    /// The type of a class property of no struct type.
    fn prop_type_plain(&mut self, d: &Decl, sid: usize) -> R<Type> {
        Ok(match d.kind() {
            NetKind::Real => Type::Real,
            NetKind::String => Type::Str,
            NetKind::Class => Type::Class(self.class_of_decl(d, sid)?),
            NetKind::Event => return self.err("an event as a class property is not supported"),
            NetKind::Integer => Type::vec(32, true),
            NetKind::Int(n) if d.range().is_none() => Type::vec(n, d.signed()),
            NetKind::Time if d.range().is_none() => Type::vec(64, false),
            _ => {
                let (m, l) = self.range_vals(&d.range(), sid)?;
                let mut w = ((m - l).abs() + 1) as u32;
                for (a, b) in d.packed() {
                    w *= ((self.const_int(a, sid)? - self.const_int(b, sid)?).abs() + 1) as u32;
                }
                Type::vec(w, d.signed())
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
        let elem = match Some(&d.ty).filter(|t| t.param.is_some()) {
            // elements of a type parameter's type
            Some(t) => {
                let r = self.resolve_type(t, sid)?;
                self.elem_for(r.kind, w, r.signed, Some(&r), sid)?
            }
            None => self.elem_for(d.kind(), w, signed, Some(&*d.ty), sid)?,
        };
        if elem.leaf == crate::types::Leaf::Str || matches!(kind, DynKind::Assoc(rt::KeyKind::Str)) {
            self.intern(b"");
        }
        self.sigs.has_dyn = true;
        Ok(Type::Dyn { kind, elem })
    }

    /// The class a declaration of a class type names.
    pub(crate) fn class_of_decl(&self, d: &Decl, sid: usize) -> R<u32> {
        let name = d.ty.class.clone().unwrap_or_default();
        self.class_named(&name, sid)
    }

    /// The class `name` names in scope `sid` as the type of a handle: an error for none, and
    /// for a parameterized class with a parameter without a default (only its
    /// specializations exist).
    pub(crate) fn class_named(&self, name: &str, sid: usize) -> R<u32> {
        let c = self.find_class(name, sid).ok_or_else(|| self.mkerr(format!("unknown class '{name}'")))?;
        let ci = &self.classes.list[c as usize];
        if Self::needs_params(ci.decl) && ci.name == ci.decl.name {
            return self.err(format!("class '{name}' has parameters without a default: it needs parameter values ('{name} #(...)')"));
        }
        Ok(c)
    }

    /// Class `c` has a parameter without a default value: only its specializations exist.
    pub(crate) fn needs_params(c: &ClassDecl) -> bool {
        c.params.iter().any(|p| matches!(p, Item::Param(ParamDecl { value: None, .. }) | Item::TypeParam { default: None, .. }))
    }

    /// The class named `name` visible from scope `sid`.
    pub(crate) fn find_class(&self, name: &str, mut sid: usize) -> Option<u32> {
        // a type parameter (or typedef) naming a class
        if let Some(Sym::Type(t)) = self.lookup(name, sid) {
            if let Some(cn) = t.class.as_deref().filter(|cn| *cn != name) {
                return self.find_class(cn, sid);
            }
        }
        if let Some((a, n)) = name.rsplit_once("::") {
            // `P::C`: class C of package P
            if let Some(c) = self.hier.pkg_scopes.get(a).and_then(|ps| self.classes.by_name.get(&(*ps, n.to_string()))) {
                return Some(*c);
            }
            // `C::T`: the class typedef T of class C names (in C's scope)
            let c = &self.classes.list[self.find_class(a, sid)? as usize];
            let t = c.decl.types.iter().find(|(k, _)| k == n)?.1.class.clone()?;
            return self.find_class(&t, c.scope);
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

    /// The class scope `sid` is in (its own scope, a method's or a property initializer's)
    /// and the class's name as declared (also for a specialization being declared).
    pub(crate) fn enclosing_class_scope(&self, mut sid: usize) -> Option<(usize, &str)> {
        loop {
            if let Some(&c) = self.classes.of_scope.get(&sid) {
                return Some((sid, &self.classes.list[c as usize].decl.name));
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

    /// Member `m` of class `c` used in scope `sid`: a `local` one only inside the class
    /// declaring it, a `protected` one only there and in classes derived from it (IEEE
    /// 1800-2017 8.18).
    pub(crate) fn check_access(&self, c: u32, m: &str, sid: usize) -> R<()> {
        let mut x = Some(c);
        while let Some(k) = x {
            let ci = &self.classes.list[k as usize];
            if let Some(&(_, local)) = ci.decl.hidden.iter().find(|h| h.0 == m) {
                let from = self.class_of_scope(sid);
                let ok = match from {
                    Some(a) if local => a == k,
                    Some(mut a) => loop {
                        if a == k {
                            break true;
                        }
                        match self.classes.list[a as usize].base {
                            Some(b) => a = b,
                            None => break false,
                        }
                    },
                    None => false,
                };
                if !ok {
                    let what = if local { "local" } else { "protected" };
                    return self.err(format!("'{m}' is a {what} member of class '{}' and not visible here", ci.name));
                }
                return Ok(());
            }
            // declared here without a qualifier: public
            if ci.decl.props.iter().any(|d| d.names.iter().any(|n| n.name == m)) || ci.decl.methods.iter().any(|f| f.name == m) {
                return Ok(());
            }
            x = ci.base;
        }
        Ok(())
    }

    /// A method of class `c` or of a base class: (method, the class declaring it).
    pub(crate) fn find_method(&self, c: u32, name: &str) -> Option<(&'a SubroutineDecl, u32)> {
        let mut x = Some(c);
        while let Some(k) = x {
            let ci = &self.classes.list[k as usize];
            if let Some(f) = ci.decl.methods.iter().find(|f| f.name == name) {
                return Some((f, k));
            }
            x = ci.base;
        }
        // (a method an abstract class leaves to a derived class: its interface's prototype)
        let mut x = Some(c);
        while let Some(k) = x {
            let ci = &self.classes.list[k as usize];
            for &i in &ci.ifaces {
                let mut v = Vec::new();
                self.iface_methods(i, &mut v);
                if let Some(&m) = v.iter().find(|(f, _)| f.name == name) {
                    return Some(m);
                }
            }
            x = ci.base;
        }
        None
    }

    /// A parameter or enum constant `name` of class `c` (or of a base class).
    pub(crate) fn class_const(&self, c: u32, name: &str) -> Option<Sym> {
        let s = self.classes.list[c as usize].scope;
        match self.scopes[s].syms.get(name) {
            Some(x @ (Sym::Const(_) | Sym::Real(_))) => Some(x.clone()),
            _ => None,
        }
    }

    /// `T::x` through type parameter `T` (`ExprKind::ClassMember`): the name to evaluate
    /// instead and the scope to evaluate it in, a constant of the class (in the class's
    /// scope) or a static property (its variable `C$x`, beside the class); also of a base
    /// class. None for any other expression.
    /// Also an enum method of a variable of a type parameter's enum (`enum_method`).
    pub(crate) fn class_member(&self, e: &Expr, sid: usize) -> R<Option<(Expr, usize)>> {
        if let Some(x) = self.enum_method(e, sid)? {
            return Ok(Some((x, sid)));
        }
        let ExprKind::ClassMember { class, name } = &e.kind else { return Ok(None) };
        let cn = match self.lookup(class, sid) {
            Some(Sym::Type(t)) => t.class.clone(),
            // (a specialization `C#k`)
            _ => Some(class.clone()),
        };
        let Some(mut c) = cn.and_then(|n| self.find_class(&n, sid)) else {
            return self.err(format!("'{class}::{name}': '{class}' is not a class here"));
        };
        loop {
            let ci = &self.classes.list[c as usize];
            // (a static property of a parameterized class: in the class's scope)
            if self.class_const(c, name).is_some() || ci.decl.static_props.iter().any(|d| matches!(d, Item::Decl(d) if d.names.iter().any(|x| x.name == *name))) {
                return Ok(Some((ExprKind::Ident(name.clone()).into(), ci.scope)));
            }
            let var = format!("{}${name}", ci.decl.name);
            if let Some(parent) = self.scopes[ci.scope].parent {
                if self.scopes[parent].syms.contains_key(&var) {
                    return Ok(Some((ExprKind::Ident(var).into(), parent)));
                }
            }
            match ci.base {
                Some(b) => c = b,
                None => return self.err(format!("class '{}' has no static property or constant '{name}'", self.classes.list[c as usize].name)),
            }
        }
    }

    /// `obj.f` naming a method of the object's class that is no property: a call without
    /// arguments (IEEE 1800-2017 13.5.5, `obj.f()`).
    fn paren_free_call(&mut self, e: &Expr, sid: usize) -> R<Option<Expr>> {
        let ExprKind::Member(b, m) = &e.kind else { return Ok(None) };
        let Some(c) = self.handle_class(b, sid)? else { return Ok(None) };
        let ci = &self.classes.list[c as usize];
        if ci.props.iter().any(|p| p.name == *m) || self.class_const(c, m).is_some() || self.find_method(c, m).is_none() {
            return Ok(None);
        }
        Ok(Some(Expr::new(ExprKind::MethodCall(b.clone(), m.clone(), vec![]), e.line)))
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
    pub(crate) fn scalar_prop(&self, c: u32, name: &str) -> R<Prop> {
        let p = self.prop(c, name)?;
        if !p.dims.is_empty() {
            return self.err(format!("the whole array property '{name}' is not supported here (only its elements)"));
        }
        Ok(p)
    }

    /// An element of a fixed-size array property (`obj.a[i][j]`): (the handle expression,
    /// the property, an index per dimension); None if `e` is no such element.
    pub(crate) fn prop_elem<'e>(&mut self, e: &'e Expr, sid: usize) -> R<Option<(&'e Expr, Prop, Vec<&'e Expr>)>> {
        let mut idx = Vec::new();
        let mut cur = e;
        while let ExprKind::Index(a, i) = &cur.kind {
            idx.push(&**i);
            cur = a;
        }
        let ExprKind::Member(b, m) = &cur.kind else { return Ok(None) };
        if idx.is_empty() {
            return Ok(None);
        }
        let Some(c) = self.handle_class(b, sid)? else { return Ok(None) };
        if self.class_const(c, m).is_some() {
            return Ok(None);
        }
        self.check_access(c, m, sid)?;
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

    /// A whole fixed-size array property (`obj.a`): its element width and dimensions, each
    /// (lowest index, size, descending); None for any other expression.
    pub(crate) fn prop_array_dims(&mut self, e: &Expr, sid: usize) -> R<Option<(u32, Vec<(i64, u32, bool)>)>> {
        let ExprKind::Member(b, m) = &e.kind else { return Ok(None) };
        if self.classes.list.is_empty() {
            return Ok(None);
        }
        let Some(c) = self.handle_class(b, sid)? else { return Ok(None) };
        let Some(p) = self.classes.list[c as usize].props.iter().rev().find(|p| p.name == *m).cloned() else { return Ok(None) };
        if p.dims.is_empty() || self.class_const(c, m).is_some() {
            return Ok(None);
        }
        self.check_access(c, m, sid)?;
        Ok(Some((p.ty.width(), p.dims.iter().zip(&p.desc).map(|(&(lo, n), &d)| (lo, n, d)).collect())))
    }

    /// The flat element index (32-bit signed) of indices `idx` of array property `p`.
    fn prop_index(&mut self, p: &Prop, idx: &[&Expr], sid: usize) -> R<u32> {
        let b = |x: Expr| Box::new(x);
        let num = |v: i64| -> Expr {
            let n = ExprKind::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
            if v < 0 {
                ExprKind::Unary(UnOp::Neg, b(n.into())).into()
            } else {
                n.into()
            }
        };
        let mut lin: Option<Expr> = None;
        for (k, &(lo, n)) in p.dims.iter().enumerate() {
            let term = ExprKind::Binary(BinOp::Sub, b(idx[k].clone()), b(num(lo)));
            lin = Some(match lin {
                None => term.into(),
                Some(l) => ExprKind::Binary(BinOp::Add, b(ExprKind::Binary(BinOp::Mul, b(l), b(num(n as i64))).into()), b(term.into())).into(),
            });
        }
        self.expr(&lin.unwrap(), sid, 32, true)
    }

    /// The class of a handle expression (`super`: the base class of the method's class).
    pub(crate) fn handle_class(&mut self, e: &Expr, sid: usize) -> R<Option<u32>> {
        if let ExprKind::Ident(n) = &e.kind {
            if n == "super" {
                let Some(c) = self.class_of_scope(sid) else { return self.err("`super` outside a class") };
                return match self.classes.list[c as usize].base {
                    Some(b) => Ok(Some(b)),
                    None => self.err(format!("class '{}' has no base class", self.classes.list[c as usize].name)),
                };
            }
        }
        if !matches!(&e.kind, ExprKind::Ident(_) | ExprKind::Hier(_) | ExprKind::Member(..) | ExprKind::MethodCall(..) | ExprKind::Index(..) | ExprKind::Call(..) | ExprKind::StaticCall(..)) {
            return Ok(None);
        }
        Ok(match self.type_of(e, sid)? {
            Type::Class(c) => Some(c),
            _ => None,
        })
    }

    /// The slot of a handle's value (`super`: `this`).
    pub(crate) fn handle_value(&mut self, e: &Expr, sid: usize) -> R<u32> {
        let e = match &e.kind {
            ExprKind::Ident(n) if n == "super" => &Expr::new(ExprKind::Ident("this".into()), e.line),
            _ => e,
        };
        self.expr(e, sid, 64, false)
    }

    /// The type of a property access or a method call, None for other expressions.
    pub(crate) fn obj_type_of(&mut self, e: &Expr, sid: usize) -> R<Option<Type>> {
        if let Some(call) = self.paren_free_call(e, sid)? {
            return self.obj_type_of(&call, sid);
        }
        match &e.kind {
            ExprKind::Null | ExprKind::New(_) => Ok(Some(Type::Class(NONE))),
            ExprKind::NewCopy(x) => self.type_of(x, sid).map(Some),
            ExprKind::NewTyped(c, _) => Ok(Some(Type::Class(self.named_class(c, sid)?))),
            ExprKind::StaticCall(cn, m, _) => {
                let (f, owner) = self.static_method(cn, m, sid)?;
                if f.is_task || f.ret_kind() == NetKind::Int(0) {
                    return self.err(format!("'{m}' gives no value"));
                }
                let scope = self.classes.list[owner as usize].scope;
                self.func_ret_type(f, scope).map(Some)
            }
            ExprKind::Member(b, m) => match self.handle_class(b, sid)? {
                Some(c) => Ok(Some(match self.class_const(c, m) {
                    Some(Sym::Const(k)) => Type::vec(k.w, k.signed),
                    Some(Sym::Real(_)) => Type::Real,
                    _ => self.scalar_prop(c, m)?.ty,
                })),
                None => Ok(None),
            },
            // (a character of a string property: a byte)
            ExprKind::Index(a, _) if matches!(a.kind, ExprKind::Member(..)) && matches!(self.type_of(a, sid), Ok(Type::Str)) => Ok(Some(Type::vec(8, false))),
            ExprKind::Index(..) => Ok(self.prop_elem(e, sid)?.map(|(_, p, _)| p.ty)),
            ExprKind::MethodCall(o, m, _) => {
                let Some(c) = self.handle_class(o, sid)? else {
                    // (a method of an array property: `heap.rs`)
                    match self.type_of(o, sid)? {
                        Type::Dyn { .. } => return Ok(None),
                        // a string method with an integral result (`c.name.len()`)
                        Type::Str => {
                            if let Some((w, s)) = Self::str_method_width(m) {
                                return Ok(Some(Type::vec(w, s)));
                            }
                        }
                        _ => {}
                    }
                    return self.err(format!("'.{m}()' of a value that is not an object"));
                };
                let Some((f, owner)) = self.find_method(c, m) else {
                    return self.err(format!("class '{}' has no method '{m}'", self.classes.list[c as usize].name));
                };
                if f.is_task || f.ret_kind() == NetKind::Int(0) {
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
        if let Some(call) = self.paren_free_call(e, sid)? {
            return self.obj_value(&call, sid);
        }
        match &e.kind {
            ExprKind::Null => Ok(Some((self.const_slot(&Const::zero(64)), Type::Class(NONE)))),
            ExprKind::New(_) => self.err("`new` needs a class handle to assign to"),
            ExprKind::NewTyped(c, args) => {
                let c = self.named_class(c, sid)?;
                Ok(Some((self.construct(c, args, sid)?, Type::Class(c))))
            }
            ExprKind::StaticCall(cn, m, args) => {
                let t = self.obj_type_of(e, sid)?.unwrap();
                let (r, _, _) = self.static_call(cn, m, args, sid)?;
                Ok(Some((r.unwrap(), t)))
            }
            ExprKind::NewCopy(x) => {
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
            ExprKind::Member(b, m) => {
                let Some(c) = self.handle_class(b, sid)? else { return Ok(None) };
                // a parameter or enum constant of the class
                match self.class_const(c, m) {
                    Some(Sym::Const(k)) => return Ok(Some((self.const_slot(&k), Type::vec(k.w, k.signed)))),
                    Some(Sym::Real(r)) => return Ok(Some((self.const_slot(&Const::from_i64(r.to_bits() as i64, 64, false)), Type::Real))),
                    _ => {}
                }
                self.check_access(c, m, sid)?;
                let p = self.scalar_prop(c, m)?;
                let h = self.handle_value(b, sid)?;
                let w = p.ty.width();
                let d = self.tmp(w);
                self.emit(Instr::Obj { op: ObjOp::Get, d, a: h, b: NONE, off: p.off, w });
                Ok(Some((d, p.ty)))
            }
            ExprKind::Index(..) => {
                let Some((b, p, idx)) = self.prop_elem(e, sid)? else { return Ok(None) };
                let h = self.handle_value(b, sid)?;
                let ix = self.prop_index(&p, &idx, sid)?;
                let w = p.ty.width();
                let d = self.tmp(w);
                self.emit(Instr::Obj { op: ObjOp::GetAt(p.count()), d, a: h, b: ix, off: p.off, w });
                Ok(Some((d, p.ty)))
            }
            ExprKind::MethodCall(o, m, args) => {
                let Some(t) = self.obj_type_of(e, sid)? else { return Ok(None) };
                let (r, _, _) = self.method_call(o, m, args, sid)?;
                Ok(Some((r.unwrap(), t)))
            }
            _ => Ok(None),
        }
    }

    /// A handle value for a target of class `c` (`new(args)` makes an object of class `c`).
    pub(crate) fn handle_for(&mut self, e: &Expr, c: u32, sid: usize) -> R<u32> {
        match &e.kind {
            ExprKind::New(args) => self.construct(c, args, sid),
            ExprKind::Null => Ok(self.const_slot(&Const::zero(64))),
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

    /// Class `c` is `base` or derives from it, or implements interface class `base`.
    pub(crate) fn derives(&self, c: u32, base: u32) -> bool {
        if c == base {
            return true;
        }
        let ci = &self.classes.list[c as usize];
        ci.base.is_some_and(|b| self.derives(b, base)) || ci.ifaces.iter().any(|&i| self.derives(i, base))
    }

    /// The methods of interface class `i` and of those it extends: (method, the interface
    /// class declaring it), the first of each name.
    fn iface_methods(&self, i: u32, out: &mut Vec<(&'a SubroutineDecl, u32)>) {
        let ci = &self.classes.list[i as usize];
        for f in &ci.decl.methods {
            if !out.iter().any(|(g, _)| g.name == f.name) {
                out.push((f, i));
            }
        }
        for &j in &ci.ifaces {
            self.iface_methods(j, out);
        }
    }

    /// The rules of interface classes (IEEE 1800-2017 8.26) for class `c`, just declared:
    /// an interface class inherits a method name from one interface class only, unless it
    /// declares its own; a class implements each method of its interface classes (an
    /// abstract one may leave them to a derived class), with the prototype's signature.
    fn check_interfaces(&mut self, c: u32) -> R<()> {
        let ci = &self.classes.list[c as usize];
        if ci.ifaces.is_empty() {
            return Ok(());
        }
        let (decl, ifaces, cname) = (ci.decl, ci.ifaces.clone(), ci.name.clone());
        let mut all: Vec<(&'a SubroutineDecl, u32)> = Vec::new();
        for &i in &ifaces {
            let mut v = Vec::new();
            self.iface_methods(i, &mut v);
            for (f, owner) in v {
                if decl.interface && !decl.methods.iter().any(|g| g.name == f.name) {
                    if let Some(&(_, o2)) = all.iter().find(|(g, o)| g.name == f.name && *o != owner) {
                        let (a, b) = (&self.classes.list[o2 as usize].name, &self.classes.list[owner as usize].name);
                        return self.err(format!("interface class '{cname}' inherits method '{}' from both '{a}' and '{b}'; it must declare its own", f.name));
                    }
                }
                all.push((f, owner));
            }
        }
        if decl.interface {
            return Ok(());
        }
        for (proto, owner) in all {
            let iname = self.classes.list[owner as usize].name.clone();
            // the implementation: in the class or a base class (not a pure one)
            let mut x = Some(c);
            let mut found = None;
            while let Some(k) = x {
                let ki = &self.classes.list[k as usize];
                if let Some(f) = ki.decl.methods.iter().find(|f| f.name == proto.name) {
                    found = Some((f, k, ki.decl.pures.contains(&f.name)));
                    break;
                }
                x = ki.base;
            }
            match found {
                None | Some((_, _, true)) if !decl.abstract_ => {
                    return self.err(format!("class '{cname}' does not implement method '{}' of interface class '{iname}'", proto.name));
                }
                Some((f, k, _)) => {
                    let ks = self.classes.list[k as usize].scope;
                    let os = self.classes.list[owner as usize].scope;
                    if !self.same_signature(f, ks, proto, os)? {
                        return self.err(format!(
                            "method '{}' of class '{cname}' does not match its prototype in interface class '{iname}'",
                            f.name
                        ));
                    }
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Two methods have the same kind, return type and arguments (directions and types).
    fn same_signature(&mut self, f: &SubroutineDecl, fs: usize, g: &SubroutineDecl, gs: usize) -> R<bool> {
        if f.is_task != g.is_task {
            return Ok(false);
        }
        let void = |h: &SubroutineDecl| h.ret_kind() == NetKind::Int(0);
        if void(f) != void(g) || (!f.is_task && !void(f) && !self.types_match(&f.ret_ty, fs, &g.ret_ty, gs)?) {
            return Ok(false);
        }
        let ports = |h: &'_ SubroutineDecl| -> Vec<(Dir, std::rc::Rc<ferrooxide_front::lower::SvType>, usize)> {
            h.decls.iter().filter_map(|d| d.dir.map(|x| (x, d.ty.clone(), d.names.len()))).flat_map(|(x, t, n)| std::iter::repeat_n((x, t, 1), n)).collect()
        };
        let (pf, pg) = (ports(f), ports(g));
        if pf.len() != pg.len() {
            return Ok(false);
        }
        for ((da, ta, _), (db, tb, _)) in pf.iter().zip(&pg) {
            if da != db || !self.types_match(ta, fs, tb, gs)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `new(args)` for class `c`: the slot of the new object's handle.
    fn construct(&mut self, c: u32, args: &[Expr], sid: usize) -> R<u32> {
        let ci = &self.classes.list[c as usize];
        if ci.decl.abstract_ {
            let what = if ci.decl.interface { "interface" } else { "virtual" };
            return self.err(format!("cannot create an object of {what} class '{}'", ci.name));
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
                        let lhs: Expr = ExprKind::Member(Box::new(ExprKind::Ident("this".into()).into()), n.name.clone()).into();
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
                Stmt::MethodCall { obj: Expr { kind: ExprKind::Ident(o), .. }, name, .. } => return o == "super" && name == "new",
                _ => return false,
            }
        }
    }

    /// `obj.m(args)`: (result slot, width, signed).
    pub(crate) fn method_call(&mut self, obj: &Expr, m: &str, args: &[Expr], sid: usize) -> R<(Option<u32>, u32, bool)> {
        let is_super = matches!(&obj.kind, ExprKind::Ident(n) if n == "super");
        if is_super && m == "new" {
            // the base part of the object being constructed, then this class's initializers
            let Some(c) = self.class_of_scope(sid) else { return self.err("`super` outside a class") };
            let h = self.handle_value(obj, sid)?;
            self.ctor_prologue(c, h, Some(args), sid)?;
            return Ok((None, 0, false));
        }
        let Some(c) = self.handle_class(obj, sid)? else {
            if let Some((w, s)) = Self::str_method_width(m) {
                if self.type_of(obj, sid)?.is_str() {
                    let d = self.str_method_call(obj, m, args, sid)?;
                    return Ok((Some(d), w, s));
                }
            }
            return self.err(format!("'.{m}()' of a value that is not an object"));
        };
        let Some((f, owner)) = self.find_method(c, m) else {
            return self.err(format!("class '{}' has no method '{m}'", self.classes.list[c as usize].name));
        };
        self.check_access(c, m, sid)?;
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
            Sym::Sig(s) if matches!(self.sig_type(s), Type::Class(_)) => Some((ExprKind::Hier(parts).into(), m.to_string())),
            _ => None,
        }
    }

    /// `inst.obj.p`, `inst.s.f.g` (a hierarchical name that resolves to nothing else) as
    /// member selects of its longest prefix that is a class handle or a struct variable.
    pub(crate) fn hier_member(&self, e: &Expr, sid: usize) -> Option<Expr> {
        let ExprKind::Hier(p) = &e.kind else { return None };
        if self.lookup_hier(p, sid).is_some() {
            return None;
        }
        for k in (2..p.len()).rev() {
            // (an element of an array variable: `inst.v[i].m`, the index in the segment)
            let mut prefix = p[..k].to_vec();
            let mut index = None;
            let sym = match self.lookup_hier(&prefix, sid) {
                None => {
                    let last = prefix.last_mut().unwrap();
                    let Some(i) = last.find('[').filter(|_| last.ends_with(']')) else { continue };
                    let ix = &last[i + 1..last.len() - 1];
                    index = Some(match ix.strip_prefix('@') {
                        Some(n) => ferrooxide_front::lower::hier_index_expr(n.parse().ok()?)?,
                        None => ExprKind::Num(ferrooxide_front::ast::dec(ix.parse().ok()?)).into(),
                    });
                    last.truncate(i);
                    match self.lookup_hier(&prefix, sid) {
                        Some(s) => s,
                        None => continue,
                    }
                }
                Some(s) => s,
            };
            match sym {
                Sym::Sig(s) if matches!(self.sig_type(s), Type::Class(_)) || self.layout(s).is_some_and(|l| l.fields.is_some()) => {
                    let mut x: Expr = ExprKind::Hier(prefix).into();
                    if let Some(i) = index {
                        x = ExprKind::Index(Box::new(x), Box::new(i)).into();
                    }
                    for m in &p[k..] {
                        x = ExprKind::Member(Box::new(x), m.clone()).into();
                    }
                    return Some(x);
                }
                _ => return None,
            }
        }
        None
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
    fn is_virtual(&self, c: u32, m: &str) -> bool {
        let ci = &self.classes.list[c as usize];
        ci.decl.virtuals.iter().any(|v| v == m) || ci.base.is_some_and(|b| self.is_virtual(b, m)) || ci.ifaces.iter().any(|&i| self.is_virtual(i, m))
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
            // (no objects of an interface class)
            if d == c || !self.derives(d, c) || self.classes.list[d as usize].decl.interface {
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
        let ret_t = if dflt.0.is_task || dflt.0.ret_kind() == NetKind::Int(0) {
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
        // a member (or a part of one) of a struct property
        if self.sigs.has_structs {
            if let Some((prop, off, w, walk)) = self.prop_part(lhs, sid)? {
                let ExprKind::Member(b, m) = &prop.kind else { unreachable!() };
                let c = self.handle_class(b, sid)?.unwrap();
                self.check_access(c, m, sid)?;
                let p = self.scalar_prop(c, m)?;
                let t = match walk.leaf {
                    crate::types::Leaf::Real => Type::Real,
                    crate::types::Leaf::Str => Type::Str,
                    crate::types::Leaf::Bits => Type::vec(w as u32, walk.ty.as_ref().is_some_and(|t| t.signed)),
                };
                let v = self.value_for(rhs, sid, w as u32, t, walk.ty.as_ref())?;
                let h = self.handle_value(b, sid)?;
                self.emit(Instr::Obj { op: ObjOp::Set, d: NONE, a: h, b: v, off: p.off + off as u32, w: w as u32 });
                return Ok(true);
            }
        }
        let ExprKind::Member(b, m) = &lhs.kind else { return Ok(false) };
        let Some(c) = self.handle_class(b, sid)? else { return Ok(false) };
        self.check_access(c, m, sid)?;
        let p = self.scalar_prop(c, m)?;
        let v = match p.ty {
            Type::Class(pc) => self.handle_for(rhs, pc, sid)?,
            t => self.value_for(rhs, sid, t.width(), t, p.layout.as_deref())?,
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
        Ok(match &e.kind {
            ExprKind::Member(b, _) => self.handle_class(b, sid)?.is_some() || self.has_prop(b, sid)?,
            ExprKind::Index(a, _) | ExprKind::Range(a, ..) | ExprKind::IndexedRange(a, ..) => self.has_prop(a, sid)?,
            ExprKind::Concat(v) => {
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
