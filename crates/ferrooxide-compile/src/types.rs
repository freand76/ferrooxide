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
            Type::Real | Type::Str => 64,
        }
    }

    /// Sign where the value is used as a vector (a real converts to a signed integer).
    pub(crate) fn signed(self) -> bool {
        match self {
            Type::Vec { signed, .. } => signed,
            Type::Real => true,
            Type::Str => false,
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
        if let Some(n) = self.md_norm(e, sid) {
            return self.type_of(&n, sid);
        }
        let v = Type::vec;
        Ok(match e {
            Expr::Type(_) => return self.err("a data type is not a value"),
            Expr::Num(n) => {
                if n.fill {
                    v(1, false)
                } else {
                    let c = Const::from_number(n).or_else(|m| self.err(m))?;
                    v(c.w, c.signed)
                }
            }
            Expr::Real(_) => Type::Real,
            Expr::Str(s) => v((8 * s.len()).max(8) as u32, false),
            Expr::Ident(_) | Expr::Hier(_) => match self.resolve_base(e, sid)? {
                Some(Base::Sig(s)) => self.sig_type(s),
                Some(Base::Const(c)) => v(c.w, c.signed),
                Some(Base::Real(_)) => Type::Real,
                None => unreachable!(),
            },
            Expr::Index(a, _) => {
                if self.is_string_scalar(a, sid) {
                    // a character of a string
                    v(8, false)
                } else if let Some(arr) = self.array_of(a, sid) {
                    self.sig_type(arr)
                } else {
                    v(1, false)
                }
            }
            Expr::Range(_, m, l) => match (self.const_index(m, sid)?, self.const_index(l, sid)?) {
                (Some(m), Some(l)) => v(((m - l).abs() + 1) as u32, false),
                // an x/z bound: a 1-bit x
                _ => v(1, false),
            },
            Expr::IndexedRange(_, _, w, _) => v(self.const_int(w, sid)? as u32, false),
            Expr::Unary(op, a) => match op {
                UnOp::Plus | UnOp::Neg => match self.type_of(a, sid)? {
                    Type::Real => Type::Real,
                    t => t.as_vec(),
                },
                UnOp::Not => self.type_of(a, sid)?.as_vec(),
                _ => v(1, false),
            },
            Expr::Binary(op, a, b) => match op {
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
            Expr::Cond(_, t, f) => {
                let (tt, tf) = (self.type_of(t, sid)?, self.type_of(f, sid)?);
                if tt.is_str() || tf.is_str() {
                    Type::Str
                } else if tt.is_real() || tf.is_real() {
                    Type::Real
                } else {
                    v(tt.width().max(tf.width()), tt.signed() && tf.signed())
                }
            }
            Expr::Concat(xs) | Expr::Repl(_, xs) => {
                let mut w = 0;
                let mut str = false;
                for x in xs {
                    let t = self.type_of(x, sid)?;
                    str |= t.is_str();
                    w += t.width();
                }
                if str {
                    Type::Str
                } else if let Expr::Repl(n, _) = e {
                    let n = self.const_int(n, sid)?;
                    v((w as i64 * n.max(0)) as u32, false)
                } else {
                    v(w, false)
                }
            }
            Expr::Call(name, _) => {
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
                    return self.err(format!("unknown function '{name}'"));
                };
                if f.is_task {
                    return self.err(format!("task '{name}' used as a function"));
                }
                self.func_ret_type(f, fs)?
            }
            Expr::SysCall(name, args) => match name.as_str() {
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
                "$realtobits" | "$__ticks" => v(64, false),
                "$countones" => v(32, false),
                "$onehot" | "$onehot0" | "$isunknown" => v(1, false),
                _ => return self.err(format!("unsupported system function '{name}'")),
            },
        })
    }

    /// The type a function returns.
    pub(crate) fn func_ret_type(&mut self, f: &SubroutineDecl, fs: usize) -> R<Type> {
        Ok(match f.ret_kind {
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
