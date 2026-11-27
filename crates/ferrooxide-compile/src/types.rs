//! Types in the compiler: the type of each signal and of each expression.

use crate::expr::Base;
use crate::*;
use ferrooxide_front::ast::BinOp as B;

/// The type of a signal (of an element for an array) or of an expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Type {
    /// a packed vector
    Vec { w: u32, signed: bool },
    /// `real` / `realtime`: the value word holds f64 bits
    Real,
    /// `string`: a 64-bit handle of an interned text
    Str,
    /// a queue, dynamic or associative array: a 64-bit handle of a heap object (`heap.rs`)
    Dyn { kind: DynKind, elem: Elem },
    /// a handle of an object of this class (`classes.list`; NONE: `null`, `new`)
    Class(u32),
    /// an event that is no module's (a class property): a 64-bit handle of an event object
    /// (`rt::EvOp`)
    Event,
}

/// The kind of a `Type::Dyn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DynKind {
    /// a queue holding at most this many elements (0: unbounded)
    Queue(u32),
    Darray,
    Assoc(rt::KeyKind),
}

/// The element type of a queue or dynamic array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Elem {
    pub(crate) w: u32,
    pub(crate) signed: bool,
    pub(crate) leaf: Leaf,
    /// a 2-state type: an element outside the array reads as 0, not x
    pub(crate) two_state: bool,
    /// a handle of an object of this class (`NONE`: not a handle); 64 bits, null outside
    pub(crate) class: u32,
    /// the handle of a queue, dynamic or associative array (`Sigs::nested`, `NONE`: none);
    /// 64 bits, the empty array outside
    pub(crate) nested: u32,
}

impl Elem {
    /// The type of an element value.
    pub(crate) fn ty(self) -> Type {
        if self.class != NONE {
            return Type::Class(self.class);
        }
        match self.leaf {
            Leaf::Bits => Type::vec(self.w, self.signed),
            Leaf::Real => Type::Real,
            Leaf::Str => Type::Str,
        }
    }

    /// An element outside the array reads as x.
    pub(crate) fn x(self) -> bool {
        self.leaf == Leaf::Bits && !self.two_state && self.class == NONE
    }
}

impl Type {
    pub(crate) fn vec(w: u32, signed: bool) -> Type {
        Type::Vec { w, signed }
    }

    /// Width where the value is used as a vector: a real converts to a 64-bit integer, a
    /// string is its 64-bit handle.
    pub(crate) fn width(self) -> u32 {
        match self {
            Type::Vec { w, .. } => w,
            Type::Real | Type::Str | Type::Dyn { .. } | Type::Class(_) | Type::Event => 64,
        }
    }

    /// Sign where the value is used as a vector (a real converts to a signed integer).
    pub(crate) fn signed(self) -> bool {
        match self {
            Type::Vec { signed, .. } => signed,
            Type::Real => true,
            Type::Str | Type::Dyn { .. } | Type::Class(_) | Type::Event => false,
        }
    }

    /// (width, sign) as a vector.
    pub(crate) fn ws(self) -> (u32, bool) {
        (self.width(), self.signed())
    }

    /// The same width and sign as a vector.
    pub(crate) fn as_vec(self) -> Type {
        Type::vec(self.width(), self.signed())
    }

    pub(crate) fn is_real(self) -> bool {
        self == Type::Real
    }

    pub(crate) fn is_str(self) -> bool {
        self == Type::Str
    }
}

/// String methods that give a string.
pub(crate) const STR_METHODS: &[&str] = &["substr", "toupper", "tolower"];

/// System functions that give a real.
const REAL_FUNCS: &[&str] = &[
    "$realtime", "$itor", "$__toreal", "$bitstoreal", "$bitstoshortreal", "$sqrt", "$ln", "$log10", "$exp", "$floor", "$ceil", "$sin", "$cos", "$tan",
    "$asin", "$acos", "$atan", "$sinh", "$cosh", "$tanh", "$asinh", "$acosh", "$atanh", "$atan2", "$hypot", "$pow",
];

impl<'a> Compiler<'a> {
    /// The type of signal `s` (of its elements for an array).
    pub(crate) fn sig_type(&self, s: u32) -> Type {
        self.sigs.types[s as usize]
    }

    /// Register a new signal with its type (every `prog.signals` push goes through here).
    pub(crate) fn push_signal(&mut self, info: SignalInfo, ty: Type) -> u32 {
        let id = self.prog.signals.len() as u32;
        self.prog.signals.push(info);
        self.sigs.types.push(ty);
        id
    }

    /// The type of an expression in a self-determined context (IEEE 1800 11.6, 11.8.1): a
    /// string where an operand of a concatenation, replication or condition is one; a real
    /// where an operand of an arithmetic operator or a condition is real; otherwise a vector.
    pub(crate) fn type_of(&mut self, e: &Expr, sid: usize) -> R<Type> {
        self.at(e.line, |c| c.type_of_here(e, sid))
    }

    fn type_of_here(&mut self, e: &Expr, sid: usize) -> R<Type> {
        if let Some((n, s)) = self.class_member(e, sid)? {
            return self.type_of(&n, s);
        }
        if let Some(x) = self.array_equality(e, sid)? {
            return self.type_of(&x, sid);
        }
        if let Some(x) = self.typename_str(e, sid)? {
            return self.type_of(&x, sid);
        }
        if let Some(x) = self.sampled_call(e, sid)? {
            return self.type_of(&x, sid);
        }
        if !self.classes.list.is_empty() || matches!(&e.kind, ExprKind::Null | ExprKind::New(_) | ExprKind::NewCopy(_) | ExprKind::MethodCall(..)) {
            if let Some(t) = self.obj_type_of(e, sid)? {
                return Ok(t);
            }
        }
        if let Some(t) = self.member_type(e, sid) {
            return Ok(t);
        }
        if self.sigs.has_dyn {
            if let Some(t) = self.dyn_type_of(e, sid)? {
                return Ok(t);
            }
        }
        if let Some(n) = self.md_norm(e, sid) {
            return self.type_of(&n, sid);
        }
        let v = Type::vec;
        Ok(match &e.kind {
            ExprKind::Type(_) => return self.err("a data type is not a value"),
            ExprKind::Binary(_, a, b) if matches!((&a.kind, &b.kind), (ExprKind::Type(_), ExprKind::Type(_))) => v(1, false),
            ExprKind::ClassMember { class, name } => return self.err(format!("'{class}::{name}' is not a value here")),
            ExprKind::Randomize { .. } => Type::vec(32, true),
            ExprKind::Stream { items, .. } => {
                let mut w = 0;
                for x in items {
                    // (a queue or dynamic array: its size is known at run time only)
                    match self.type_of(x, sid)? {
                        Type::Dyn { .. } => {}
                        t => w += t.width(),
                    }
                }
                Type::vec(w, false)
            }
            ExprKind::ArrayWith { method, .. } => return self.err(format!("array method '{method}' with `with` is not supported for this array (only for queues, dynamic arrays and one-dimensional fixed-size arrays of vectors, reals, strings or class handles)")),
            ExprKind::Member(..) => return self.member_error(e, sid),
            ExprKind::NewArray(..) => return self.err("new[] needs a dynamic array to assign to"),
            ExprKind::Null | ExprKind::New(_) | ExprKind::NewTyped(..) | ExprKind::NewCopy(_) | ExprKind::MethodCall(..) | ExprKind::StaticCall(..) => {
                return self.err("no classes are declared")
            }
            ExprKind::EmptyArg => return self.err("an empty argument is allowed only in a call of a function or task"),
            ExprKind::NamedArg(..) => return self.err("a named argument is allowed only in a call of a function or task"),
            ExprKind::Last => return self.err("`$` outside an index of a queue"),
            ExprKind::IncDec { e, .. } | ExprKind::AssignExpr { lhs: e, .. } => return self.type_of(e, sid),
            ExprKind::Tagged(m, _) => return self.err(format!("`tagged {m}` needs a tagged union target here")),
            ExprKind::TagIs(..) => v(1, false),
            ExprKind::Unchecked(x) => return self.type_of(x, sid),
            ExprKind::Pattern(..) => {
                let r = self.typed_pattern(e, sid)?;
                return self.type_of(&r, sid);
            }
            ExprKind::Num(n) => {
                if n.fill {
                    v(1, false)
                } else {
                    let c = Const::from_number(n).or_else(|m| self.err(m))?;
                    v(c.w, c.signed)
                }
            }
            ExprKind::Real(_) => Type::Real,
            ExprKind::Str(s) => v((8 * s.len()).max(8) as u32, false),
            ExprKind::Ident(_) | ExprKind::Hier(_) => match self.resolve_base(e, sid)? {
                Some(Base::Sig(s)) => self.sig_type(s),
                Some(Base::Const(_)) if matches!(&e.kind, ExprKind::Ident(n) if self.is_string_param(n, sid)) => Type::Str,
                Some(Base::Const(c)) => v(c.w, c.signed),
                Some(Base::Real(_)) => Type::Real,
                None => unreachable!(),
            },
            ExprKind::Index(a, _) => {
                if self.is_string_scalar(a, sid) {
                    // a character of a string
                    v(8, false)
                } else if let Some(arr) = self.array_of(a, sid) {
                    self.sig_type(arr)
                } else {
                    v(1, false)
                }
            }
            ExprKind::Range(_, m, l) => match (self.const_index(m, sid)?, self.const_index(l, sid)?) {
                (Some(m), Some(l)) => v(((m - l).abs() + 1) as u32, false),
                // an x/z bound: a 1-bit x
                _ => v(1, false),
            },
            ExprKind::IndexedRange(_, _, w, _) => {
                // (IEEE 1800-2017 11.5.1: a positive width)
                let w = self.const_int(w, sid)?;
                if w <= 0 {
                    return self.err(format!("the width of an indexed part-select is {w}; it must be positive"));
                }
                v(w as u32, false)
            }
            ExprKind::Unary(op, a) => match op {
                UnOp::Plus | UnOp::Neg => match self.type_of(a, sid)? {
                    Type::Real => Type::Real,
                    t => t.as_vec(),
                },
                UnOp::Not => self.type_of(a, sid)?.as_vec(),
                _ => v(1, false),
            },
            ExprKind::Binary(op, a, b) => match op {
                B::Add | B::Sub | B::Mul | B::Div | B::Mod | B::And | B::Or | B::Xor | B::Xnor => {
                    let (ta, tb) = (self.type_of(a, sid)?, self.type_of(b, sid)?);
                    if matches!(op, B::Add | B::Sub | B::Mul | B::Div) && (ta.is_real() || tb.is_real()) {
                        Type::Real
                    } else {
                        v(ta.width().max(tb.width()), ta.signed() && tb.signed())
                    }
                }
                B::Pow => {
                    let ta = self.type_of(a, sid)?;
                    if ta.is_real() || self.type_of(b, sid)?.is_real() {
                        Type::Real
                    } else {
                        ta.as_vec()
                    }
                }
                B::Shl | B::Shr | B::AShl | B::AShr => self.type_of(a, sid)?.as_vec(),
                _ => v(1, false),
            },
            ExprKind::Cond(_, t, f) => {
                let (tt, tf) = (self.type_of(t, sid)?, self.type_of(f, sid)?);
                if tt.is_str() || tf.is_str() {
                    Type::Str
                } else if tt.is_real() || tf.is_real() {
                    Type::Real
                } else {
                    v(tt.width().max(tf.width()), tt.signed() && tf.signed())
                }
            }
            ExprKind::Concat(xs) | ExprKind::Repl(_, xs) => {
                let mut w = 0;
                let mut str = false;
                let mut dynt = None;
                for x in xs {
                    let t = self.type_of(x, sid)?;
                    str |= t.is_str();
                    if let Type::Dyn { .. } = t {
                        dynt.get_or_insert(t);
                    }
                    w += t.width();
                }
                if let (Some(t), ExprKind::Concat(_)) = (dynt, &e.kind) {
                    // (a concatenation with a queue makes a queue)
                    return Ok(t);
                }
                if xs.is_empty() {
                    return self.err("an empty concatenation needs a queue to assign to");
                }
                if str {
                    Type::Str
                } else if let ExprKind::Repl(n, _) = &e.kind {
                    let n = self.const_int(n, sid)?;
                    v((w as i64 * n.max(0)) as u32, false)
                } else {
                    v(w, false)
                }
            }
            ExprKind::Call(name, args) => {
                if let Some((_, m)) = self.str_method(name, sid) {
                    if STR_METHODS.contains(&m) {
                        return Ok(Type::Str);
                    }
                    if m == "atoreal" {
                        return Ok(Type::Real);
                    }
                    if let Some((w, s)) = Self::str_method_width(m) {
                        return Ok(v(w, s));
                    }
                }
                let Some((f, fs)) = self.lookup_func(name, sid).or_else(|| self.class_static_method(name, sid)) else {
                    if let Some((obj, m)) = self.hier_handle(name, sid) {
                        return self.type_of(&Expr::from(ExprKind::MethodCall(Box::new(obj), m, args.clone())), sid);
                    }
                    if let Some((base, m)) = name.rsplit_once('.').filter(|_| !self.classes.list.is_empty()) {
                        let parts: Vec<String> = base.split('.').map(String::from).collect();
                        if let Some(obj) = self.hier_member(&ExprKind::Hier(parts).into(), sid) {
                            return self.type_of(&Expr::from(ExprKind::MethodCall(Box::new(obj), m.to_string(), args.clone())), sid);
                        }
                    }
                    if name == "get_randstate" && self.lookup("this", sid).is_some() {
                        return Ok(Type::Str);
                    }
                    return self.err(format!("unknown function '{name}'"));
                };
                if f.is_task {
                    return self.err(format!("task '{name}' used as a function"));
                }
                self.func_ret_type(f, fs)?
            }
            ExprKind::SysCall(name, args) => match name.as_str() {
                "$sformatf" | "$psprintf" | "$__tostring" | "$__process_get_randstate" => Type::Str,
                n if REAL_FUNCS.contains(&n) => Type::Real,
                "$time" => v(64, false),
                "$stime" => v(32, false),
                "$random" | "$dist_uniform" | "$dist_normal" | "$dist_exponential" | "$dist_poisson" | "$dist_chi_square" | "$dist_t" | "$dist_erlang" => v(32, true),
                "$urandom" | "$urandom_range" => v(32, false),
                "$__process_self" => v(64, false),
                "$__process_status" => v(32, false),
                "$signed" | "$unsigned" => {
                    let a = args.first().ok_or_else(|| self.mkerr(format!("{name} needs an argument")))?;
                    v(self.type_of(a, sid)?.width(), name == "$signed")
                }
                "$clog2" | "$bits" => v(32, false),
                "$isunbounded" => v(1, false),
                "$q_full" => v(32, true),
                // SV cast T'(e) / N'(e): (width, sign: 0, 1, 2 = the expression's, e)
                "$__cast" => {
                    let w = self.const_int(&args[0], sid)?;
                    if !(1..=1 << 20).contains(&w) {
                        return self.err(format!("bad cast width {w}"));
                    }
                    let sg = match self.const_int(&args[1], sid)? {
                        0 => false,
                        1 => true,
                        _ => self.type_of(&args[2], sid)?.signed(),
                    };
                    v(w as u32, sg)
                }
                "$__pattern" => return self.err("an assignment pattern needs a known target type here"),
                "$__str" => self.type_of(&args[0], sid)?.as_vec(),
                "$fopen" => v(32, false),
                "$test$plusargs" | "$value$plusargs" | "$cast" => v(32, true),
                "$fgetc" | "$ungetc" | "$fgets" | "$fread" | "$ferror" | "$feof" | "$ftell" | "$fseek" | "$rewind" | "$fscanf" | "$sscanf" => {
                    v(32, true)
                }
                "$rtoi" => v(32, true),
                "$realtobits" | "$__ticks" | "$__strbits" => v(64, false),
                "$__tag_check" => self.type_of(&args[1], sid)?,
                "$shortrealtobits" => v(32, false),
                "$countones" | "$countbits" => v(32, false),
                "$left" | "$right" | "$low" | "$high" | "$size" | "$increment" | "$dimensions" | "$unpacked_dimensions" => v(32, true),
                "$onehot" | "$onehot0" | "$isunknown" => v(1, false),
                _ => return self.err(format!("unsupported system function '{name}'")),
            },
        })
    }

    /// The type a function returns.
    pub(crate) fn func_ret_type(&mut self, f: &SubroutineDecl, fs: usize) -> R<Type> {
        // a type parameter
        if let Some(t) = Some(&f.ret_ty).filter(|t| t.param.is_some()) {
            let r = self.resolve_type(t, fs)?;
            return self.resolved_type(&r, fs);
        }
        if let Some(t) = Some(&f.ret_ty).filter(|t| t.heap.is_some()) {
            let w = self.const_int(&t.width(), fs)? as u32;
            let elem = self.elem_for(t.kind, w, t.signed, Some(t), fs)?;
            let kind = self.dyn_kind(t.heap.as_ref().unwrap(), fs)?;
            return Ok(Type::Dyn { kind, elem });
        }
        Ok(match f.ret_kind() {
            NetKind::Class => match f.ret_ty.class.as_ref() {
                Some(n) => Type::Class(self.find_class(n, fs).ok_or_else(|| self.mkerr(format!("unknown class '{n}'")))?),
                None => return self.err("a function returning a class handle without its class"),
            },
            NetKind::Integer => Type::vec(32, f.signed()),
            NetKind::Real => Type::Real,
            NetKind::String => Type::Str,
            _ => {
                let (m, l) = self.range_vals(&f.range(), fs)?;
                Type::vec(((m - l).abs() + 1) as u32, f.signed())
            }
        })
    }
}

impl<'a> Compiler<'a> {
    /// The `Type` of a value of resolved type `r` (no type parameters left) in scope `sid`.
    pub(crate) fn resolved_type(&mut self, r: &SvType, sid: usize) -> R<Type> {
        if !r.unpacked.is_empty() || r.heap.is_some() || r.param.is_some() {
            return self.err("an array or queue type given by a type parameter is not supported here");
        }
        Ok(match r.kind {
            NetKind::Real => Type::Real,
            NetKind::String => Type::Str,
            NetKind::Event => return self.err("an event type given by a type parameter is not supported here"),
            NetKind::Class => match r.class.as_deref().and_then(|n| self.find_class(n, sid)) {
                Some(c) => Type::Class(c),
                None => return self.err(format!("unknown class '{}' of a type parameter", r.class.as_deref().unwrap_or(""))),
            },
            _ => Type::vec(packed_of(r).width() as u32, r.signed),
        })
    }

    /// `f` with the types its arguments and result take from type parameters resolved in
    /// scope `sid` (kind, sign and ranges of the type, as declarations of a module); `f`
    /// itself where it uses none.
    pub(crate) fn subr_resolved(&mut self, f: &'a SubroutineDecl, sid: usize) -> R<&'a SubroutineDecl> {
        let ret_param = f.ret_ty.param.is_some();
        if !ret_param && !f.decls.iter().any(|d| d.type_param().is_some()) {
            return Ok(f);
        }
        let mut g = f.clone();
        if ret_param {
            let r = self.resolve_type(&f.ret_ty, sid)?;
            if r.kind == NetKind::Event {
                return self.err("a function returning an event is not supported");
            }
            g.ret_ty = Rc::new(r);
        }
        for d in &mut g.decls {
            let Some(tp) = d.type_param().map(str::to_string) else { continue };
            let Some(Sym::Type(t)) = self.lookup(&tp, sid) else {
                return self.err(format!("'{tp}' is not a type"));
            };
            // (with the declaration's own packed dimensions: `input T [3:0] v`)
            d.ty = if d.ty.dims.is_empty() { t } else { Rc::new(self.resolve_type(&d.ty, sid)?) };
        }
        Ok(Box::leak(Box::new(g)))
    }
}

/// A packed type with its layout, for struct members and assignment patterns: dimensions
/// evaluated, type parameters replaced.
#[derive(Clone, Debug)]
pub(crate) struct Packed {
    /// packed dimensions (left, right), outermost first
    pub(crate) dims: Vec<(i64, i64)>,
    pub(crate) signed: bool,
    /// `int`, `integer`, `byte`, ...: declared without a range (a pattern gives it one value)
    pub(crate) int_kind: bool,
    /// the element is a struct / union
    pub(crate) fields: Option<Rc<Fields>>,
    /// a real or string member of an unpacked struct (64 bits: f64 bits / a string handle)
    pub(crate) leaf: Leaf,
    /// an enum (or a packed array of them): its constants
    pub(crate) enum_names: Option<Rc<Vec<String>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leaf {
    Bits,
    Real,
    Str,
}

#[derive(Clone, Debug)]
pub(crate) struct Fields {
    pub(crate) union: bool,
    /// a tagged union: the tag (`tag_bits` wide) above the widest member
    pub(crate) tagged: bool,
    pub(crate) fields: Vec<(String, Packed)>,
}

impl Fields {
    /// The width of a tagged union's tag.
    pub(crate) fn tag_bits(&self) -> i64 {
        ferrooxide_front::lower::tag_bits(self.fields.len()) as i64
    }

    /// The width of the widest member (of a tagged union: where its tag starts).
    pub(crate) fn widest(&self) -> i64 {
        self.fields.iter().map(|x| x.1.width()).max().unwrap_or(0)
    }
}

fn dim_size(d: (i64, i64)) -> i64 {
    (d.0 - d.1).abs() + 1
}

impl Packed {
    /// Width of one element after the packed dimensions.
    fn leaf_width(&self) -> i64 {
        match &self.fields {
            None => 1,
            Some(f) if f.tagged => f.tag_bits() + f.widest(),
            Some(f) if f.union => f.fields.first().map_or(0, |x| x.1.width()),
            Some(f) => f.fields.iter().map(|x| x.1.width()).sum(),
        }
    }

    /// Width of the dimensions from `from` on, times the element width.
    fn width_from(&self, from: usize) -> i64 {
        self.dims[from..].iter().map(|&d| dim_size(d)).product::<i64>() * self.leaf_width()
    }

    pub(crate) fn width(&self) -> i64 {
        self.width_from(0)
    }
}

/// The layout of a type from `resolve_type` (its dimensions are numbers).
pub(crate) fn packed_of(t: &SvType) -> Packed {
    let dims = t.dims.iter().map(|(a, b)| (lit(a).unwrap_or(0), lit(b).unwrap_or(0))).collect();
    let fields = t.fields.as_ref().map(|f| {
        Rc::new(Fields { union: f.union, tagged: f.tagged, fields: f.fields.iter().map(|(n, ft)| (n.clone(), packed_of(ft))).collect() })
    });
    let leaf = match t.kind {
        NetKind::Real => Leaf::Real,
        NetKind::String => Leaf::Str,
        _ => Leaf::Bits,
    };
    Packed { dims, signed: t.signed, int_kind: t.int_kind, fields, leaf, enum_names: t.enum_names.clone() }
}

/// The bit offsets of the real and string members in a value of type `t` at `off`.
pub(crate) fn value_members(t: &Packed, off: i64, out: &mut Vec<i64>) {
    if t.leaf != Leaf::Bits {
        out.push(off);
        return;
    }
    let Some(f) = &t.fields else { return };
    let n: i64 = t.dims.iter().map(|&d| dim_size(d)).product();
    let el = t.leaf_width();
    for k in 0..n {
        let mut o = off + k * el;
        for (_, ft) in f.fields.iter().rev() {
            value_members(ft, o, out);
            if !f.union {
                o += ft.width();
            }
        }
    }
}

/// One select of a chain: `[i]`, `[m:l]`, `[b +: w]` / `[b -: w]`, `.m`.
#[derive(Clone, Copy)]
enum Sel<'e> {
    Index(&'e Expr),
    Range(&'e Expr, &'e Expr),
    Part(&'e Expr, &'e Expr, bool),
    Member(&'e str),
}

/// A chain of selects as its base and the selects, innermost first.
fn chain(e: &Expr) -> (&Expr, Vec<Sel<'_>>) {
    let mut ops = Vec::new();
    let mut cur = e;
    loop {
        match &cur.kind {
            ExprKind::Index(a, i) => {
                ops.push(Sel::Index(i));
                cur = a;
            }
            ExprKind::Range(a, m, l) => {
                ops.push(Sel::Range(m, l));
                cur = a;
            }
            ExprKind::IndexedRange(a, b, w, up) => {
                ops.push(Sel::Part(b, w, *up));
                cur = a;
            }
            ExprKind::Member(a, m) => {
                ops.push(Sel::Member(m));
                cur = a;
            }
            _ => break,
        }
    }
    ops.reverse();
    (cur, ops)
}

fn apply(e: Expr, op: Sel) -> Expr {
    let b = Box::new;
    match op {
        Sel::Index(i) => ExprKind::Index(b(e), b(i.clone())).into(),
        Sel::Range(m, l) => ExprKind::Range(b(e), b(m.clone()), b(l.clone())).into(),
        Sel::Part(x, w, up) => ExprKind::IndexedRange(b(e), b(x.clone()), b(w.clone()), up).into(),
        Sel::Member(m) => ExprKind::Member(b(e), m.to_string()).into(),
    }
}

fn num(v: i64) -> Expr {
    let n = ExprKind::Num(ferrooxide_front::ast::dec(v.unsigned_abs()));
    if v < 0 {
        ExprKind::Unary(UnOp::Neg, Box::new(n.into())).into()
    } else {
        n.into()
    }
}

fn lit(e: &Expr) -> Option<i64> {
    match &e.kind {
        ExprKind::Num(n) if !n.fill && !n.digits.contains(['x', 'z', 'X', 'Z', '?']) => {
            i64::from_str_radix(&n.digits.replace('_', ""), n.base).ok()
        }
        ExprKind::Unary(UnOp::Neg, a) => lit(a).map(|v| -v),
        _ => None,
    }
}

/// `a op b`, folded where both are plain numbers.
fn arith(op: B, a: Expr, b: Expr) -> Expr {
    match (op, lit(&a), lit(&b)) {
        (B::Add, Some(x), Some(y)) => num(x + y),
        (B::Sub, Some(x), Some(y)) => num(x - y),
        (B::Mul, Some(x), Some(y)) => num(x * y),
        (B::Add, Some(0), _) | (B::Mul, Some(1), _) => b,
        (B::Add | B::Sub, _, Some(0)) | (B::Mul, _, Some(1)) => a,
        _ => ExprKind::Binary(op, Box::new(a), Box::new(b)).into(),
    }
}

/// Position of index `k` in dimension `d`, counted from its right end.
fn dim_pos(d: (i64, i64), k: Expr) -> Expr {
    if d.0 >= d.1 {
        arith(B::Sub, k, num(d.1))
    } else {
        arith(B::Sub, num(d.1), k)
    }
}

/// A member chain's flat form without its tag checks (`$__tag_check`).
pub(crate) fn strip_tag_checks(mut e: Expr) -> Expr {
    if let ExprKind::SysCall(n, a) = &mut e.kind {
        if n == "$__tag_check" {
            return strip_tag_checks(a.swap_remove(1));
        }
    }
    e.for_each_child_mut(&mut |c| *c = strip_tag_checks(std::mem::replace(c, ExprKind::Null.into())));
    e
}

/// `$__cast(width, unsigned, v)`: `v` as a value of `w` bits.
fn cast(w: i64, v: Expr) -> Expr {
    ExprKind::SysCall("$__cast".into(), vec![num(w), num(0), v]).into()
}

/// A member chain of a signal with a struct type in its flat form.
pub(crate) struct Walk {
    /// the flat expression
    pub(crate) expr: Expr,
    /// the type selected, where the chain selects a whole member or element
    pub(crate) ty: Option<Packed>,
    /// a whole real or string member
    pub(crate) leaf: Leaf,
    /// an lvalue that sets a member of a tagged union
    pub(crate) tag_write: bool,
}

impl<'a> Compiler<'a> {
    /// The layout of a data type in scope `sid`.
    pub(crate) fn packed(&mut self, t: &SvType, sid: usize) -> R<Packed> {
        Ok(packed_of(&self.resolve_type(t, sid)?))
    }

    /// The layout of a declared type (none for real, string and event), evaluated in scope
    /// `sid` before the declared name is in it (`input [w-1:0] w` uses an outer `w`).
    pub(crate) fn layout_for(&mut self, t: Option<&Rc<SvType>>, sid: usize) -> R<Option<Packed>> {
        match t {
            Some(t) if !matches!(t.kind, NetKind::Real | NetKind::String | NetKind::Event | NetKind::Genvar) => Ok(Some(self.packed(t, sid)?)),
            _ => Ok(None),
        }
    }

    /// Give signal `s` (declared as `name` in scope `sid`) the layout of its declared type.
    pub(crate) fn set_layout(&mut self, s: u32, name: &str, p: Option<Packed>, sid: usize) -> R<()> {
        let Some(p) = p else { return Ok(()) };
        let id = self.packed_enum(&p, sid);
        if id != NONE {
            self.enums.sigs.insert(s, id);
        }
        if p.fields.is_some() {
            // member chains name the flat storage
            self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(s));
            self.sigs.has_structs = true;
            // real and string members start as 0.0 and the empty string (handle 0)
            let mut zero = Vec::new();
            value_members(&p, 0, &mut zero);
            if !zero.is_empty() {
                self.sigs.has_struct_values = true;
                self.intern(b"");
                let info = self.sig(s);
                let (slot, n) = (info.slot as usize, rt::nw(info.width) as usize);
                for e in 0..info.depth.max(1) as usize {
                    let base = slot + 2 * n * e;
                    for &off in &zero {
                        for b in off..off + 64 {
                            let (w, m) = (b / 64, 1u64 << (b % 64));
                            self.prog.mem[base + w as usize] &= !m;
                            self.prog.mem[base + n + w as usize] &= !m;
                        }
                    }
                }
            }
        }
        self.sigs.layout.insert(s, Rc::new(p));
        Ok(())
    }

    /// The layout of signal `s`, if its declaration has a type.
    pub(crate) fn layout(&self, s: u32) -> Option<Rc<Packed>> {
        self.sigs.layout.get(&s).cloned()
    }

    /// A member chain of a signal with a struct type (`v[i].f.g[3:0]`) as one part select of
    /// its flat storage (`$flat:v[i][off +: w]`, of a queue or dynamic array element
    /// `q[i][off +: w]`; offsets are expressions, so run-time indexes work); a whole signed member read is `$signed` unless `lvalue`. None where `e`
    /// is no such chain, an error message where it is a wrong one.
    pub(crate) fn struct_walk(&self, e: &Expr, sid: usize, lvalue: bool) -> Option<Result<Walk, String>> {
        let (base, ops) = chain(e);
        // a struct property of an object (`obj.p.f`): selects of the property's value
        if let Some((layout, elem, k)) = self.prop_root(base, &ops, sid) {
            return self.walk_members(layout, elem, &ops, k, lvalue);
        }
        // `base` with its last name prefixed (`$flat:v`, `$type:p`)
        let renamed = |pre: &str| -> Expr {
            match &base.kind {
                ExprKind::Ident(n) => ExprKind::Ident(format!("{pre}{n}")).into(),
                ExprKind::Hier(p) => {
                    let mut p = p.clone();
                    let l = p.last_mut().unwrap();
                    *l = format!("{pre}{l}");
                    ExprKind::Hier(p).into()
                }
                _ => unreachable!(),
            }
        };
        let sym = match &base.kind {
            ExprKind::Ident(n) if !n.starts_with("$flat:") => self.lookup(n, sid)?,
            ExprKind::Hier(p) if !p.last()?.starts_with("$flat:") => self.lookup_hier(p, sid)?,
            _ => return None,
        };
        let is_param = matches!(sym, Sym::Const(_));
        // (a queue or dynamic array of structs: `q[i]` is the struct)
        let dyn_elem = matches!(sym, Sym::Sig(s) if self.sigs.elem_layout.contains_key(&s));
        let (layout, nun) = match sym {
            Sym::Sig(s) if dyn_elem => ((*self.sigs.elem_layout[&s]).clone(), 1),
            Sym::Sig(s) => {
                let info = self.sig(s);
                // (an array: the unpacked dimensions come first)
                let nun = if info.depth > 0 { self.sigs.md_dims.get(&s).map_or(1, |d| d.len()) } else { 0 };
                ((**self.sigs.layout.get(&s)?).clone(), nun)
            }
            // a parameter with a struct type
            Sym::Const(_) => {
                let t = match renamed("$type:").kind {
                    ExprKind::Ident(n) => self.lookup(&n, sid)?,
                    ExprKind::Hier(p) => self.lookup_hier(&p, sid)?,
                    _ => unreachable!(),
                };
                let Sym::Type(t) = t else { return None };
                (packed_of(&t), 0)
            }
            _ => return None,
        };
        // (a parameter with several packed dimensions has no other path for its selects)
        if layout.fields.is_none() && !(is_param && layout.dims.len() > 1) {
            return None;
        }
        // (a port joined with the signal connected to it has that signal's layout: only a name
        // declared with a struct type has members)
        if matches!(sym, Sym::Sig(_)) && !dyn_elem {
            let declared = match renamed("$flat:").kind {
                ExprKind::Ident(n) => self.lookup(&n, sid),
                ExprKind::Hier(p) => self.lookup_hier(&p, sid),
                _ => unreachable!(),
            };
            declared?;
        }
        let flat_base = if dyn_elem { base.clone() } else { renamed("$flat:") };
        let mut k = 0;
        let mut elem = base.clone();
        while k < nun {
            match ops.get(k) {
                Some(&op @ Sel::Index(_)) => elem = apply(elem, op),
                Some(Sel::Range(..) | Sel::Part(..)) => return Some(Err("a slice of an unpacked struct array is not supported".into())),
                _ => return None,
            }
            k += 1;
        }
        let elem = match nun {
            0 => flat_base,
            1 => match elem.kind {
                ExprKind::Index(_, i) => ExprKind::Index(Box::new(flat_base), i).into(),
                _ => unreachable!(),
            },
            _ => self.md_norm_elem(&elem, sid)?,
        };
        self.walk_members(layout, elem, &ops, k, lvalue)
    }

    /// An assignment target that is a part of a struct property (`obj.p.f`, `obj.p.f[3:0]`):
    /// the property (`obj.p`), the bit offset and width of the part in it, the walk; None
    /// for other targets.
    pub(crate) fn prop_part(&self, e: &Expr, sid: usize) -> R<Option<(Expr, i64, i64, Walk)>> {
        let (base, ops) = chain(e);
        // (a member of an element of an array property: the array's element store)
        if self.prop_root(base, &ops, sid).is_none_or(|r| matches!(r.1.kind, ExprKind::Index(..))) {
            return Ok(None);
        }
        let w = match self.struct_walk(e, sid, true) {
            None => return Ok(None),
            Some(Err(m)) => return self.err(m),
            Some(Ok(w)) => w,
        };
        if w.tag_write {
            return self.err("an assignment to a member of a tagged union property is not supported");
        }
        let ExprKind::IndexedRange(p, off, width, true) = &w.expr.kind else {
            return self.err("this assignment to a part of a struct property is not supported");
        };
        match (lit(off), lit(width)) {
            (Some(off), Some(width)) => Ok(Some(((**p).clone(), off, width, w))),
            _ => self.err("an assignment to a part of a struct property at a run-time index is not supported"),
        }
    }

    /// The layout of `e`, a struct element of an array property (`obj.q[i]`).
    pub(crate) fn prop_elem_layout(&self, e: &Expr, sid: usize) -> Option<Packed> {
        let (base, ops) = chain(e);
        self.prop_root(base, &ops, sid).filter(|r| matches!(r.1.kind, ExprKind::Index(..)) && r.2 == ops.len()).map(|r| r.0)
    }

    /// `e` is a member (or a part of one) of a struct element of an array property
    /// (`obj.q[i].f`).
    pub(crate) fn prop_elem_member(&self, e: &Expr, sid: usize) -> bool {
        let (base, ops) = chain(e);
        self.prop_root(base, &ops, sid).is_some_and(|r| matches!(r.1.kind, ExprKind::Index(..)) && r.2 < ops.len())
    }

    /// The struct property a select chain (`base` with `ops`) starts with (`obj.p`, also
    /// through handle properties `a.b.p`): its layout, the property as an expression and
    /// the number of selects it takes.
    fn prop_root(&self, base: &Expr, ops: &[Sel], sid: usize) -> Option<(Packed, Expr, usize)> {
        if self.classes.list.is_empty() {
            return None;
        }
        let sym = match &base.kind {
            ExprKind::Ident(n) => self.lookup(n, sid)?,
            ExprKind::Hier(p) => self.lookup_hier(p, sid)?,
            _ => return None,
        };
        let Sym::Sig(s) = sym else { return None };
        let mut cur = base.clone();
        // (an element of a queue or array of handles: `q[i].p.f`)
        let (mut c, mut k) = match self.sig_type(s) {
            Type::Class(c) => (c, 0),
            Type::Dyn { elem, .. } if elem.class != NONE && elem.nested == NONE => {
                let Some(&ix @ Sel::Index(_)) = ops.first() else { return None };
                cur = apply(cur, ix);
                (elem.class, 1)
            }
            _ => return None,
        };
        while k < ops.len() {
            let op = ops[k];
            let Sel::Member(m) = op else { return None };
            let p = self.classes.list.get(c as usize)?.props.iter().rev().find(|p| p.name == m)?;
            if !p.dims.is_empty() {
                return None;
            }
            cur = apply(cur, op);
            match (&p.layout, p.ty) {
                // an element of an array of structs (`obj.q[i].f`, `obj.m[a][b].f`)
                (Some(l), Type::Dyn { elem, .. }) => {
                    let mut depth = 1;
                    let mut e = elem;
                    while let Some((_, inner)) = self.inner_arr(e) {
                        depth += 1;
                        e = inner;
                    }
                    for j in 0..depth {
                        let Some(&ix @ Sel::Index(_)) = ops.get(k + 1 + j) else { return None };
                        cur = apply(cur, ix);
                    }
                    return Some(((**l).clone(), cur, k + 1 + depth));
                }
                (Some(l), _) => return Some(((**l).clone(), cur, k + 1)),
                (None, Type::Class(x)) if x != NONE => c = x,
                // (a queue of handles: `obj.q[i].p.f`)
                (None, Type::Dyn { elem, .. }) if elem.class != NONE && elem.nested == NONE => {
                    let Some(&ix @ Sel::Index(_)) = ops.get(k + 1) else { return None };
                    cur = apply(cur, ix);
                    c = elem.class;
                    k += 1;
                }
                _ => return None,
            }
            k += 1;
        }
        None
    }

    /// The selects `ops[k..]` of a value of type `layout` (`elem`) as one part select of it.
    fn walk_members(&self, layout: Packed, elem: Expr, ops: &[Sel], mut k: usize, lvalue: bool) -> Option<Result<Walk, String>> {
        let mut cur: Packed = layout;
        let mut di = 0;
        let mut off = num(0);
        let mut width = num(cur.width());
        let mut touched = false;
        let mut whole = true;
        // (tagged union members read: their tag, the member's position, its name)
        let mut tag_checks: Vec<(Expr, usize, String)> = Vec::new();
        let mut tag_write = false;
        loop {
            match ops.get(k) {
                // (a select of a struct member that is a packed struct: its bits `[w-1:0]`)
                Some(&op @ (Sel::Index(_) | Sel::Range(..) | Sel::Part(..))) if di < cur.dims.len() || (touched && di == cur.dims.len() && cur.fields.as_ref().is_some_and(|f| !f.fields.is_empty())) => {
                    if di == cur.dims.len() {
                        cur = Packed { dims: vec![(cur.width() - 1, 0)], signed: false, int_kind: false, fields: None, leaf: Leaf::Bits, enum_names: None };
                        di = 0;
                    }
                    let r = cur.dims[di];
                    let el = cur.width_from(di + 1);
                    k += 1;
                    touched = true;
                    match op {
                        Sel::Index(a) => {
                            off = arith(B::Add, off, arith(B::Mul, dim_pos(r, a.clone()), num(el)));
                            width = num(el);
                            di += 1;
                            continue;
                        }
                        Sel::Range(a, b) => {
                            off = arith(B::Add, off, arith(B::Mul, dim_pos(r, b.clone()), num(el)));
                            let n = match (lit(a), lit(b)) {
                                (Some(a), Some(b)) => num(dim_size((a, b))),
                                _ => {
                                    let ge = ExprKind::Binary(B::Ge, Box::new(a.clone()), Box::new(b.clone()));
                                    let d = ExprKind::Cond(
                                        Box::new(ge.into()),
                                        Box::new(arith(B::Sub, a.clone(), b.clone())),
                                        Box::new(arith(B::Sub, b.clone(), a.clone())),
                                    );
                                    arith(B::Add, d.into(), num(1))
                                }
                            };
                            width = arith(B::Mul, n, num(el));
                        }
                        Sel::Part(a, n, up) => {
                            let (a, n) = (a.clone(), n.clone());
                            let low = match (up, r.0 >= r.1) {
                                (true, true) | (false, false) => a,
                                (true, false) => arith(B::Sub, arith(B::Add, a, n.clone()), num(1)),
                                (false, true) => arith(B::Add, arith(B::Sub, a, n.clone()), num(1)),
                            };
                            off = arith(B::Add, off, arith(B::Mul, dim_pos(r, low), num(el)));
                            width = arith(B::Mul, n, num(el));
                        }
                        Sel::Member(_) => unreachable!(),
                    }
                    whole = false;
                    break;
                }
                Some(Sel::Member(m)) if di == cur.dims.len() && cur.fields.is_some() => {
                    let st = cur.fields.clone().unwrap();
                    // (internal) `$tag`: the tag of a tagged union
                    if *m == "$tag" && st.tagged {
                        k += 1;
                        off = arith(B::Add, off, num(st.widest()));
                        let w = st.tag_bits();
                        cur = Packed { dims: vec![(w - 1, 0)], signed: false, int_kind: false, fields: None, leaf: Leaf::Bits, enum_names: None };
                        di = 0;
                        width = num(w);
                        touched = true;
                        continue;
                    }
                    let Some(fk) = st.fields.iter().position(|f| f.0 == *m) else {
                        return Some(Err(format!("no member '{m}' in the struct")));
                    };
                    k += 1;
                    if st.tagged {
                        if st.fields[fk].1.fields.as_ref().is_some_and(|f| f.fields.is_empty() && !f.union) {
                            return Some(Err(format!("'{m}' is a void member of a tagged union: it has no value")));
                        }
                        // a member of a tagged union is read only while the tag names it
                        // (IEEE 1800-2017 7.3.2); it is set only by a tagged union expression
                        if lvalue {
                            tag_write = true;
                        } else if st.tag_bits() > 0 {
                            let tag: Expr = ExprKind::IndexedRange(Box::new(elem.clone()), Box::new(arith(B::Add, off.clone(), num(st.widest()))), Box::new(num(st.tag_bits())), true).into();
                            tag_checks.push((tag, fk, m.to_string()));
                        }
                    }
                    if !st.union {
                        let after: i64 = st.fields[fk + 1..].iter().map(|f| f.1.width()).sum();
                        off = arith(B::Add, off, num(after));
                    }
                    cur = st.fields[fk].1.clone();
                    di = 0;
                    width = num(cur.width());
                    touched = true;
                    continue;
                }
                _ => break,
            }
        }
        let ty = whole.then(|| {
            let mut t = cur.clone();
            t.dims.drain(..di);
            if di > 0 {
                t.int_kind = false;
            }
            t
        });
        if !touched {
            return None;
        }
        let mut out: Expr = ExprKind::IndexedRange(Box::new(elem), Box::new(off), Box::new(width), true).into();
        let leaf = if whole && di == 0 { cur.leaf } else { Leaf::Bits };
        if !lvalue && whole && di == 0 {
            match cur.leaf {
                // a real member read: its bits as a real
                Leaf::Real => out = ExprKind::SysCall("$bitstoreal".into(), vec![out]).into(),
                // (a string member: see `member_type`)
                Leaf::Str => {}
                // a whole signed member reads as signed
                Leaf::Bits if cur.signed => out = ExprKind::SysCall("$signed".into(), vec![out]).into(),
                Leaf::Bits => {}
            }
        }
        for &op in &ops[k..] {
            out = apply(out, op);
        }
        // a member read while the tag names another member: a run-time error (an unknown tag
        // is not checked)
        for (tag, fk, m) in tag_checks.into_iter().rev().filter(|_| leaf == Leaf::Bits) {
            let other = ExprKind::Binary(B::CaseEq, Box::new(ExprKind::Binary(B::Ne, Box::new(tag), Box::new(num(fk as i64))).into()), Box::new(num(1))).into();
            out = ExprKind::SysCall("$__tag_check".into(), vec![other, out, ExprKind::Str(m).into()]).into();
        }
        let whole_chain = k == ops.len();
        Some(Ok(Walk { expr: out, ty: if whole_chain { ty } else { None }, leaf: if whole_chain { leaf } else { Leaf::Bits }, tag_write }))
    }

    /// The type of a member chain that selects a whole real or string member of an unpacked
    /// struct.
    pub(crate) fn member_type(&self, e: &Expr, sid: usize) -> Option<Type> {
        if !self.sigs.has_struct_values || !matches!(&e.kind, ExprKind::Member(..) | ExprKind::Index(..)) {
            return None;
        }
        match self.struct_walk(e, sid, true)?.ok()?.leaf {
            Leaf::Real => Some(Type::Real),
            Leaf::Str => Some(Type::Str),
            Leaf::Bits => None,
        }
    }

    /// A string member of a struct as the slot of its handle.
    pub(crate) fn str_member(&mut self, e: &Expr, sid: usize) -> R<Option<u32>> {
        if self.member_type(e, sid) != Some(Type::Str) {
            return Ok(None);
        }
        let n = self.lv_norm(e, sid).unwrap();
        Ok(Some(self.expr(&n, sid, 64, false)?))
    }

    /// The error for a member select that names no member.
    pub(crate) fn member_error<T>(&self, e: &Expr, sid: usize) -> R<T> {
        match self.struct_walk(e, sid, false) {
            Some(Err(m)) => self.err(m),
            _ => {
                let ExprKind::Member(_, m) = &e.kind else { unreachable!() };
                self.err(format!("'.{m}' selects a member of a value without members"))
            }
        }
    }

    /// The type an assignment to `e` gives a pattern on its right.
    pub(crate) fn target_layout(&self, e: &Expr, sid: usize) -> Option<Packed> {
        let (base, ops) = chain(e);
        let sym = match &base.kind {
            ExprKind::Ident(n) => self.lookup(n, sid)?,
            ExprKind::Hier(p) => self.lookup_hier(p, sid)?,
            _ => return None,
        };
        let Sym::Sig(s) = sym else { return None };
        // the whole variable, or a whole element of an array
        let info = self.sig(s);
        let nun = if info.depth > 0 { self.sigs.md_dims.get(&s).map_or(1, |d| d.len()) } else { 0 };
        if ops.len() == nun && ops.iter().all(|o| matches!(o, Sel::Index(_))) {
            return self.layout(s).map(|l| (*l).clone());
        }
        if let Some(Ok(w)) = self.struct_walk(e, sid, true) {
            return w.ty;
        }
        // an element of a packed array (`v[i]` of `logic [3:0][7:0] v`)
        if ops.len() > nun && ops.iter().all(|o| matches!(o, Sel::Index(_))) {
            let mut l = (*self.layout(s)?).clone();
            let k = ops.len() - nun;
            if l.dims.len() <= k {
                return None;
            }
            l.dims.drain(..k);
            l.int_kind = false;
            return Some(l);
        }
        // a part select of a vector: a vector of its width (`'{default: b}`)
        let w = match ops.last()? {
            Sel::Range(m, l) => (self.const_value(m, sid)? - self.const_value(l, sid)?).abs() + 1,
            Sel::Part(_, w, _) => self.const_value(w, sid)?,
            _ => return None,
        };
        Some(Packed { dims: vec![(w - 1, 0)], signed: false, int_kind: false, fields: None, leaf: Leaf::Bits, enum_names: None })
    }

    /// `e` with the assignment patterns at its top (or in a conditional's branches) resolved
    /// against `t` (a typed pattern against its own type); None where there are none.
    pub(crate) fn with_target(&mut self, e: &Expr, t: Option<&Packed>, sid: usize) -> R<Option<Expr>> {
        match &e.kind {
            ExprKind::Pattern(Some(pt), items) => {
                let pt = self.packed(pt, sid)?;
                Ok(Some(self.build_pattern(items, &pt, sid)?))
            }
            ExprKind::Pattern(None, items) => match t {
                Some(t) => Ok(Some(self.build_pattern(items, t, sid)?)),
                None => Ok(None),
            },
            ExprKind::Tagged(m, v) => match t {
                Some(t) if t.dims.is_empty() && t.fields.as_ref().is_some_and(|f| f.tagged) => Ok(Some(self.build_tagged(m, v.as_deref(), t, sid)?)),
                Some(_) => self.err(format!("`tagged {m}` is assigned to a value that is not a tagged union")),
                None => Ok(None),
            },
            ExprKind::Cond(c, a, b) => {
                let (ra, rb) = (self.with_target(a, t, sid)?, self.with_target(b, t, sid)?);
                if ra.is_none() && rb.is_none() {
                    return Ok(None);
                }
                let a = ra.unwrap_or_else(|| (**a).clone());
                let b = rb.unwrap_or_else(|| (**b).clone());
                Ok(Some(ExprKind::Cond(c.clone(), Box::new(a), Box::new(b)).into()))
            }
            _ => Ok(None),
        }
    }

    /// An assignment pattern for type `t` as a concatenation of its elements or members,
    /// each cast to its width (IEEE 1800 10.9).
    fn build_pattern(&mut self, items: &[PatItem], t: &Packed, sid: usize) -> R<Expr> {
        let items = &self.expand_pattern(items, sid)?[..];
        let mut pos = Vec::new();
        let mut named = Vec::new();
        let mut typed: Vec<(Packed, &Expr)> = Vec::new();
        let mut default = None;
        // index keys of an array: (element position, value)
        let mut indexed: Vec<(usize, &Expr)> = Vec::new();
        let array = !t.dims.is_empty() && !t.int_kind;
        for it in items {
            match it {
                PatItem::Pos(v) => pos.push(v),
                // (a name as the key of an array: a parameter giving the index)
                PatItem::Key(k, v) if array => indexed.push((self.pattern_index(&ExprKind::Ident(k.clone()).into(), t.dims[0], sid)?, v)),
                PatItem::Key(k, v) => named.push((k, v)),
                PatItem::Default(v) => default = Some(v),
                PatItem::Index(i, v) if array => indexed.push((self.pattern_index(i, t.dims[0], sid)?, v)),
                PatItem::Index(..) => return self.err("an index key in an assignment pattern of a struct"),
                PatItem::Type(kt, v) => {
                    let p = self.packed(kt, sid)?;
                    typed.push((p, v));
                }
                PatItem::Repl(..) => unreachable!("expanded"),
            }
        }
        let parts: Vec<(Option<&str>, Packed)> = if array {
            let mut el = t.clone();
            el.dims.remove(0);
            (0..dim_size(t.dims[0])).map(|_| (None, el.clone())).collect()
        } else if let Some(s) = &t.fields {
            if s.union {
                return self.err("assignment patterns for unions are not supported");
            }
            s.fields.iter().map(|(n, ft)| (Some(n.as_str()), ft.clone())).collect()
        } else {
            // (IEEE 1800-2017 10.9: for arrays and structs)
            if t.dims.is_empty() && !t.int_kind {
                return self.err("an assignment pattern for a scalar type");
            }
            return Ok(match default {
                Some(v) => cast(t.width(), v.clone()),
                None => ExprKind::Concat(pos.into_iter().map(|v| cast(1, v.clone())).collect()).into(),
            });
        };
        if !pos.is_empty() && pos.len() != parts.len() {
            return self.err(format!("assignment pattern has {} values for {} elements", pos.len(), parts.len()));
        }
        let mut out = Vec::new();
        for (k, (name, pt)) in parts.iter().enumerate() {
            let v = if let Some(v) = pos.get(k) {
                *v
            } else if let Some((_, v)) = indexed.iter().rev().find(|(i, _)| *i == k) {
                *v
            } else if let Some((_, v)) = named.iter().find(|(n, _)| Some(n.as_str()) == *name) {
                *v
            } else if let Some((_, v)) = typed.iter().rev().find(|(kt, _)| same_packed(kt, pt)) {
                // (a type key: the members and elements of that type, 10.9.2)
                *v
            } else if let Some(v) = default {
                v
            } else {
                return self.err(match name {
                    Some(n) => format!("assignment pattern has no value for member {n}"),
                    None => format!("assignment pattern has no value for element {k} (from the left)"),
                });
            };
            let v = self.with_target(v, Some(pt), sid)?.unwrap_or_else(|| v.clone());
            out.push(match (pt.leaf, v) {
                (Leaf::Real, v) => ExprKind::SysCall("$realtobits".into(), vec![v]).into(),
                (Leaf::Str, v) => ExprKind::SysCall("$__strbits".into(), vec![v]).into(),
                (_, v @ Expr { kind: ExprKind::Concat(_), .. }) => v,
                (_, v) => cast(pt.width(), v),
            });
        }
        Ok(ExprKind::Concat(out).into())
    }

    /// `tagged m v` for tagged union type `t` (IEEE 1800-2017 11.9): the tag, zeros, the member
    /// value right-justified.
    fn build_tagged(&mut self, m: &str, v: Option<&Expr>, t: &Packed, sid: usize) -> R<Expr> {
        let f = t.fields.as_ref().unwrap();
        let Some(k) = f.fields.iter().position(|x| x.0 == m) else {
            return self.err(format!("the tagged union has no member '{m}'"));
        };
        let mt = &f.fields[k].1;
        let void = mt.fields.as_ref().is_some_and(|x| x.fields.is_empty() && !x.union);
        let mut out = Vec::new();
        if f.tag_bits() > 0 {
            out.push(cast(f.tag_bits(), num(k as i64)));
        }
        let pad = f.widest() - mt.width();
        if pad > 0 {
            out.push(cast(pad, num(0)));
        }
        match (v, void) {
            (Some(_), true) => return self.err(format!("`tagged {m}` of a void member takes no value")),
            (None, false) => return self.err(format!("`tagged {m}` needs a value for the member")),
            (None, true) => {}
            (Some(v), false) => {
                let v = self.with_target(v, Some(mt), sid)?.unwrap_or_else(|| v.clone());
                out.push(match (mt.leaf, v) {
                    (Leaf::Real, v) => ExprKind::SysCall("$realtobits".into(), vec![v]).into(),
                    (Leaf::Str, v) => ExprKind::SysCall("$__strbits".into(), vec![v]).into(),
                    (_, v) => cast(mt.width(), v),
                });
            }
        }
        if out.is_empty() {
            return self.err("a tagged union without bits");
        }
        Ok(ExprKind::Concat(out).into())
    }

    /// `TagIs(e, m)`: the test that the tag of tagged union `e` is member `m`.
    pub(crate) fn tag_is(&self, e: &Expr, m: &str, sid: usize) -> R<Expr> {
        let Some(t) = self.target_layout(e, sid) else {
            return self.err("a `tagged` pattern matches a value that is not a tagged union");
        };
        let Some(f) = t.fields.as_ref().filter(|f| f.tagged && t.dims.is_empty()) else {
            return self.err("a `tagged` pattern matches a value that is not a tagged union");
        };
        let Some(k) = f.fields.iter().position(|x| x.0 == m) else {
            return self.err(format!("the tagged union has no member '{m}'"));
        };
        if f.tag_bits() == 0 {
            return Ok(num(1));
        }
        // (the members on the way were tested before)
        let tag = ExprKind::Unchecked(Box::new(ExprKind::Member(Box::new(e.clone()), "$tag".into()).into())).into();
        Ok(ExprKind::Binary(B::CaseEq, Box::new(tag), Box::new(cast(f.tag_bits(), num(k as i64)))).into())
    }

    /// The items of a pattern with replications (`n{a, b}`, 10.10.1) written out as
    /// positional items.
    pub(crate) fn expand_pattern(&mut self, items: &[PatItem], sid: usize) -> R<Vec<PatItem>> {
        let mut out = Vec::new();
        for it in items {
            match it {
                PatItem::Repl(n, v) => {
                    let k = self.const_int(n, sid)?;
                    if k < 0 {
                        return self.err(format!("replication count {k} in an assignment pattern is negative"));
                    }
                    for _ in 0..k {
                        out.extend(v.iter().cloned().map(PatItem::Pos));
                    }
                }
                x => out.push(x.clone()),
            }
        }
        Ok(out)
    }

    /// The position (from the left) of the element that index key `i` names in a packed
    /// dimension `d` (msb, lsb).
    fn pattern_index(&mut self, i: &Expr, d: (i64, i64), sid: usize) -> R<usize> {
        let v = self.const_int(i, sid)?;
        if v < d.0.min(d.1) || v > d.0.max(d.1) {
            return self.err(format!("index {v} of an assignment pattern is outside [{}:{}]", d.0, d.1));
        }
        Ok((v - d.0).unsigned_abs() as usize)
    }

    /// A pattern in a place without a target type: one with its own type.
    pub(crate) fn typed_pattern(&mut self, e: &Expr, sid: usize) -> R<Expr> {
        match self.with_target(e, None, sid)? {
            Some(r) => Ok(r),
            None => self.err("an assignment pattern needs a known target type here"),
        }
    }
}

/// Two layouts are the same type (for type keys of assignment patterns): shape, sign, kind,
/// and the same struct or enum.
pub(crate) fn same_packed(a: &Packed, b: &Packed) -> bool {
    let ws = |p: &Packed| p.dims.iter().map(|&(l, r)| (l - r).abs()).collect::<Vec<_>>();
    ws(a) == ws(b)
        && a.signed == b.signed
        && a.int_kind == b.int_kind
        && a.leaf == b.leaf
        && match (&a.fields, &b.fields) {
            (None, None) => true,
            (Some(x), Some(y)) => Rc::ptr_eq(x, y) || (x.union == y.union && x.fields.len() == y.fields.len() && x.fields.iter().zip(&y.fields).all(|(f, g)| f.0 == g.0 && same_packed(&f.1, &g.1))),
            _ => false,
        }
        && match (&a.enum_names, &b.enum_names) {
            (None, None) => true,
            (Some(x), Some(y)) => Rc::ptr_eq(x, y),
            _ => false,
        }
}

impl<'a> Compiler<'a> {
    /// `type(a) == type(b)` & co. (IEEE 1800-2017 6.23): whether the types match (6.22.1);
    /// None if `e` is no comparison of two types.
    pub(crate) fn type_cmp(&mut self, e: &Expr, sid: usize) -> R<Option<bool>> {
        let ExprKind::Binary(op, a, b) = &e.kind else { return Ok(None) };
        let (ExprKind::Type(x), ExprKind::Type(y)) = (&a.kind, &b.kind) else { return Ok(None) };
        let eq = match op {
            BinOp::Eq | BinOp::CaseEq => true,
            BinOp::Ne | BinOp::CaseNe => false,
            _ => return self.err("types can only be compared with ==, !=, === and !=="),
        };
        Ok(Some(self.types_match(x, sid, y, sid)? == eq))
    }

    /// Whether two data types match (IEEE 1800-2017 6.22.1): the same built-in type, packed
    /// and unpacked dimensions and signing, or the same enum, struct or class.
    /// (`x` in scope `sx`, `y` in scope `sy`)
    pub(crate) fn types_match(&mut self, x: &SvType, sx: usize, y: &SvType, sy: usize) -> R<bool> {
        // (a type parameter: its type; enums and structs are told apart by their declaration)
        let base = |c: &Self, t: &SvType, sid: usize| -> Option<Rc<SvType>> {
            match t.param.as_ref().map(|p| c.lookup(p, sid)) {
                Some(Some(Sym::Type(b))) => Some(b),
                _ => None,
            }
        };
        let (bx, by) = (base(self, x, sx), base(self, y, sy));
        let (ix, iy) = (bx.as_deref().unwrap_or(x), by.as_deref().unwrap_or(y));
        let same_rc = |a: &Option<Rc<Vec<String>>>, b: &Option<Rc<Vec<String>>>| match (a, b) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let same_struct = match (&ix.fields, &iy.fields) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same_rc(&ix.enum_names, &iy.enum_names) || !same_struct || ix.class != iy.class {
            return Ok(false);
        }
        let (rx, ry) = (self.resolve_type(x, sx)?, self.resolve_type(y, sy)?);
        let norm = |k: NetKind| if k == NetKind::Wire { NetKind::Reg } else { k };
        if rx.heap.is_some() || ry.heap.is_some() {
            return self.err("matching of queue, dynamic or associative array types is not supported");
        }
        if norm(rx.kind) != norm(ry.kind) || rx.signed != ry.signed || rx.int_kind != ry.int_kind {
            return Ok(false);
        }
        let dims = |c: &mut Self, t: &SvType, sid: usize| -> R<Vec<(i64, i64)>> {
            t.dims.iter().chain(&t.unpacked).map(|(a, b)| Ok((c.const_int(a, sid)?, c.const_int(b, sid)?))).collect()
        };
        Ok(dims(self, &rx, sx)? == dims(self, &ry, sy)? && rx.dims.len() == ry.dims.len())
    }
}

impl<'a> Compiler<'a> {
    /// `$typename(T)` (IEEE 1800-2017 20.6.1) as its string literal: the resolved type, its
    /// signing where it is not the default, ranges as decimal numbers, `$` for an unpacked
    /// array; one space between keywords. None for any other expression.
    pub(crate) fn typename_str(&mut self, e: &Expr, sid: usize) -> R<Option<Expr>> {
        let ExprKind::SysCall(n, args) = &e.kind else { return Ok(None) };
        let (true, [Expr { kind: ExprKind::Type(t), .. }]) = (n == "$typename", args.as_slice()) else { return Ok(None) };
        let r = self.resolve_type(t, sid)?;
        if r.enum_names.is_some() || r.fields.is_some() || r.class.is_some() || r.heap.is_some() || r.inner_heap.is_some() {
            return self.err("$typename of an enum, struct, union, class or dynamic type is not supported");
        }
        let (name, dflt_signed) = match r.kind {
            NetKind::Int(32) => ("int", true),
            NetKind::Int(8) => ("byte", true),
            NetKind::Int(16) => ("shortint", true),
            NetKind::Int(64) => ("longint", true),
            NetKind::Integer => ("integer", true),
            NetKind::Time => ("time", false),
            NetKind::Reg | NetKind::Wire => ("logic", false),
            NetKind::Bit => ("bit", false),
            NetKind::Real => ("real", true),
            NetKind::String => ("string", false),
            NetKind::Event => ("event", false),
            _ => return self.err("$typename of this type is not supported"),
        };
        let mut s = name.to_string();
        let atom = matches!(r.kind, NetKind::Real | NetKind::String | NetKind::Event);
        if !atom && r.signed != dflt_signed {
            s.push_str(if r.signed { " signed" } else { " unsigned" });
        }
        let range = |c: &mut Self, (a, b): &(Expr, Expr)| -> R<String> { Ok(format!("[{}:{}]", c.const_int(a, sid)?, c.const_int(b, sid)?)) };
        if !(r.int_kind || atom) {
            for d in &r.dims {
                s.push_str(&range(self, d)?);
            }
        }
        if !r.unpacked.is_empty() {
            s.push('$');
            for d in &r.unpacked {
                s.push_str(&range(self, d)?);
            }
        }
        Ok(Some(Expr::new(ExprKind::Str(s), e.line)))
    }
}
