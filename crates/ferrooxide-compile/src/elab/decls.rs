//! Declarations: signals from their merged declarations, ports, initial values, implicit
//! nets, views of signals.

use crate::types::Packed;
use crate::*;

/// A signal's declarations merged by name (`output [3:0] q; reg [3:0] q;`).
struct Merged<'d> {
    dir: Option<Dir>,
    kind: NetKind,
    net: NetType,
    real_net: bool,
    strength: Option<(u8, u8)>,
    delay: Option<&'d Expr>,
    signed: bool,
    range: Option<Range>,
    packed: &'d [Range],
    dims: &'d [Range],
    init: Option<&'d Expr>,
    /// declared with this type parameter
    tparam: Option<&'d str>,
    /// the declared data type
    ty: Option<&'d Rc<SvType>>,
    /// a queue or dynamic array
    heap: Option<&'d HeapKind>,
    /// of queues, dynamic or associative arrays of this kind
    inner: Option<&'d HeapKind>,
    line: u32,
    /// declared as a port without a range (`input a;`)
    scalar_port: bool,
    /// the line of a declaration without a direction that has a range
    vec_line: Option<u32>,
}

/// The evaluated shape of a declared signal.
struct Shape {
    msb: i64,
    lsb: i64,
    width: u32,
    signed: bool,
    /// the number of elements of an array (0: no array), its lowest index, a descending range
    depth: u32,
    arr_lo: i64,
    desc: bool,
    /// packed dimensions (the vector is flat)
    pk: Option<Vec<(i64, i64)>>,
    /// the dimensions of a multi-dimensional array (stored flat)
    md: Option<Vec<(i64, u32, bool)>>,
    kind: SigKind,
    is_real: bool,
    is_event: bool,
    layout: Option<Packed>,
}

impl<'a> Compiler<'a> {
    /// 2. the declarations of a scope, merged by name: the directions of its ports
    pub(crate) fn declarations(
        &mut self,
        items: &'a [Item],
        sid: usize,
        module_level: bool,
        port_conns: &HashMap<String, Option<Expr>>,
        parent_scope: Option<usize>,
    ) -> R<HashMap<String, Dir>> {
        let (order, mut merged) = self.merged_decls(items, sid)?;
        let mut port_dirs: HashMap<String, Dir> = HashMap::new();
        // the module's port list (signals named in port expressions have a direction but are
        // not ports themselves)
        let port_list = if module_level { self.hier.scope_module.get(&sid).map(|m| &m.ports) } else { None };
        for name in order {
            let mut mg = merged.remove(&name).unwrap();
            self.line = mg.line;
            if let (true, Some(l)) = (mg.scalar_port && mg.dir.is_some(), mg.vec_line) {
                if let Some((a, b)) = &mg.range {
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
            self.declare(name, mg, sid, module_level, port_conns, parent_scope)?;
        }
        Ok(port_dirs)
    }

    /// The declarations of `items`, merged by name, in the order of their first declaration.
    fn merged_decls(&mut self, items: &'a [Item], sid: usize) -> R<(Vec<String>, HashMap<String, Merged<'a>>)> {
        self.check_redeclared(items.iter().filter_map(|it| if let Item::Decl(d) = it { Some(d) } else { None }), sid)?;
        let mut order: Vec<String> = Vec::new();
        let mut merged: HashMap<String, Merged> = HashMap::new();
        for it in items {
            if let Item::Decl(d) = it {
                if d.kind() == NetKind::Genvar {
                    continue;
                }
                let scalar_port = d.dir.is_some() && d.range().is_none() && d.packed().is_empty() && d.ty.fields.is_none() && d.ty.enum_names.is_none() && matches!(d.kind(), NetKind::Wire | NetKind::Reg);
                let vec_line = (d.dir.is_none() && d.range_written).then_some(d.line);
                for dl in &d.names {
                    match merged.get_mut(&dl.name) {
                        Some(mg) => {
                            mg.scalar_port |= scalar_port;
                            mg.vec_line = mg.vec_line.or(vec_line);
                            if d.dir.is_some() {
                                mg.dir = d.dir;
                            }
                            if d.explicit_kind && d.kind() != NetKind::Wire {
                                mg.kind = d.kind();
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
                            mg.signed |= d.signed();
                            if d.range().is_some() {
                                mg.range = d.range();
                                mg.packed = d.packed();
                            }
                            if !dl.dims.is_empty() {
                                mg.dims = &dl.dims;
                            }
                            if dl.init.is_some() {
                                mg.init = dl.init.as_ref();
                            }
                            if d.type_param().is_some() {
                                mg.tparam = d.type_param();
                            }
                            mg.ty = Some(&d.ty);
                            if dl.heap.is_some() {
                                mg.heap = dl.heap.as_ref();
                                mg.inner = dl.inner.as_ref();
                            }
                        }
                        None => {
                            order.push(dl.name.clone());
                            merged.insert(
                                dl.name.clone(),
                                Merged {
                                    dir: d.dir,
                                    kind: d.kind(),
                                    net: d.net,
                                    real_net: d.real_net,
                                    strength: d.strength,
                                    delay: d.delay.as_ref(),
                                    signed: d.signed(),
                                    range: d.range(),
                                    packed: &d.packed(),
                                    dims: &dl.dims,
                                    init: dl.init.as_ref(),
                                    tparam: d.type_param(),
                                    ty: Some(&d.ty),
                                    heap: dl.heap.as_ref(),
                                    inner: dl.inner.as_ref(),
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
        Ok((order, merged))
    }

    /// The range, packed and unpacked dimensions and kind of a declared signal, evaluated.
    fn decl_shape(&mut self, name: &str, mg: &mut Merged, sid: usize) -> R<Shape> {
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
            if let Some((a, b)) = &mg.range {
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
        Ok(Shape { msb, lsb, width, signed, depth, arr_lo, desc, pk, md, kind, is_real, is_event, layout })
    }

    /// Signal `name` of scope `sid` from its merged declarations.
    fn declare(
        &mut self,
        name: String,
        mut mg: Merged<'a>,
        sid: usize,
        module_level: bool,
        port_conns: &HashMap<String, Option<Expr>>,
        parent_scope: Option<usize>,
    ) -> R<()> {
        let sh = self.decl_shape(&name, &mut mg, sid)?;
        let full = format!("{}.{}", self.scopes[sid].name, name);
        // a port of a module instance
        if let (Some(dir), Some(ps), true) = (mg.dir, parent_scope, module_level) {
            if let Some(Some(conn)) = port_conns.get(&name) {
                if mg.heap.is_none() && mg.kind != NetKind::Class {
                    return self.declare_port(name, full, mg, sh, dir, conn, ps, sid);
                }
            }
        }
        let Shape { msb, lsb, width, signed, depth, arr_lo, desc, mut pk, mut md, kind, is_real, is_event, layout } = sh;
        // a queue or dynamic array: a handle
        if let Some(h) = mg.heap {
            if mg.dir.is_some() && module_level {
                return self.err(format!("port '{name}': a queue or dynamic array port is not supported"));
            }
            let elem = self.elem_for(mg.kind, width, signed, mg.ty.map(|t| &**t), sid)?;
            let elem = self.nest_elem(elem, mg.inner, sid)?;
            let s = self.new_dyn(full, &name, elem, h, sid)?;
            // (of a type parameter: its type)
            let et = match mg.tparam.map(|tp| self.lookup(tp, sid)) {
                Some(Some(Sym::Type(t))) => Some(t),
                _ => mg.ty.cloned(),
            };
            self.dyn_enum(s, et.as_deref(), sid);
            self.dyn_layout(s, layout.as_ref());
            if let Some(init) = mg.init {
                let line = self.line;
                self.code.jobs.push(Job::Init { lhs: ExprKind::Ident(name.clone()).into(), rhs: init.clone(), scope: sid, line });
            }
            return Ok(());
        }
        // (an array of class handles: an array of 64-bit words, below)
        let mut class_arr = None;
        if mg.kind == NetKind::Class {
            // a class handle: starts as null; an initializer is assigned at time 0
            if mg.dir.is_some() {
                return self.err(format!("'{name}': a class handle port is not supported"));
            }
            let c = match mg.ty.and_then(|t| t.class.clone()) {
                Some(n) => self.class_named(&n, sid)?,
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
                self.code.jobs.push(Job::Init { lhs: ExprKind::Ident(name.clone()).into(), rhs: init.clone(), scope: sid, line });
            }
            return Ok(());
        }
        let s = self.new_signal(full, width, signed, depth, msb, lsb, arr_lo, kind);
        if depth > 0 {
            // (its array methods and assignments with queues take the paths of those)
            self.sigs.has_dyn = true;
        }
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
                let rhs = ExprKind::Num(Number { size: None, signed: false, base: 2, based: true, digits: d.into(), fill: true });
                let line = self.line;
                self.nets.cur_strength = Some((7, 7));
                self.push_job(Job::Assign { lhs: ExprKind::Ident(name.clone()).into(), rhs: rhs.into(), delay: None, scope: sid, line });
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
        Ok(())
    }

    /// Port `name` of a module instance, connected to `conn` in the parent scope `ps`: another
    /// name for the connected signal where it can be, else a signal of its own driven by
    /// (input) or driving (output) the connection.
    #[allow(clippy::too_many_arguments)]
    fn declare_port(&mut self, name: String, full: String, mg: Merged<'a>, sh: Shape, dir: Dir, conn: &Expr, ps: usize, sid: usize) -> R<()> {
        let Shape { msb, lsb, width, signed, depth, arr_lo, desc, mut pk, md, kind, is_real, is_event: _, layout } = sh;
        let mut aliased = None;
        if depth == 0 {
            if let ExprKind::Ident(pn) = &conn.kind {
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
            return Ok(());
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
            Dir::Inout if self.link_exprs(&Expr::from(ExprKind::Ident(name.clone())), sid, conn, ps, None) => {}
            Dir::Inout => self.warn(format!(
                "inout port '{name}' connected to an expression: connection ignored"
            )),
        }
        if let Some(init) = mg.init {
            self.decl_init(init, s, kind, mg.kind, is_real, depth, width, mg.strength, mg.delay, &name, sid)?;
        }
        Ok(())
    }

    /// A new name for signal `base` with its own declared range and sign.
    pub(crate) fn new_view(&mut self, base: u32, name: String, msb: i64, lsb: i64, signed: bool) -> u32 {
        let base = self.canon(base);
        let mut info = self.sig(base).clone();
        info.name = name.clone();
        info.msb = msb;
        info.lsb = lsb;
        info.signed = signed;
        let ty = match self.sig_type(base) {
            types::Type::Vec { w, .. } => types::Type::vec(w, signed),
            t => t,
        };
        let id = self.push_signal(info, ty);
        self.prog.names.push((name, id));
        self.nets.view_of.insert(id, base);
        id
    }

    /// After code generation: instructions, event controls and names refer to the signals the
    /// views stand for.
    pub(crate) fn resolve_views(&mut self) {
        if self.nets.view_of.is_empty() {
            return;
        }
        let canon: Vec<u32> = (0..self.prog.signals.len() as u32).map(|s| self.canon(s)).collect();
        for p in &mut self.prog.procs {
            for ins in &mut p.code {
                rt::instr_sigs_mut(ins, &mut |s| *s = canon[*s as usize]);
            }
        }
        for c in &mut self.prog.ctls {
            for t in &mut c.triggers {
                t.0 = canon[t.0 as usize];
            }
        }
        for n in &mut self.prog.names {
            n.1 = canon[n.1 as usize];
        }
        for (&v, _) in &self.nets.view_of {
            if self.prog.signals[v as usize].forced {
                self.prog.signals[canon[v as usize] as usize].forced = true;
            }
        }
    }

    /// The initializer of a declaration: a continuous assignment for a net, the start value
    /// for a variable.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decl_init(
        &mut self,
        init: &Expr,
        s: u32,
        kind: SigKind,
        nkind: NetKind,
        is_real: bool,
        depth: u32,
        width: u32,
        strength: Option<(u8, u8)>,
        delay: Option<&Expr>,
        name: &str,
        sid: usize,
    ) -> R<()> {
        if let (0, Some(&id)) = (depth, self.enums.sigs.get(&s)) {
            self.check_enum(id, init, sid)?;
        }
        if kind == SigKind::Wire {
            self.nets.cur_strength = strength;
            self.push_job(Job::Assign {
                lhs: ExprKind::Ident(name.to_string()).into(),
                rhs: init.clone(),
                delay: delay.cloned(),
                scope: sid,
                line: self.line,
            });
            self.nets.cur_strength = None;
            return Ok(());
        }
        // an initializer that is not a constant expression (6.8: it need not be one) is
        // assigned at time 0, before the processes start
        if self.decl_init_const(init, s, nkind, is_real, depth, width, sid).is_err() {
            let line = self.line;
            self.code.jobs.push(Job::Init { lhs: ExprKind::Ident(name.to_string()).into(), rhs: init.clone(), scope: sid, line });
        }
        Ok(())
    }

    /// The initial value of variable `s` from a constant initializer, set in memory.
    #[allow(clippy::too_many_arguments)]
    fn decl_init_const(&mut self, init: &Expr, s: u32, nkind: NetKind, is_real: bool, depth: u32, width: u32, sid: usize) -> R<()> {
        if nkind == NetKind::String {
            // the initial text
            let text = match &init.kind {
                ExprKind::Str(t) => t.chars().map(|c| c as u32 as u8).filter(|&c| c != 0).collect(),
                _ => {
                    let c = self.const_eval(init, sid)?;
                    let mut v = Vec::new();
                    for k in (0..c.w.div_ceil(8)).rev() {
                        let ch = (c.a[(8 * k / 64) as usize] >> (8 * k % 64)) as u8;
                        if ch != 0 {
                            v.push(ch);
                        }
                    }
                    v
                }
            };
            let h = self.intern(&text);
            let slot = self.sig(s).slot as usize;
            self.prog.mem[slot] = h;
            self.prog.mem[slot + 1] = 0;
            self.code.init_touch.push(s);
        } else if is_real {
            let r = self.const_real(init, sid)?;
            let slot = self.sig(s).slot as usize;
            self.prog.mem[slot] = r.to_bits();
            self.prog.mem[slot + 1] = 0;
            self.code.init_touch.push(s);
        } else {
            if depth == 0 {
                self.code.init_touch.push(s);
            }
            // an assignment pattern for the variable's type
            let layout = if depth == 0 { self.layout(s) } else { None };
            let r = self.with_target(init, layout.as_deref(), sid)?;
            let init = r.as_ref().unwrap_or(init);
            let c = self.const_eval(init, sid)?;
            let c = c.resize(width, c.signed);
            let slot = self.sig(s).slot as usize;
            let n = rt::nw(width);
            for i in 0..n {
                self.prog.mem[slot + i] = c.a[i];
                self.prog.mem[slot + n + i] = c.b[i];
            }
        }
        Ok(())
    }

    /// An implicitly declared net (`default_nettype; `none` makes it an error).
    pub(crate) fn implicit_net(&mut self, n: &str, width: u32, sid: usize) -> R<u32> {
        let nt = self.scopes[sid].nettype.clone();
        let net = match nt.as_deref() {
            None => NetType::Wire,
            Some("none") => return self.err(format!("undeclared identifier '{n}' (`default_nettype none)")),
            Some("wand" | "triand") => NetType::Wand,
            Some("wor" | "trior") => NetType::Wor,
            Some("tri0") => NetType::Tri0,
            Some("tri1") => NetType::Tri1,
            Some("supply0") => NetType::Supply0,
            Some("supply1") => NetType::Supply1,
            Some(_) => NetType::Wire,
        };
        let full = format!("{}.{}", self.scopes[sid].name, n);
        let s = self.new_signal(full, width, false, 0, width as i64 - 1, 0, 0, SigKind::Wire);
        self.scopes[sid].syms.insert(n.to_string(), Sym::Sig(s));
        if net != NetType::Wire {
            self.nets.net_types.insert(s, net);
        }
        Ok(s)
    }
}
