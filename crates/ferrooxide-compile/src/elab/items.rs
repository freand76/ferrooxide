//! Module items: the walk over a module or generate scope.

use crate::*;

impl<'a> Compiler<'a> {
    /// Port `name` was declared without a range, its net or variable declaration (at `line`)
    /// with [a:b]: an error, or with `-gno-io-range-error` a warning (as the reference).
    fn scalar_port(&mut self, name: &str, line: u32, a: i64, b: i64) -> R<()> {
        let msg = format!("Scalar port ``{name}'' has a vectored net declaration [{a}:{b}].");
        if !self.dialect.io_range_warn {
            self.line = line;
            return self.err(msg);
        }
        let w = format!("{}: warning: {msg}", ferrooxide_front::loc_str(&self.file, line));
        if !self.dialect.no_anachronisms && self.diag.warned.insert(w.clone()) {
            self.diag.warnings.push(w);
        }
        Ok(())
    }

    /// Rejects names declared twice among `decls` (the declarations of one scope, or of one
    /// subroutine; IEEE 1364-2005 12.3.3, 1800-2017 23.2.2.1): at most one direction and one
    /// net or variable declaration each; a direction declaration with a data type declares
    /// the port completely; with both, the ranges are the same (a typed declaration, or one
    /// without a range, goes only with a direction declaration without a range).
    pub(crate) fn check_redeclared<'d>(&mut self, decls: impl Iterator<Item = &'d Decl>, sid: usize) -> R<()> {
        let mut seen: HashMap<&str, (Option<&Decl>, Option<&Decl>)> = HashMap::new();
        for d in decls {
            if d.kind == NetKind::Genvar {
                continue;
            }
            for dl in &d.names {
                let e = seen.entry(dl.name.as_str()).or_default();
                let slot = if d.dir.is_some() { &mut e.0 } else { &mut e.1 };
                if slot.is_some() {
                    self.line = dl.line;
                    let what = if d.dir.is_some() { "port " } else { "" };
                    return self.err(format!("{what}'{}' is declared more than once", dl.name));
                }
                *slot = Some(d);
                let (Some(p), Some(v)) = *e else { continue };
                self.line = dl.line;
                let lines = format!("lines {} and {}", p.line.min(v.line), p.line.max(v.line));
                if p.explicit_kind {
                    return self.err(format!(
                        "port '{}' is declared twice ({lines}): a port declaration with a data type declares it completely",
                        dl.name
                    ));
                }
                if p.type_param.is_some() || v.type_param.is_some() || !p.range_written {
                    continue; // (a range on the net only: the scalar port check)
                }
                // (ranges that cannot be evaluated here, e.g. with a subroutine's own
                // parameters, are not compared)
                let same = v.range_written && {
                    let mut ranges = |d: &Decl| -> Option<Vec<(i64, i64)>> {
                        let mut r = Vec::new();
                        for (a, b) in d.range.iter().chain(&d.packed) {
                            r.push((self.const_int(a, sid).ok()?, self.const_int(b, sid).ok()?));
                        }
                        Some(r)
                    };
                    match (ranges(p), ranges(v)) {
                        (Some(a), Some(b)) => a == b,
                        _ => true,
                    }
                };
                if !same {
                    return self.err(format!("port '{}': its declarations ({lines}) have different ranges", dl.name));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn elab_items(
        &mut self,
        items: &'a [Item],
        sid: usize,
        module_level: bool,
        port_conns: &HashMap<String, Option<Expr>>,
        parent_scope: Option<usize>,
    ) -> R<()> {
        // 1. subroutines, classes and local parameters of generate scopes
        for it in items {
            if let Item::Subroutine(f) = it {
                self.scopes[sid].funcs.insert(f.name.clone(), f);
            }
        }
        if !module_level {
            for it in items {
                if let Item::TypeParam { name, default: Some(d), line, .. } = it {
                    self.line = *line;
                    let t = Rc::new(self.resolve_type(d, sid)?);
                    self.scopes[sid].syms.insert(name.clone(), Sym::Type(t));
                }
                if let Item::Param(p) = it {
                    self.line = p.line;
                    let v = self.param_value(p, None, sid)?;
                    self.insert_param(p, v, sid)?;
                if p.range.is_some() && !p.real {
                    let r = self.range_vals(&p.range, sid)?;
                    self.scopes[sid].param_ranges.insert(p.name.clone(), r);
                }
                }
            }
        }
        self.register_defparams(items, sid)?;
        // (after the parameters, which property types may use; the ids first, so that a
        // property can be a handle of a class declared after it)
        let mut id = self.classes.list.len() as u32;
        for it in items {
            if let Item::Class(c) = it {
                self.classes.by_name.insert((sid, c.name.clone()), id);
                id += 1;
            }
        }
        for it in items {
            if let Item::Class(c) = it {
                self.declare_class(c, sid)?;
            }
        }
        // 2. declarations (merged by name: `output [3:0] q; reg [3:0] q;`)
        self.check_redeclared(items.iter().filter_map(|it| if let Item::Decl(d) = it { Some(d) } else { None }), sid)?;
        struct Merged<'d> {
            dir: Option<Dir>,
            kind: NetKind,
            net: NetType,
            real_net: bool,
            strength: Option<(u8, u8)>,
            delay: Option<&'d Expr>,
            signed: bool,
            range: Option<&'d Range>,
            packed: &'d [Range],
            dims: &'d [Range],
            init: Option<&'d Expr>,
            /// declared with this type parameter
            tparam: Option<&'d str>,
            /// the declared data type
            ty: Option<&'d Rc<SvType>>,
            /// a queue or dynamic array
            heap: Option<&'d HeapKind>,
            line: u32,
            /// declared as a port without a range (`input a;`)
            scalar_port: bool,
            /// the line of a declaration without a direction that has a range
            vec_line: Option<u32>,
        }
        let mut order: Vec<String> = Vec::new();
        let mut merged: HashMap<String, Merged> = HashMap::new();
        for it in items {
            if let Item::Decl(d) = it {
                if d.kind == NetKind::Genvar {
                    continue;
                }
                let scalar_port = d.dir.is_some() && d.range.is_none() && d.packed.is_empty() && d.ty.as_ref().is_none_or(|t| t.fields.is_none() && t.enum_names.is_none()) && matches!(d.kind, NetKind::Wire | NetKind::Reg);
                let vec_line = (d.dir.is_none() && d.range_written).then_some(d.line);
                for dl in &d.names {
                    match merged.get_mut(&dl.name) {
                        Some(mg) => {
                            mg.scalar_port |= scalar_port;
                            mg.vec_line = mg.vec_line.or(vec_line);
                            if d.dir.is_some() {
                                mg.dir = d.dir;
                            }
                            if d.explicit_kind && d.kind != NetKind::Wire {
                                mg.kind = d.kind;
                            }
                            if d.net != NetType::Wire {
                                mg.net = d.net;
                            }
                            mg.real_net |= d.real_net;
                            if d.strength.is_some() {
                                mg.strength = d.strength;
                            }
                            if d.delay.is_some() {
                                mg.delay = d.delay.as_ref();
                            }
                            mg.signed |= d.signed;
                            if d.range.is_some() {
                                mg.range = d.range.as_ref();
                                mg.packed = &d.packed;
                            }
                            if !dl.dims.is_empty() {
                                mg.dims = &dl.dims;
                            }
                            if dl.init.is_some() {
                                mg.init = dl.init.as_ref();
                            }
                            if d.type_param.is_some() {
                                mg.tparam = d.type_param.as_deref();
                            }
                            if d.ty.is_some() {
                                mg.ty = d.ty.as_ref();
                            }
                            if dl.heap.is_some() {
                                mg.heap = dl.heap.as_ref();
                            }
                        }
                        None => {
                            order.push(dl.name.clone());
                            merged.insert(
                                dl.name.clone(),
                                Merged {
                                    dir: d.dir,
                                    kind: d.kind,
                                    net: d.net,
                                    real_net: d.real_net,
                                    strength: d.strength,
                                    delay: d.delay.as_ref(),
                                    signed: d.signed,
                                    range: d.range.as_ref(),
                                    packed: &d.packed,
                                    dims: &dl.dims,
                                    init: dl.init.as_ref(),
                                    tparam: d.type_param.as_deref(),
                                    ty: d.ty.as_ref(),
                                    heap: dl.heap.as_ref(),
                                    line: dl.line,
                                    scalar_port,
                                    vec_line,
                                },
                            );
                        }
                    }
                }
            }
        }
        let mut port_dirs: HashMap<String, Dir> = HashMap::new();
        // the module's port list (signals named in port expressions have a direction but are
        // not ports themselves)
        let port_list = if module_level { self.hier.scope_module.get(&sid).map(|m| &m.ports) } else { None };
        for name in order {
            let mut mg = merged.remove(&name).unwrap();
            self.line = mg.line;
            if let (true, Some(l)) = (mg.scalar_port && mg.dir.is_some(), mg.vec_line) {
                if let Some((a, b)) = mg.range {
                    let (a, b) = (self.const_int(a, sid)?, self.const_int(b, sid)?);
                    self.scalar_port(&name, l, a, b)?;
                }
            }
            if port_list.is_some_and(|p| !p.contains(&name)) {
                mg.dir = None;
            }
            if let Some(d) = mg.dir {
                port_dirs.insert(name.clone(), d);
            }
            // the range and further packed dimensions, evaluated
            let mut range_v = None;
            let mut packed_v = Vec::new();
            if let Some(tp) = mg.tparam {
                let Some(Sym::Type(t)) = self.lookup(tp, sid) else {
                    return self.err(format!("'{tp}' is not a type"));
                };
                let (k, sg, r, p) = t.decl_parts();
                // (an input port of a logic type is a net)
                mg.kind = if k == NetKind::Reg && matches!(mg.dir, Some(Dir::Input | Dir::Inout)) { NetKind::Wire } else { k };
                mg.signed = sg;
                if let Some((a, b)) = r {
                    range_v = Some((self.const_int(&a, sid)?, self.const_int(&b, sid)?));
                }
                for (a, b) in &p {
                    packed_v.push((self.const_int(a, sid)?, self.const_int(b, sid)?));
                }
            } else {
                if let Some((a, b)) = mg.range {
                    range_v = Some((self.const_int(a, sid)?, self.const_int(b, sid)?));
                }
                for (a, b) in mg.packed {
                    packed_v.push((self.const_int(a, sid)?, self.const_int(b, sid)?));
                }
            }
            let (mut msb, mut lsb) = range_v.unwrap_or((0, 0));
            // packed dimensions: one flat vector [W-1:0], the dimensions kept for selects
            let mut pk: Option<Vec<(i64, i64)>> = None;
            if !packed_v.is_empty() {
                let mut dims = vec![(msb, lsb)];
                dims.extend(packed_v.iter().copied());
                let w: i64 = dims.iter().map(|(a, b)| (a - b).abs() + 1).product();
                (msb, lsb) = (w - 1, 0);
                pk = Some(dims);
            }
            let layout = self.layout_for(mg.ty, sid)?;
            let mut signed = mg.signed;
            let kind = match mg.kind {
                NetKind::Wire => SigKind::Wire,
                NetKind::Integer => {
                    (msb, lsb) = (31, 0);
                    signed = true;
                    SigKind::Integer
                }
                NetKind::Real if mg.real_net => {
                    (msb, lsb) = (63, 0);
                    SigKind::Wire
                }
                NetKind::Time | NetKind::Real => {
                    (msb, lsb) = (63, 0);
                    SigKind::Reg
                }
                NetKind::Event => SigKind::Reg,
                NetKind::String | NetKind::Class => {
                    (msb, lsb) = (63, 0);
                    SigKind::Reg
                }
                NetKind::Int(n) => {
                    if range_v.is_none() {
                        (msb, lsb) = (n as i64 - 1, 0);
                    }
                    SigKind::Integer
                }
                _ => SigKind::Reg,
            };
            let is_real = mg.kind == NetKind::Real;
            let is_event = mg.kind == NetKind::Event;
            let width = ((msb - lsb).abs() + 1) as u32;
            // a multi-dimensional array is stored flat (row-major); `md` keeps its dimensions
            let mut md: Option<Vec<(i64, u32, bool)>> = None;
            let mut desc = false;
            let (depth, arr_lo) = match mg.dims.len() {
                0 => (0, 0),
                1 => {
                    let (a, b) = (self.const_int(&mg.dims[0].0, sid)?, self.const_int(&mg.dims[0].1, sid)?);
                    desc = a > b;
                    (((a - b).abs() + 1) as u32, a.min(b))
                }
                _ => {
                    let mut dims = Vec::new();
                    let mut total: u64 = 1;
                    for (x, y) in mg.dims {
                        let (a, b) = (self.const_int(x, sid)?, self.const_int(y, sid)?);
                        let n = ((a - b).abs() + 1) as u32;
                        total *= n as u64;
                        dims.push((a.min(b), n, a > b));
                    }
                    if total > u32::MAX as u64 {
                        return self.err(format!("array '{name}' is too large"));
                    }
                    md = Some(dims);
                    (total as u32, 0)
                }
            };
            let full = format!("{}.{}", self.scopes[sid].name, name);
            // a queue or dynamic array: a handle
            if let Some(h) = mg.heap {
                if mg.dir.is_some() && module_level {
                    return self.err(format!("port '{name}': a queue or dynamic array port is not supported"));
                }
                let elem = self.elem_for(mg.kind, width, signed, mg.ty.map(|t| &**t), sid)?;
                self.new_dyn(full, &name, elem, h, sid)?;
                if let Some(init) = mg.init {
                    let line = self.line;
                    self.code.jobs.push(Job::Init { lhs: Expr::Ident(name.clone()), rhs: init.clone(), scope: sid, line });
                }
                continue;
            }
            // (an array of class handles: an array of 64-bit words, below)
            let mut class_arr = None;
            if mg.kind == NetKind::Class {
                // a class handle: starts as null; an initializer is assigned at time 0
                if mg.dir.is_some() {
                    return self.err(format!("'{name}': a class handle port is not supported"));
                }
                let c = match mg.ty.and_then(|t| t.class.clone()) {
                    Some(n) => self.find_class(&n, sid).ok_or_else(|| self.mkerr(format!("unknown class '{n}'")))?,
                    None => return self.err(format!("'{name}': a class handle without its class")),
                };
                if depth > 0 {
                    if mg.init.is_some() {
                        return self.err(format!("'{name}': an initializer of an array of class handles is not supported"));
                    }
                    class_arr = Some(c);
                }
            }
            if mg.kind == NetKind::Class && class_arr.is_none() {
                let c = self.find_class(mg.ty.and_then(|t| t.class.as_deref()).unwrap_or_default(), sid).unwrap();
                let s = self.new_signal(full, 64, false, 0, 63, 0, 0, SigKind::Reg);
                self.sigs.types[s as usize] = types::Type::Class(c);
                self.zero_init(s);
                self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
                if let Some(init) = mg.init {
                    let line = self.line;
                    self.code.jobs.push(Job::Init { lhs: Expr::Ident(name.clone()), rhs: init.clone(), scope: sid, line });
                }
                continue;
            }
            // port aliasing
            if let (Some(dir), Some(ps)) = (mg.dir, parent_scope) {
                if module_level {
                    if let Some(Some(conn)) = port_conns.get(&name) {
                        let mut aliased = None;
                        if depth == 0 {
                            if let Expr::Ident(pn) = conn {
                                let sym = match self.lookup(pn, ps) {
                                    Some(s) => Some(s),
                                    None => {
                                        // implicit net in the parent
                                        let s = self.implicit_net(pn, width, ps)?;
                                        let _ = (msb, lsb);
                                        Some(Sym::Sig(s))
                                    }
                                };
                                if let Some(Sym::Sig(s)) = sym {
                                    let si = self.sig(s);
                                    if si.width == width && si.depth == 0 {
                                        aliased = Some(s);
                                    }
                                }
                            }
                        }
                        if depth == 0 && aliased.is_none() {
                            aliased = self.elem_port_alias(conn, ps, width, &full);
                        }
                        // an unpacked array port: another name for a connected array of the
                        // same shape (element type, index range and direction)
                        if depth > 0 {
                            let typed = mg.ty.is_some_and(|t| t.has_struct() || t.param.is_some());
                            let same = match self.whole_array(conn, ps)? {
                                Some((a, _)) => {
                                    let ai = self.sig(a);
                                    md.is_none()
                                        && pk.is_none()
                                        && !typed
                                        && !is_real
                                        && (ai.depth, ai.arr_lo, ai.width, ai.msb, ai.lsb, ai.signed) == (depth, arr_lo, width, msb, lsb, signed)
                                        && self.sigs.desc_arrays.contains(&a) == desc
                                }
                                None => false,
                            };
                            if !same {
                                return self.err(format!("array port '{name}' must be connected to an array of the same shape (others are not supported)"));
                            }
                            aliased = self.whole_array(conn, ps)?.map(|x| x.0);
                        }
                        if let Some(s) = aliased {
                            let si = self.sig(s);
                            let typed = mg.ty.is_some_and(|t| t.has_struct() || t.param.is_some());
                            let view = (si.msb, si.lsb, si.signed) != (msb, lsb, signed) || pk.is_some() || typed;
                            let s = if view {
                                // own range / sign / packed dimensions: a view
                                let v = self.new_view(s, full.clone(), msb, lsb, signed);
                                self.prog.names.pop();
                                if let Some(d) = pk.take() {
                                    self.sigs.packed_dims.insert(v, d);
                                    self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(v));
                                }
                                v
                            } else {
                                s
                            };
                            let s0 = self.canon(s);
                            if kind != SigKind::Wire && self.sig(s0).kind == SigKind::Wire {
                                // a variable output collapsed onto a net: starts as x, not z
                                let (slot, w) = (self.sig(s0).slot as usize, self.sig(s0).width);
                                let n = rt::nw(w);
                                for i in 0..n {
                                    self.prog.mem[slot + i] = self.prog.mem[slot + n + i];
                                }
                                self.prog.signals[s0 as usize].kind = SigKind::Reg;
                                self.prog.signals[s as usize].kind = SigKind::Reg;
                            }
                            self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
                            self.prog.names.push((full, s));
                            // (a layout belongs to this name: only on a view of its own)
                            if view {
                                self.set_layout(s, &name, layout, sid)?;
                            }
                            if let Some(init) = mg.init {
                                self.decl_init(init, s, kind, mg.kind, is_real, depth, width, mg.strength, mg.delay, &name, sid)?;
                            }
                            continue;
                        }
                        let s = self.new_signal(full, width, signed, depth, msb, lsb, arr_lo, kind);
                        if desc {
                            self.sigs.desc_arrays.insert(s);
                        }
                        if is_real {
                            self.mark_real(s);
                        }
                        if mg.kind == NetKind::String {
                            self.mark_string(s);
                        }
                        self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
                        self.set_layout(s, &name, layout, sid)?;
                        let line = self.line;
                        match dir {
                            Dir::Input => {
                                self.code.jobs.push(Job::PortIn { child_sig: s, expr: conn.clone(), scope: ps, line })
                            }
                            Dir::Output => {
                                self.code.jobs.push(Job::PortOut { lhs: conn.clone(), child_sig: s, scope: ps, line })
                            }
                            Dir::Inout if self.link_exprs(&Expr::Ident(name.clone()), sid, conn, ps, None) => {}
                            Dir::Inout => self.warn(format!(
                                "inout port '{name}' connected to an expression: connection ignored"
                            )),
                        }
                        if let Some(init) = mg.init {
                            self.decl_init(init, s, kind, mg.kind, is_real, depth, width, mg.strength, mg.delay, &name, sid)?;
                        }
                        continue;
                    }
                }
            }
            let s = self.new_signal(full, width, signed, depth, msb, lsb, arr_lo, kind);
            if desc {
                self.sigs.desc_arrays.insert(s);
            }
            if let Some(d) = pk.take() {
                self.sigs.packed_dims.insert(s, d);
                self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(s));
            }
            if let Some(d) = md.take() {
                self.sigs.md_dims.insert(s, d);
                // the flat storage, for index chains already turned into one index
                self.scopes[sid].syms.insert(format!("$flat:{name}"), Sym::Sig(s));
            }
            if is_real {
                self.mark_real(s);
            }
            if mg.kind == NetKind::String {
                self.mark_string(s);
            }
            if let Some(c) = class_arr {
                // every element starts as null
                self.sigs.types[s as usize] = types::Type::Class(c);
                self.zero_init(s);
            }
            if is_event {
                self.zero_init(s);
                self.prog.signals[s as usize].event = true;
                // the time of the last trigger, for `.triggered` (never at the start)
                let ht = self.new_signal(format!("{}.{name}$trig", self.scopes[sid].name), 64, false, 0, 63, 0, 0, SigKind::Reg);
                self.prog.names.pop();
                let slot = self.sig(ht).slot as usize;
                self.prog.mem[slot] = !0;
                self.prog.mem[slot + 1] = 0;
                self.scopes[sid].syms.insert(format!("{name}$trig"), Sym::Sig(ht));
                self.sigs.has_events = true;
            }
            self.mark_two_state(s, mg.kind);
            if mg.net != NetType::Wire {
                self.nets.net_types.insert(s, mg.net);
                if matches!(mg.net, NetType::Supply0 | NetType::Supply1) {
                    let d = if mg.net == NetType::Supply1 { "1" } else { "0" };
                    let rhs = Expr::Num(Number { size: None, signed: false, base: 2, based: true, digits: d.into(), fill: true });
                    let line = self.line;
                    self.nets.cur_strength = Some((7, 7));
                    self.push_job(Job::Assign { lhs: Expr::Ident(name.clone()), rhs, delay: None, scope: sid, line });
                    self.nets.cur_strength = None;
                }
            }
            self.scopes[sid].syms.insert(name.clone(), Sym::Sig(s));
            self.set_layout(s, &name, layout, sid)?;
            // `unconnected_drive: an unconnected input port is pulled
            if let (Some(Dir::Input), true, Some(up)) = (mg.dir, module_level, self.scopes[sid].uncdrive) {
                if parent_scope.is_some() && !matches!(port_conns.get(&name), Some(Some(_))) && depth == 0 {
                    self.nets.pulls.entry(s).or_default().push((0, width, up));
                }
            }
            if let Some(init) = mg.init {
                self.decl_init(init, s, kind, mg.kind, is_real, depth, width, mg.strength, mg.delay, &name, sid)?;
            }
        }
        // function and task ports declared without a range that get one later: warned about
        // (as the reference does) by subroutine name
        let mut subs: Vec<&SubroutineDecl> = items.iter().filter_map(|it| if let Item::Subroutine(f) = it { Some(f) } else { None }).collect();
        subs.sort_by(|a, b| a.name.cmp(&b.name));
        for f in subs {
            let written = if f.decls_written.is_empty() { &f.decls } else { &f.decls_written };
            self.check_redeclared(written.iter(), sid)?;
            for (n, l) in &f.scalar_vec_ports {
                let Some((a, b)) = f.decls.iter().find(|d| d.dir.is_some() && d.names.iter().any(|x| &x.name == n)).and_then(|d| d.range.as_ref()) else { continue };
                let (a, b) = (self.const_int(a, sid)?, self.const_int(b, sid)?);
                self.scalar_port(n, *l, a, b)?;
            }
        }
        // 3b. implicit nets: undeclared names on the left of continuous assignments and as gate
        // terminals become 1-bit wires
        let mut implicit: Vec<&String> = Vec::new();
        fn lhs_names<'e>(e: &'e Expr, out: &mut Vec<&'e String>) {
            match e {
                Expr::Ident(n) => out.push(n),
                Expr::Concat(v) => v.iter().for_each(|x| lhs_names(x, out)),
                _ => {}
            }
        }
        for it in items {
            match it {
                Item::ContAssign { lhs, .. } => lhs_names(lhs, &mut implicit),
                Item::Gate { terms, .. } => terms.iter().for_each(|t| lhs_names(t, &mut implicit)),
                _ => {}
            }
        }
        for n in implicit {
            if self.lookup(n, sid).is_none() {
                self.implicit_net(n, 1, sid)?;
            }
        }
        // 3b'. specify path delays on output ports
        if module_level && self.dialect.specify {
            self.path_delays(items, sid, &port_dirs)?;
        }
        // 3c. static tasks and functions, so hierarchical names can reach their variables
        for it in items {
            if let Item::Subroutine(f) = it {
                if !f.automatic {
                    self.func_instance(f, sid, &f.name)?;
                }
            }
        }
        // 4. everything else; interface instances first, so that modules can be connected to
        // interfaces declared after them
        let mut genblk = 0;
        let is_iface = |it: &Item| matches!(it, Item::Instances { module, .. } if self.find_module(module, sid).is_some_and(|m| m.interface));
        let order: Vec<&'a Item> = items.iter().filter(|it| is_iface(it)).chain(items.iter().filter(|it| !is_iface(it))).collect();
        for it in order {
            match it {
                Item::Decl(_)
                | Item::Param(_)
                | Item::TypeParam { .. }
                | Item::Subroutine(_)
                | Item::Class(_)
                | Item::Defparam { .. }
                | Item::SpecPath { .. } => {}
                Item::ContAssign { delay, lhs, rhs, strength, line } => {
                    self.nets.cur_strength = *strength;
                    // (a whole unpacked array becomes an alias or one assignment per element in
                    // `alias_assigns`)
                    self.push_job(Job::Assign { lhs: lhs.clone(), rhs: rhs.clone(), delay: delay.clone(), scope: sid, line: *line });
                    self.nets.cur_strength = None;
                }
                Item::Process { kind, body, line } => {
                    self.code.jobs.push(Job::Proc { kind: *kind, body, scope: sid, line: *line });
                }
                Item::Instances { module, params, insts, line } => {
                    self.line = *line;
                    let m = match self.find_module(module, sid) {
                        Some(m) => m,
                        None => return self.err(format!("unknown module '{module}'")),
                    };
                    if m.udp.is_some() {
                        // a user-defined primitive: positional terminals, an optional delay
                        let delay = match params.first() {
                            Some(Conn::Positional(Some(e))) => Some(e.clone()),
                            _ => None,
                        };
                        for inst in insts {
                            let mut terms = Vec::new();
                            for c in &inst.conns {
                                match c {
                                    Conn::Positional(Some(e)) | Conn::Named(_, Some(e)) => terms.push(e.clone()),
                                    _ => return self.err("unconnected UDP terminal"),
                                }
                            }
                            if terms.len() != m.ports.len() {
                                return self.err(format!("primitive '{module}' needs {} terminals", m.ports.len()));
                            }
                            self.code.jobs.push(Job::Udp { module: m, terms, delay: delay.clone(), scope: sid, line: inst.line });
                        }
                        continue;
                    }
                    let mut ovs = Vec::new();
                    for p in params {
                        match p {
                            Conn::Positional(Some(e)) => ovs.push((None, self.pval(e, sid)?)),
                            Conn::Positional(None) => ovs.push((None, PVal::Default)),
                            Conn::Named(n, Some(e)) => ovs.push((Some(n.clone()), self.pval(e, sid)?)),
                            _ => {}
                        }
                    }
                    for inst in insts {
                        self.line = inst.line;
                        if inst.name.starts_with('$') {
                            return self.err(format!("Instantiation of module {} requires an instance name.", m.name));
                        }
                        if inst.dims.len() > 1 {
                            return self.err("multi-dimensional arrays of instances are not supported");
                        }
                        if let Some((l, r)) = inst.dims.first() {
                            // an array of instances: name[i] for i from l to r, terminals sliced
                            let (l, r) = (self.const_int(l, sid)?, self.const_int(r, sid)?);
                            let n = ((l - r).abs() + 1) as u32;
                            let pws = self.port_widths(m, &ovs)?;
                            self.hier.inst_arrays.insert((sid, inst.name.clone()), (l, r));
                            // elaborated lowest index first, like Icarus
                            for i in l.min(r)..=l.max(r) {
                                let k = (i - l).unsigned_abs() as u32;
                                let mut conns = Vec::new();
                                for (pos, c) in inst.conns.iter().enumerate() {
                                    let (port, e) = match c {
                                        Conn::Positional(e) => (m.ports.get(pos).cloned().unwrap_or_default(), e),
                                        Conn::Named(nm, e) => (nm.clone(), e),
                                        Conn::Wildcard => {
                                            conns.push(Conn::Wildcard);
                                            continue;
                                        }
                                    };
                                    let e = match e {
                                        Some(e) => {
                                            let pw = pws.get(&port).copied().unwrap_or(1);
                                            let tw = self.type_of(e, sid)?.width();
                                            if n > 1 && tw == pw * n {
                                                Some(self.array_slice(e, k, n, pw, sid)?)
                                            } else if tw == pw || tw < pw * n {
                                                Some(e.clone())
                                            } else {
                                                return self.err(format!(
                                                    "port '{port}' of instance array '{}': {tw} bits do not fit {n} x {pw}",
                                                    inst.name
                                                ));
                                            }
                                        }
                                        None => None,
                                    };
                                    conns.push(match c {
                                        Conn::Positional(_) => Conn::Positional(e),
                                        _ => Conn::Named(port, e),
                                    });
                                }
                                // the generated connections live as long as the elaboration
                                let conns: &'a [Conn] = Box::leak(conns.into_boxed_slice());
                                let iname = format!("{}[{}]", inst.name, i);
                                let path = format!("{}.{}", self.scopes[sid].name, iname);
                                let child = self.instantiate(m, path, &ovs, Some((conns, sid)))?;
                                self.scopes[sid].children.insert(iname, child);
                            }
                            continue;
                        }
                        let path = format!("{}.{}", self.scopes[sid].name, inst.name);
                        let child = self.instantiate(m, path, &ovs, Some((&inst.conns, sid)))?;
                        self.scopes[sid].children.insert(inst.name.clone(), child);
                    }
                }
                Item::GenFor { init_var, init, cond, step_var, step, name, items: body, line } => {
                    self.line = *line;
                    genblk += 1;
                    let bname = name.clone().unwrap_or_else(|| format!("genblk{genblk}"));
                    let mut v = self.const_int(init, sid)?;
                    let mut iters = 0;
                    loop {
                        let it_name = format!("{bname}[{v}]");
                        let path = format!("{}.{}", self.scopes[sid].name, it_name);
                        let file = self.file.clone();
                        let isid = self.new_scope(path, Some(sid), file);
                        self.scopes[isid].syms.insert(init_var.clone(), Sym::Const(Const::from_i64(v, 32, true)));
                        if !self.const_eval(cond, isid)?.is_true() {
                            break;
                        }
                        self.scopes[sid].children.insert(it_name, isid);
                        self.elab_items(body, isid, false, &HashMap::new(), None)?;
                        if step_var != init_var {
                            return self.err("generate loop must step its own genvar");
                        }
                        v = self.const_int(step, isid)?;
                        iters += 1;
                        if iters > 1_000_000 {
                            return self.err("generate loop does not terminate");
                        }
                    }
                }
                Item::GenIf { cond, then_items, then_name, else_items, else_name, line, .. } => {
                    self.line = *line;
                    genblk += 1;
                    // an `else if` chain is one construct (IEEE 1800 27.5): the chosen branch is a
                    // scope of this one, with the chain's genblk number
                    let (mut cond, mut then_items, mut then_name, mut else_items, mut else_name) =
                        (cond, then_items, then_name, else_items, else_name);
                    loop {
                        let c = self.const_eval(cond, sid)?.is_true();
                        if c {
                            self.gen_scope(then_items, then_name, genblk, sid)?;
                            break;
                        }
                        if let ([Item::GenIf { cond: c2, then_items: t2, then_name: n2, else_items: e2, else_name: en2, line: l2, .. }], None) =
                            (&else_items[..], else_name)
                        {
                            self.line = *l2;
                            (cond, then_items, then_name, else_items, else_name) = (c2, t2, n2, e2, en2);
                            continue;
                        }
                        self.gen_scope(else_items, else_name, genblk, sid)?;
                        break;
                    }
                }
                Item::GenCase { expr, arms, line, .. } => {
                    self.line = *line;
                    genblk += 1;
                    let v = self.const_eval(expr, sid)?;
                    let mut chosen = None;
                    for (labels, nm, its) in arms {
                        if labels.is_empty() {
                            if chosen.is_none() {
                                chosen = Some((its, nm));
                            }
                            continue;
                        }
                        let mut hit = false;
                        for l in labels {
                            let lv = self.const_eval(l, sid)?;
                            let w = lv.w.max(v.w);
                            if lv.resize(w, lv.signed).a == v.resize(w, v.signed).a {
                                hit = true;
                            }
                        }
                        if hit {
                            chosen = Some((its, nm));
                            break;
                        }
                    }
                    if let Some((its, nm)) = chosen {
                        self.gen_scope(its, nm, genblk, sid)?;
                    }
                }
                Item::GenBlock { name, items: its, line } => {
                    self.line = *line;
                    genblk += 1;
                    self.gen_scope(its, name, genblk, sid)?;
                }
                Item::Gate { kind, terms, line, dims, strength, delay, .. } => {
                    self.line = *line;
                    self.nets.cur_strength = *strength;
                    self.nets.cur_delay = delay.clone();
                    let r = self.gate_items(kind, terms, dims, sid, *line);
                    self.nets.cur_strength = None;
                    self.nets.cur_delay = None;
                    r?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn gen_scope(&mut self, items: &'a [Item], name: &Option<String>, n: usize, sid: usize) -> R<()> {
        if items.is_empty() {
            return Ok(());
        }
        // a conditional that is the whole block: its blocks belong to this scope
        if let (None, [Item::GenIf { direct: true, .. } | Item::GenCase { direct: true, .. }]) = (name, items) {
            return self.elab_items(items, sid, false, &HashMap::new(), None);
        }
        let bname = name.clone().unwrap_or_else(|| format!("genblk{n}"));
        let path = format!("{}.{}", self.scopes[sid].name, bname);
        let file = self.file.clone();
        let child = self.new_scope(path, Some(sid), file);
        self.scopes[sid].children.insert(bname, child);
        self.elab_items(items, child, false, &HashMap::new(), None)
    }
}
