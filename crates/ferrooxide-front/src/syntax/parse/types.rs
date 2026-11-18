//! Data types and dimensions.

use super::Parser;
use crate::syntax::kw::Kw;
use crate::syntax::token::T;
use crate::syntax::tree::*;
use crate::Result;

pub(crate) fn int_kw(k: Kw) -> Option<IntKw> {
    Some(match k {
        Kw::Bit => IntKw::Bit,
        Kw::Logic => IntKw::Logic,
        Kw::Reg => IntKw::Reg,
        Kw::Byte => IntKw::Byte,
        Kw::Shortint => IntKw::Shortint,
        Kw::Int => IntKw::Int,
        Kw::Longint => IntKw::Longint,
        Kw::Integer => IntKw::Integer,
        Kw::Time => IntKw::Time,
        _ => return None,
    })
}

pub(crate) fn real_kw(k: Kw) -> Option<RealKw> {
    Some(match k {
        Kw::Real => RealKw::Real,
        Kw::Realtime => RealKw::Realtime,
        Kw::Shortreal => RealKw::Shortreal,
        _ => return None,
    })
}

/// A keyword that starts a data type.
pub(crate) fn is_type_kw(k: Kw) -> bool {
    int_kw(k).is_some()
        || real_kw(k).is_some()
        || matches!(k, Kw::String | Kw::Chandle | Kw::Event | Kw::Void | Kw::Struct | Kw::Union | Kw::Enum)
}

impl Parser {
    /// `signed` / `unsigned`
    pub(crate) fn signing(&mut self) -> Option<bool> {
        if self.eat_kw(Kw::Signed) {
            Some(true)
        } else if self.eat_kw(Kw::Unsigned) {
            Some(false)
        } else {
            None
        }
    }

    pub(crate) fn packed_dims(&mut self) -> Result<Vec<Dim>> {
        let mut v = Vec::new();
        while self.is_p("[") {
            v.push(self.dim()?);
        }
        Ok(v)
    }

    /// Unpacked dimensions of a declarator.
    pub(crate) fn unpacked_dims(&mut self) -> Result<Vec<Dim>> {
        self.packed_dims()
    }

    /// One dimension `[...]`.
    pub(crate) fn dim(&mut self) -> Result<Dim> {
        self.expect_p("[")?;
        if self.eat_p("]") {
            return Ok(Dim::Unsized);
        }
        if self.is_p("*") && self.is_p_at(1, "]") {
            self.bump();
            self.bump();
            return Ok(Dim::Assoc(None));
        }
        if self.is_p("$") {
            self.bump();
            let max = if self.eat_p(":") { Some(self.expr()?) } else { None };
            self.expect_p("]")?;
            return Ok(Dim::Queue(max));
        }
        // `[int]`, `[string]`, `[T]`: an associative array indexed by a type
        if self.type_at_bracket() {
            let t = self.data_type()?.unwrap();
            self.expect_p("]")?;
            return Ok(Dim::Assoc(Some(t)));
        }
        let a = self.expr()?;
        let d = if self.eat_p(":") {
            let b = self.expr()?;
            Dim::Range(a, b)
        } else {
            Dim::Size(a)
        };
        self.expect_p("]")?;
        Ok(d)
    }

    /// A type (not an expression) inside `[...]`.
    fn type_at_bracket(&self) -> bool {
        match self.peek() {
            T::Kw(k) if is_type_kw(*k) => true,
            T::Ident(n) => self.is_type_name(n) && (self.is_p_at(1, "]") || self.is_p_at(1, "[") || self.is_p_at(1, "::")),
            _ => false,
        }
    }

    /// A data type starts at the cursor (keywords, struct/enum, a type name, `p::T`,
    /// `virtual`, `type(`).
    pub(crate) fn is_type_start(&self) -> bool {
        self.is_type_start_at(0)
    }

    pub(crate) fn is_type_start_at(&self, k: usize) -> bool {
        match self.peek_at(k) {
            T::Kw(Kw::Virtual) => true,
            T::Kw(Kw::Type) => self.is_p_at(k + 1, "("),
            T::Kw(kw) if is_type_kw(*kw) => true,
            T::Ident(n) => {
                if self.is_type_name(n) {
                    // `C::T x`: a type only if T is one of class C's types (`C::p = 1` is a
                    // static property)
                    if self.is_p_at(k + 1, "::") {
                        if let (Some(c), T::Ident(t)) = (self.classes.get(n.as_str()), self.peek_at(k + 2)) {
                            return c.contains(t.as_str());
                        }
                    }
                    // `C #(...) x`: a parameterized class; `M #(...) u (...)` is an instance
                    if self.is_p_at(k + 1, "#") {
                        return self.classes.contains_key(n.as_str());
                    }
                    return !matches!(self.peek_at(k + 1), T::Punct("(" | "." | "=" | "<=" | ";" | "'"));
                }
                // `p::T`
                if self.is_p_at(k + 1, "::") {
                    if let (Some(p), T::Ident(t)) = (self.packages.get(n.as_str()), self.peek_at(k + 2)) {
                        return p.contains(t.as_str()) && !self.is_p_at(k + 3, "'");
                    }
                }
                false
            }
            T::SysIdent(n) if n == "$unit" => {
                self.is_p_at(k + 1, "::") && matches!(self.peek_at(k + 2), T::Ident(t) if self.scopes[0].contains(t.as_str()))
            }
            _ => false,
        }
    }

    /// A data type, or None if none starts here (the cursor does not move then).
    pub(crate) fn data_type(&mut self) -> Result<Option<DataType>> {
        let loc = self.loc();
        let kind = match self.peek().clone() {
            T::Kw(k) if int_kw(k).is_some() => {
                self.bump();
                TypeKind::Int(int_kw(k).unwrap())
            }
            T::Kw(k) if real_kw(k).is_some() => {
                self.bump();
                TypeKind::Real(real_kw(k).unwrap())
            }
            T::Kw(Kw::String) => {
                self.bump();
                TypeKind::String
            }
            T::Kw(Kw::Chandle) => {
                self.bump();
                TypeKind::Chandle
            }
            T::Kw(Kw::Event) => {
                self.bump();
                TypeKind::Event
            }
            T::Kw(Kw::Void) => {
                self.bump();
                TypeKind::Void
            }
            T::Kw(Kw::Struct | Kw::Union) => return self.struct_type().map(Some),
            T::Kw(Kw::Enum) => return self.enum_type().map(Some),
            T::Kw(Kw::Virtual) => {
                self.bump();
                self.eat_kw(Kw::Interface);
                let name = self.ident()?;
                let params = if self.is_p("#") { Some(self.param_values()?) } else { None };
                let modport = if self.eat_p(".") { Some(self.ident()?) } else { None };
                TypeKind::VirtualInterface { name, params, modport }
            }
            T::Kw(Kw::Type) if self.is_p_at(1, "(") => {
                self.bump();
                self.bump();
                let e = self.expr_or_type()?;
                self.expect_p(")")?;
                TypeKind::TypeOf(Box::new(e))
            }
            T::Ident(_) | T::SysIdent(_) if self.is_type_start() => TypeKind::Named(Box::new(self.type_name()?)),
            _ => return Ok(None),
        };
        let signed = self.signing();
        let packed = self.packed_dims()?;
        Ok(Some(DataType { kind, signed, packed, loc }))
    }

    /// A data type, or the implicit type of a declaration (signing and packed dimensions
    /// only, possibly nothing).
    pub(crate) fn data_type_or_implicit(&mut self) -> Result<DataType> {
        if let Some(t) = self.data_type()? {
            return Ok(t);
        }
        let loc = self.loc();
        let signed = self.signing();
        let packed = self.packed_dims()?;
        Ok(DataType { kind: TypeKind::Implicit, signed, packed, loc })
    }

    /// A possibly scoped type name `T`, `p::T`, `C#(8)::T`, `$unit::T`.
    pub(crate) fn type_name(&mut self) -> Result<Name> {
        let loc = self.loc();
        let mut scopes = Vec::new();
        let mut name = match self.bump() {
            T::SysIdent(s) => s,
            T::Ident(s) => s,
            T::Kw(k) if !k.is_hard() => k.as_str().to_string(),
            _ => {
                self.pos -= 1;
                return self.err(format!("expected a type name, found {}", self.describe()));
            }
        };
        loop {
            let params = if self.is_p("#") && self.is_p_at(1, "(") { Some(self.param_values()?) } else { None };
            if self.eat_p("::") {
                scopes.push(NamePart { name, params });
                name = self.ident()?;
                continue;
            }
            return Ok(Name { scopes, name, params, loc });
        }
    }

    fn struct_type(&mut self) -> Result<DataType> {
        let loc = self.loc();
        let union = self.is_kw(Kw::Union);
        self.bump();
        let soft = union && self.eat_kw(Kw::Soft);
        let tagged = self.eat_kw(Kw::Tagged);
        let packed = self.eat_kw(Kw::Packed);
        let signed = self.signing();
        self.expect_p("{")?;
        let mut members = Vec::new();
        while !self.eat_p("}") {
            let mloc = self.loc();
            let rand = if self.is_kw(Kw::Rand) || self.is_kw(Kw::Randc) { self.kw().inspect(|_| { self.bump(); }) } else { None };
            let Some(ty) = self.data_type()? else {
                return self.err(format!("expected a member type, found {}", self.describe()));
            };
            let decls = self.declarators()?;
            members.push(StructMember { rand, ty, decls, loc: mloc });
        }
        let dims = self.packed_dims()?;
        Ok(DataType {
            kind: TypeKind::Struct(Box::new(StructType { union, tagged, soft, packed, members })),
            signed,
            packed: dims,
            loc,
        })
    }

    fn enum_type(&mut self) -> Result<DataType> {
        let loc = self.loc();
        self.bump();
        let base = if self.is_p("{") { None } else { Some(self.data_type_or_implicit()?) };
        self.expect_p("{")?;
        let mut items = Vec::new();
        loop {
            let iloc = self.loc();
            let name = self.ident()?;
            let range = if self.eat_p("[") {
                let a = self.expr()?;
                let b = if self.eat_p(":") { Some(self.expr()?) } else { None };
                self.expect_p("]")?;
                Some((a, b))
            } else {
                None
            };
            let value = if self.eat_p("=") { Some(self.expr()?) } else { None };
            items.push(EnumItem { name, range, value, loc: iloc });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        let packed = self.packed_dims()?;
        Ok(DataType { kind: TypeKind::Enum(Box::new(EnumType { base, items })), signed: None, packed, loc })
    }

    /// `name [dims] [= init], ...;` (the `;` is consumed)
    pub(crate) fn declarators(&mut self) -> Result<Vec<Declarator>> {
        let v = self.declarator_list()?;
        self.expect_p(";")?;
        Ok(v)
    }

    /// `name [dims] [= init], ...` without the terminator.
    pub(crate) fn declarator_list(&mut self) -> Result<Vec<Declarator>> {
        let mut v = Vec::new();
        loop {
            let loc = self.loc();
            let name = self.ident()?;
            let dims = self.unpacked_dims()?;
            let init = if self.eat_p("=") { Some(self.init_expr()?) } else { None };
            v.push(Declarator { name, dims, init, loc });
            if !self.eat_p(",") {
                break;
            }
        }
        Ok(v)
    }

    /// An initializer: an expression or `new ...` (class objects, dynamic arrays).
    pub(crate) fn init_expr(&mut self) -> Result<Expr> {
        self.expr()
    }
}
