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
            Type::Real | Type::Str | Type::Dyn { .. } | Type::Class(_) => 64,
        }
    }

    /// Sign where the value is used as a vector (a real converts to a signed integer).
    pub(crate) fn signed(self) -> bool {
        match self {
            Type::Vec { signed, .. } => signed,
            Type::Real => true,
            Type::Str | Type::Dyn { .. } | Type::Class(_) => false,
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
    "$realtime", "$itor", "$bitstoreal", "$sqrt", "$ln", "$log10", "$exp", "$floor", "$ceil", "$sin", "$cos", "$tan",
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
            ExprKind::Randomize { .. } => Type::vec(32, true),
            ExprKind::ArrayWith { method, .. } => return self.err(format!("array method '{method}' with `with` is not supported for this array (only for queues, dynamic arrays and one-dimensional fixed-size arrays of vectors, reals, strings or class handles)")),
            ExprKind::Member(..) => return self.member_error(e, sid),
            ExprKind::NewArray(..) => return self.err("new[] needs a dynamic array to assign to"),
            ExprKind::Null | ExprKind::New(_) | ExprKind::NewTyped(..) | ExprKind::NewCopy(_) | ExprKind::MethodCall(..) | ExprKind::StaticCall(..) => {
                return self.err("no classes are declared")
            }
            ExprKind::EmptyArg => return self.err("an empty argument is allowed only in a call of a function or task"),
            ExprKind::NamedArg(..) => return self.err("a named argument is allowed only in a call of a function or task"),
            ExprKind::Last => return self.err("`$` outside an index of a queue"),
            ExprKind::IncDec { e, .. } => return self.type_of(e, sid),
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
                let Some((f, fs)) = self.lookup_func(name, sid) else {
                    if let Some((obj, m)) = self.hier_handle(name, sid) {
                        return self.type_of(&Expr::from(ExprKind::MethodCall(Box::new(obj), m, args.clone())), sid);
                    }
                    return self.err(format!("unknown function '{name}'"));
                };
                if f.is_task {
                    return self.err(format!("task '{name}' used as a function"));
                }
                self.func_ret_type(f, fs)?
            }
            ExprKind::SysCall(name, args) => match name.as_str() {
                "$sformatf" | "$psprintf" | "$__tostring" => Type::Str,
                n if REAL_FUNCS.contains(&n) => Type::Real,
                "$time" => v(64, false),
                "$stime" => v(32, false),
                "$random" => v(32, true),
                "$urandom" | "$urandom_range" => v(32, false),
                "$signed" | "$unsigned" => {
                    let a = args.first().ok_or_else(|| self.mkerr(format!("{name} needs an argument")))?;
                    v(self.type_of(a, sid)?.width(), name == "$signed")
                }
                "$clog2" | "$bits" => v(32, false),
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
                "$test$plusargs" | "$value$plusargs" => v(32, true),
                "$fgetc" | "$ungetc" | "$fgets" | "$feof" | "$ftell" | "$fseek" | "$rewind" | "$fscanf" | "$sscanf" => {
                    v(32, true)
                }
                "$rtoi" => v(32, true),
                "$realtobits" | "$__ticks" | "$__strbits" => v(64, false),
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
        if let Some(t) = f.ret_ty.as_ref().filter(|t| t.param.is_some()) {
            let r = self.resolve_type(t, fs)?;
            return self.resolved_type(&r, fs);
        }
        if let Some(t) = f.ret_ty.as_ref().filter(|t| t.heap.is_some()) {
            let w = self.const_int(&t.width(), fs)? as u32;
            let elem = self.elem_for(t.kind, w, t.signed, Some(t), fs)?;
            let kind = self.dyn_kind(t.heap.as_ref().unwrap(), fs)?;
            return Ok(Type::Dyn { kind, elem });
        }
        Ok(match f.ret_kind {
            NetKind::Class => match f.ret_ty.as_ref().and_then(|t| t.class.as_ref()) {
                Some(n) => Type::Class(self.find_class(n, fs).ok_or_else(|| self.mkerr(format!("unknown class '{n}'")))?),
                None => return self.err("a function returning a class handle without its class"),
            },
            NetKind::Integer => Type::vec(32, true),
            NetKind::Real => Type::Real,
            NetKind::String => Type::Str,
            _ => {
                let (m, l) = self.range_vals(&f.range, fs)?;
                Type::vec(((m - l).abs() + 1) as u32, f.signed)
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
                None => return self.err("unknown class of a type parameter"),
            },
            _ => Type::vec(packed_of(r).width() as u32, r.signed),
        })
    }

    /// `f` with the types its arguments and result take from type parameters resolved in
    /// scope `sid` (kind, sign and ranges of the type, as declarations of a module); `f`
    /// itself where it uses none.
    pub(crate) fn subr_resolved(&mut self, f: &'a SubroutineDecl, sid: usize) -> R<&'a SubroutineDecl> {
        let ret_param = f.ret_ty.as_ref().is_some_and(|t| t.param.is_some());
        if !ret_param && !f.decls.iter().any(|d| d.type_param.is_some()) {
            return Ok(f);
        }
        let mut g = f.clone();
        if ret_param {
            let r = self.resolve_type(f.ret_ty.as_ref().unwrap(), sid)?;
            if r.has_struct() {
                return self.err("a function returning a struct type given by a type parameter is not supported");
            }
            // (as the lowering gives a return type: a vector with its range, or a real, string
            // or class handle)
            (g.ret_kind, g.range) = match r.kind {
                k @ (NetKind::Real | NetKind::String | NetKind::Class) => (k, None),
                NetKind::Event => return self.err("a function returning an event is not supported"),
                _ => {
                    let w = packed_of(&r).width();
                    (NetKind::Reg, Some((ExprKind::Num(ferrooxide_front::ast::dec(w as u64 - 1)).into(), ExprKind::Num(ferrooxide_front::ast::dec(0)).into())))
                }
            };
            g.signed = r.signed;
            g.ret_ty = Some(Rc::new(r));
        }
        for d in &mut g.decls {
            let Some(tp) = d.type_param.take() else { continue };
            let Some(Sym::Type(t)) = self.lookup(&tp, sid) else {
                return self.err(format!("'{tp}' is not a type"));
            };
            if t.has_struct() {
                return self.err("an argument of a struct type given by a type parameter is not supported");
            }
            let (k, s, range, packed) = t.decl_parts();
            d.kind = k;
            d.signed = s;
            d.range = range;
            d.packed = packed;
            d.ty = Some(t.clone());
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
    pub(crate) fields: Vec<(String, Packed)>,
}

fn dim_size(d: (i64, i64)) -> i64 {
    (d.0 - d.1).abs() + 1
}

impl Packed {
    /// Width of one element after the packed dimensions.
    fn leaf_width(&self) -> i64 {
        match &self.fields {
            None => 1,
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
        Rc::new(Fields { union: f.union, fields: f.fields.iter().map(|(n, ft)| (n.clone(), packed_of(ft))).collect() })
    });
    let leaf = match t.kind {
        NetKind::Real => Leaf::Real,
        NetKind::String => Leaf::Str,
        _ => Leaf::Bits,
    };
    Packed { dims, signed: t.signed, int_kind: t.int_kind, fields, leaf, enum_names: t.enum_names.clone() }
}

/// The bit offsets of the real and string members in a value of type `t` at `off`.
fn value_members(t: &Packed, off: i64, out: &mut Vec<i64>) {
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
            Some(t) if !matches!(t.kind, NetKind::Real | NetKind::String | NetKind::Event) => Ok(Some(self.packed(t, sid)?)),
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
            _ => self.md_norm(&elem, sid)?,
        };
        let mut cur: Packed = layout;
        let mut di = 0;
        let mut off = num(0);
        let mut width = num(cur.width());
        let mut touched = false;
        let mut whole = true;
        loop {
            match ops.get(k) {
                Some(&op @ (Sel::Index(_) | Sel::Range(..) | Sel::Part(..))) if di < cur.dims.len() => {
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
                    let Some(fk) = st.fields.iter().position(|f| f.0 == *m) else {
                        return Some(Err(format!("no member '{m}' in the struct")));
                    };
                    k += 1;
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
        let whole_chain = k == ops.len();
        Some(Ok(Walk { expr: out, ty: if whole_chain { ty } else { None }, leaf: if whole_chain { leaf } else { Leaf::Bits } }))
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
        self.struct_walk(e, sid, true)?.ok()?.ty
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
        let mut pos = Vec::new();
        let mut named = Vec::new();
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
