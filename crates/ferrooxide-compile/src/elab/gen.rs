//! Generate constructs: loops, conditionals, case and blocks (IEEE 1800-2017 27).

use crate::*;

impl<'a> Compiler<'a> {
    /// A generate loop: a scope `name[i]` per value of its genvar.
    pub(super) fn gen_for(&mut self, it: &'a Item, sid: usize, genblk: &mut usize) -> R<()> {
        let Item::GenFor { init_var, init, cond, step_var, step, name, items: body, line } = it else { unreachable!() };
        self.line = *line;
        *genblk += 1;
        let bname = name.clone().unwrap_or_else(|| format!("genblk{genblk}"));
        for e in [init, cond, step] {
            self.no_specparam(e, sid)?;
        }
        let mut v = self.const_int(init, sid)?;
        if name.is_some() {
            self.scope_name_free(&bname, &format!("{bname}[{v}]"), sid)?;
        }
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
        Ok(())
    }

    /// A generate `if` (with its `else if` chain): the chosen branch's scope.
    pub(super) fn gen_if(&mut self, it: &'a Item, sid: usize, genblk: &mut usize) -> R<()> {
        let Item::GenIf { cond, then_items, then_name, else_items, else_name, line, .. } = it else { unreachable!() };
        self.line = *line;
        *genblk += 1;
        // an `else if` chain is one construct (IEEE 1800 27.5): the chosen branch is a
        // scope of this one, with the chain's genblk number
        let (mut cond, mut then_items, mut then_name, mut else_items, mut else_name) =
            (cond, then_items, then_name, else_items, else_name);
        loop {
            self.no_specparam(cond, sid)?;
            let c = self.const_eval(cond, sid)?.is_true();
            if c {
                self.gen_scope(then_items, then_name, *genblk, sid)?;
                break;
            }
            if let ([Item::GenIf { cond: c2, then_items: t2, then_name: n2, else_items: e2, else_name: en2, line: l2, .. }], None) =
                (&else_items[..], else_name)
            {
                self.line = *l2;
                (cond, then_items, then_name, else_items, else_name) = (c2, t2, n2, e2, en2);
                continue;
            }
            self.gen_scope(else_items, else_name, *genblk, sid)?;
            break;
        }
        Ok(())
    }

    /// A generate `case`: the chosen arm's scope.
    pub(super) fn gen_case(&mut self, it: &'a Item, sid: usize, genblk: &mut usize) -> R<()> {
        let Item::GenCase { expr, arms, line, .. } = it else { unreachable!() };
        self.line = *line;
        *genblk += 1;
        self.no_specparam(expr, sid)?;
        for e in arms.iter().flat_map(|a| &a.0) {
            self.no_specparam(e, sid)?;
        }
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
            self.gen_scope(its, nm, *genblk, sid)?;
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
        if name.is_some() {
            self.scope_name_free(&bname, &bname, sid)?;
        }
        let path = format!("{}.{}", self.scopes[sid].name, bname);
        let file = self.file.clone();
        let child = self.new_scope(path, Some(sid), file);
        self.scopes[sid].children.insert(bname, child);
        self.elab_items(items, child, false, &HashMap::new(), None)
    }
}
