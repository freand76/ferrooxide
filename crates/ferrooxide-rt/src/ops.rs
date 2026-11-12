//! 4-state operations on values stored in the flat memory.
//! Every function has a ≤64-bit fast path; wide values take a generic path.

use crate::{nw, tmask, BinOp, CmpOp, RedOp, ShOp, UnOp};

#[inline(always)]
pub fn mask(w: u32) -> u64 {
    if w >= 64 {
        !0
    } else {
        (1u64 << w) - 1
    }
}

/// `len` bits (≤64) from bit `bit` of the words starting at `base` (bit may run past the value
/// only if the caller guarantees the words exist).
#[inline]
pub fn get_bits(m: &[u64], base: usize, bit: u32, len: u32) -> u64 {
    if len == 0 {
        return 0;
    }
    let index = base + (bit / 64) as usize;
    let offset = bit % 64;
    let mut v = m[index] >> offset;
    if offset != 0 && offset + len > 64 {
        v |= m[index + 1] << (64 - offset);
    }
    v & mask(len)
}

#[inline]
pub fn set_bits(m: &mut [u64], base: usize, bit: u32, len: u32, value: u64) {
    if len == 0 {
        return;
    }
    let index = base + (bit / 64) as usize;
    let offset = bit % 64;
    let mk = mask(len);
    let value = value & mk;
    m[index] = (m[index] & !(mk << offset)) | (value << offset);
    if offset != 0 && offset + len > 64 {
        let high = 64 - offset;
        m[index + 1] = (m[index + 1] & !(mk >> high)) | (value >> high);
    }
}

#[inline]
pub fn set_x(m: &mut [u64], d: usize, w: u32) {
    let n = nw(w);
    for i in 0..n {
        let v = if i == n - 1 { tmask(w) } else { !0 };
        m[d + i] = v;
        m[d + n + i] = v;
    }
}

#[inline]
pub fn is_known(m: &[u64], s: usize, w: u32) -> bool {
    let n = nw(w);
    m[s + n..s + 2 * n].iter().all(|&x| x == 0)
}

/// Truthiness of a value: Some(true) if any bit is a known 1, Some(false) if all known 0, None if unknown.
#[inline]
pub fn truth(m: &[u64], s: usize, w: u32) -> Option<bool> {
    if w <= 64 {
        let (a, b) = (m[s], m[s + 1]);
        if a & !b != 0 {
            return Some(true);
        }
        return if b == 0 { Some(false) } else { None };
    }
    let n = nw(w);
    let mut unk = false;
    for i in 0..n {
        let (a, b) = (m[s + i], m[s + n + i]);
        if a & !b != 0 {
            return Some(true);
        }
        unk |= b != 0;
    }
    if unk {
        None
    } else {
        Some(false)
    }
}

/// Read a value as u64 (low 64 bits), None if any bit unknown.
#[inline]
pub fn read_u64(m: &[u64], s: usize, w: u32) -> Option<u64> {
    if w <= 64 {
        if m[s + 1] != 0 {
            return None;
        }
        return Some(m[s]);
    }
    if !is_known(m, s, w) {
        return None;
    }
    Some(m[s])
}

/// Read an index value as i64 (sign-extended if signed). Values that do not fit saturate.
#[inline]
pub fn read_index(m: &[u64], s: usize, w: u32, signed: bool) -> Option<i64> {
    if w <= 64 {
        if m[s + 1] != 0 {
            return None;
        }
        let v = m[s];
        if signed && w < 64 {
            let sh = 64 - w;
            return Some(((v << sh) as i64) >> sh);
        }
        if !signed && v > i64::MAX as u64 {
            return Some(i64::MAX);
        }
        return Some(v as i64);
    }
    if !is_known(m, s, w) {
        return None;
    }
    let n = nw(w);
    let neg = signed && (m[s + n - 1] >> ((w - 1) % 64)) & 1 == 1;
    let hi_zero = m[s + 1..s + n].iter().enumerate().all(|(i, &x)| {
        let x = if i + 1 == n - 1 { x | !tmask(w) } else { x };
        if neg {
            x == !0
        } else {
            x == 0
        }
    });
    if neg {
        return Some(if hi_zero { m[s] as i64 } else { i64::MIN });
    }
    Some(if hi_zero && m[s] <= i64::MAX as u64 { m[s] as i64 } else { i64::MAX })
}

#[inline(always)]
fn sext(v: u64, w: u32) -> i64 {
    if w >= 64 {
        v as i64
    } else {
        let sh = 64 - w;
        ((v << sh) as i64) >> sh
    }
}

// ---------------------------------------------------------------------------------------------
// wide helpers (plane slices as Vec<u64>)

fn load(m: &[u64], s: usize, w: u32) -> (Vec<u64>, Vec<u64>) {
    let n = nw(w);
    (m[s..s + n].to_vec(), m[s + n..s + 2 * n].to_vec())
}

fn store_plane(m: &mut [u64], d: usize, w: u32, a: &[u64], b: &[u64]) {
    let n = nw(w);
    for i in 0..n {
        let mk = if i == n - 1 { tmask(w) } else { !0 };
        m[d + i] = a.get(i).copied().unwrap_or(0) & mk;
        m[d + n + i] = b.get(i).copied().unwrap_or(0) & mk;
    }
}

fn w_add(a: &[u64], b: &[u64]) -> Vec<u64> {
    let mut r = vec![0; a.len()];
    let mut c = 0u64;
    for i in 0..a.len() {
        let (s1, o1) = a[i].overflowing_add(b[i]);
        let (s2, o2) = s1.overflowing_add(c);
        r[i] = s2;
        c = (o1 | o2) as u64;
    }
    r
}

fn w_neg(a: &[u64]) -> Vec<u64> {
    let inv: Vec<u64> = a.iter().map(|x| !x).collect();
    let mut one = vec![0; a.len()];
    one[0] = 1;
    w_add(&inv, &one)
}

fn w_sub(a: &[u64], b: &[u64]) -> Vec<u64> {
    w_add(a, &w_neg(b))
}

fn w_mul(a: &[u64], b: &[u64]) -> Vec<u64> {
    let n = a.len();
    let mut r = vec![0u64; n];
    for i in 0..n {
        let mut carry = 0u128;
        for j in 0..n - i {
            let t = r[i + j] as u128 + (a[i] as u128) * (b[j] as u128) + carry;
            r[i + j] = t as u64;
            carry = t >> 64;
        }
    }
    r
}

fn w_is_zero(a: &[u64]) -> bool {
    a.iter().all(|&x| x == 0)
}

fn w_cmp(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    std::cmp::Ordering::Equal
}

/// unsigned long division: (quotient, remainder)
fn w_divmod(a: &[u64], b: &[u64]) -> (Vec<u64>, Vec<u64>) {
    let n = a.len();
    let mut q = vec![0u64; n];
    let mut r = vec![0u64; n];
    for bit in (0..n * 64).rev() {
        // r = r << 1 | a[bit]
        let mut carry = (a[bit / 64] >> (bit % 64)) & 1;
        for x in r.iter_mut() {
            let nc = *x >> 63;
            *x = (*x << 1) | carry;
            carry = nc;
        }
        if w_cmp(&r, b) != std::cmp::Ordering::Less {
            r = w_sub(&r, b);
            q[bit / 64] |= 1 << (bit % 64);
        }
    }
    (q, r)
}

fn w_sign(a: &[u64], w: u32) -> bool {
    (a[(w as usize - 1) / 64] >> ((w - 1) % 64)) & 1 == 1
}

/// sign-extend top word in place (so two's complement ops work on full words)
fn w_sext(a: &mut [u64], w: u32) {
    if w_sign(a, w) {
        let n = a.len();
        a[n - 1] |= !tmask(w);
    }
}

// ---------------------------------------------------------------------------------------------

pub fn resize(m: &mut [u64], d: usize, s: usize, sw: u32, dw: u32, sign: bool) {
    if sw <= 64 && dw <= 64 {
        let (mut a, mut b) = (m[s], m[s + 1]);
        if sign && sw < dw {
            a = sext(a, sw) as u64;
            b = sext(b, sw) as u64;
        }
        let mk = mask(dw);
        m[d] = a & mk;
        m[d + 1] = b & mk;
        return;
    }
    let ns = nw(sw);
    let nd = nw(dw);
    let (msb_a, msb_b) = if sign {
        let i = (sw as usize - 1) / 64;
        let sh = (sw - 1) % 64;
        ((m[s + i] >> sh) & 1, (m[s + ns + i] >> sh) & 1)
    } else {
        (0, 0)
    };
    let mut out_a = vec![0u64; nd];
    let mut out_b = vec![0u64; nd];
    for i in 0..nd {
        for (plane, out, msb) in [(0usize, &mut out_a, msb_a), (1usize, &mut out_b, msb_b)] {
            let fill = if msb == 1 { !0u64 } else { 0 };
            let v = if i < ns {
                let mut v = m[s + plane * ns + i];
                if i == ns - 1 {
                    v |= fill & !tmask(sw);
                }
                v
            } else {
                fill
            };
            out[i] = v;
        }
    }
    store_plane(m, d, dw, &out_a, &out_b);
}

pub fn unop(m: &mut [u64], op: UnOp, d: usize, s: usize, w: u32) {
    if w <= 64 {
        let (a, b) = (m[s], m[s + 1]);
        let mk = mask(w);
        match op {
            UnOp::Not => {
                m[d] = (!a | b) & mk;
                m[d + 1] = b;
            }
            UnOp::Neg => {
                if b != 0 {
                    m[d] = mk;
                    m[d + 1] = mk;
                } else {
                    m[d] = a.wrapping_neg() & mk;
                    m[d + 1] = 0;
                }
            }
        }
        return;
    }
    let (a, b) = load(m, s, w);
    match op {
        UnOp::Not => {
            let ra: Vec<u64> = a.iter().zip(&b).map(|(x, y)| !x | y).collect();
            store_plane(m, d, w, &ra, &b);
        }
        UnOp::Neg => {
            if !w_is_zero(&b) {
                set_x(m, d, w);
            } else {
                let r = w_neg(&a);
                store_plane(m, d, w, &r, &[]);
            }
        }
    }
}

pub fn redop(m: &mut [u64], op: RedOp, d: usize, s: usize, w: u32) {
    // compute (any_known1, any_known0, any_x, parity)
    let n = nw(w);
    let (mut k1, mut k0, mut x, mut par) = (false, false, false, 0u32);
    for i in 0..n {
        let mk = if i == n - 1 { tmask(w) } else { !0 };
        let (a, b) = (m[s + i], m[s + n + i]);
        k1 |= a & !b != 0;
        k0 |= !a & !b & mk != 0;
        x |= b != 0;
        par ^= (a & !b).count_ones();
    }
    // result as (value, unknown)
    let (v, u) = match op {
        RedOp::And | RedOp::Nand => {
            if k0 {
                (0, 0)
            } else if x {
                (1, 1)
            } else {
                (1, 0)
            }
        }
        RedOp::Or | RedOp::Nor | RedOp::Bool => {
            if k1 {
                (1, 0)
            } else if x {
                (1, 1)
            } else {
                (0, 0)
            }
        }
        RedOp::Xor | RedOp::Xnor => {
            if x {
                (1, 1)
            } else {
                ((par & 1) as u64, 0)
            }
        }
    };
    let inv = matches!(op, RedOp::Nand | RedOp::Nor | RedOp::Xnor);
    m[d] = if u == 0 && inv { v ^ 1 } else { v };
    m[d + 1] = u;
}

#[inline(always)]
fn bitwise(op: BinOp, va: u64, xa: u64, vb: u64, xb: u64, mk: u64) -> (u64, u64) {
    match op {
        BinOp::And => {
            let k0 = (!va & !xa) | (!vb & !xb);
            let k1 = (va & !xa) & (vb & !xb);
            let rx = !(k0 | k1) & mk;
            (k1 | rx, rx)
        }
        BinOp::Or => {
            let k1 = (va & !xa) | (vb & !xb);
            let k0 = (!va & !xa) & (!vb & !xb);
            let rx = !(k0 | k1) & mk;
            (k1 | rx, rx)
        }
        BinOp::Xor => {
            let rx = xa | xb;
            ((va ^ vb) | rx, rx)
        }
        BinOp::Xnor => {
            let rx = xa | xb;
            ((!(va ^ vb) | rx) & mk, rx)
        }
        _ => unreachable!(),
    }
}

pub fn binop(m: &mut [u64], op: BinOp, d: usize, a: usize, b: usize, w: u32) {
    if w <= 64 {
        let (va, xa, vb, xb) = (m[a], m[a + 1], m[b], m[b + 1]);
        let mk = mask(w);
        if matches!(op, BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Xnor) {
            let (r, x) = bitwise(op, va, xa, vb, xb, mk);
            m[d] = r;
            m[d + 1] = x;
            return;
        }
        if (xa | xb) != 0 {
            m[d] = mk;
            m[d + 1] = mk;
            return;
        }
        let r = match op {
            BinOp::Add => va.wrapping_add(vb),
            BinOp::Sub => va.wrapping_sub(vb),
            BinOp::Mul => va.wrapping_mul(vb),
            BinOp::Div | BinOp::Mod | BinOp::DivS | BinOp::ModS if vb == 0 => {
                m[d] = mk;
                m[d + 1] = mk;
                return;
            }
            BinOp::Div => va / vb,
            BinOp::Mod => va % vb,
            BinOp::DivS => sext(va, w).wrapping_div(sext(vb, w)) as u64,
            BinOp::ModS => sext(va, w).wrapping_rem(sext(vb, w)) as u64,
            _ => unreachable!(),
        };
        m[d] = r & mk;
        m[d + 1] = 0;
        return;
    }
    let n = nw(w);
    if matches!(op, BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Xnor) {
        for i in 0..n {
            let mk = if i == n - 1 { tmask(w) } else { !0 };
            let (r, x) = bitwise(op, m[a + i], m[a + n + i], m[b + i], m[b + n + i], mk);
            m[d + i] = r;
            m[d + n + i] = x;
        }
        return;
    }
    let (va, xa) = load(m, a, w);
    let (vb, xb) = load(m, b, w);
    if !w_is_zero(&xa) || !w_is_zero(&xb) {
        set_x(m, d, w);
        return;
    }
    let r = match op {
        BinOp::Add => w_add(&va, &vb),
        BinOp::Sub => w_sub(&va, &vb),
        BinOp::Mul => w_mul(&va, &vb),
        BinOp::Div | BinOp::Mod => {
            if w_is_zero(&vb) {
                set_x(m, d, w);
                return;
            }
            let (q, r) = w_divmod(&va, &vb);
            if op == BinOp::Div {
                q
            } else {
                r
            }
        }
        BinOp::DivS | BinOp::ModS => {
            if w_is_zero(&vb) {
                set_x(m, d, w);
                return;
            }
            let (mut sa, mut sb) = (va.clone(), vb.clone());
            w_sext(&mut sa, w);
            w_sext(&mut sb, w);
            let na = w_sign(&va, w);
            let nb = w_sign(&vb, w);
            if na {
                sa = w_neg(&sa);
            }
            if nb {
                sb = w_neg(&sb);
            }
            let (q, r) = w_divmod(&sa, &sb);
            if op == BinOp::DivS {
                if na != nb {
                    w_neg(&q)
                } else {
                    q
                }
            } else if na {
                w_neg(&r)
            } else {
                r
            }
        }
        _ => unreachable!(),
    };
    store_plane(m, d, w, &r, &[]);
}

pub fn cmpop(m: &mut [u64], op: CmpOp, d: usize, a: usize, b: usize, w: u32) {
    let (v, x) = if w <= 64 {
        let (va, xa, vb, xb) = (m[a], m[a + 1], m[b], m[b + 1]);
        cmp_small(op, va, xa, vb, xb, w)
    } else {
        cmp_wide(m, op, a, b, w)
    };
    m[d] = v;
    m[d + 1] = x;
}

#[inline(always)]
fn cmp_small(op: CmpOp, va: u64, xa: u64, vb: u64, xb: u64, w: u32) -> (u64, u64) {
    let unk = (xa | xb) != 0;
    let b2 = |c: bool| (c as u64, 0u64);
    match op {
        CmpOp::Eq | CmpOp::Ne => {
            let known_diff = (va ^ vb) & !xa & !xb != 0;
            let r = if known_diff {
                Some(false)
            } else if unk {
                None
            } else {
                Some(true)
            };
            match r {
                None => (1, 1),
                Some(e) => b2(e == (op == CmpOp::Eq)),
            }
        }
        CmpOp::CaseEq => b2(va == vb && xa == xb),
        CmpOp::CaseNe => b2(!(va == vb && xa == xb)),
        CmpOp::ZEq => {
            // z: a=0,b=1
            let care = !((!va & xa) | (!vb & xb));
            b2(((va ^ vb) | (xa ^ xb)) & care == 0)
        }
        CmpOp::XEq => {
            let care = !(xa | xb);
            b2((va ^ vb) & care == 0)
        }
        CmpOp::WildEq | CmpOp::WildNe => {
            let care = !xb;
            let known_diff = (va ^ vb) & care & !xa != 0;
            let r = if known_diff {
                Some(false)
            } else if xa & care != 0 {
                None
            } else {
                Some(true)
            };
            match r {
                None => (1, 1),
                Some(e) => b2(e == (op == CmpOp::WildEq)),
            }
        }
        _ => {
            if unk {
                return (1, 1);
            }
            let r = match op {
                CmpOp::Lt => va < vb,
                CmpOp::Le => va <= vb,
                CmpOp::Gt => va > vb,
                CmpOp::Ge => va >= vb,
                CmpOp::LtS => sext(va, w) < sext(vb, w),
                CmpOp::LeS => sext(va, w) <= sext(vb, w),
                CmpOp::GtS => sext(va, w) > sext(vb, w),
                CmpOp::GeS => sext(va, w) >= sext(vb, w),
                _ => unreachable!(),
            };
            b2(r)
        }
    }
}

fn cmp_wide(m: &[u64], op: CmpOp, a: usize, b: usize, w: u32) -> (u64, u64) {
    let n = nw(w);
    match op {
        CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge | CmpOp::LtS | CmpOp::LeS | CmpOp::GtS | CmpOp::GeS => {
            if !is_known(m, a, w) || !is_known(m, b, w) {
                return (1, 1);
            }
            let (va, _) = load(m, a, w);
            let (vb, _) = load(m, b, w);
            let signed = matches!(op, CmpOp::LtS | CmpOp::LeS | CmpOp::GtS | CmpOp::GeS);
            let mut ord = w_cmp(&va, &vb);
            if signed {
                let (sa, sb) = (w_sign(&va, w), w_sign(&vb, w));
                if sa != sb {
                    ord = if sa { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
                }
            }
            use std::cmp::Ordering::*;
            let r = match op {
                CmpOp::Lt | CmpOp::LtS => ord == Less,
                CmpOp::Le | CmpOp::LeS => ord != Greater,
                CmpOp::Gt | CmpOp::GtS => ord == Greater,
                _ => ord != Less,
            };
            (r as u64, 0)
        }
        _ => {
            // combine per-word results of the small comparator for equality-type ops
            let mut known_diff = false;
            let mut unk = false;
            let mut all_eq = true;
            for i in 0..n {
                let (va, xa, vb, xb) = (m[a + i], m[a + n + i], m[b + i], m[b + n + i]);
                match op {
                    CmpOp::Eq | CmpOp::Ne => {
                        known_diff |= (va ^ vb) & !xa & !xb != 0;
                        unk |= (xa | xb) != 0;
                    }
                    CmpOp::WildEq | CmpOp::WildNe => {
                        known_diff |= (va ^ vb) & !xb & !xa != 0;
                        unk |= xa & !xb != 0;
                    }
                    _ => {
                        let (r, _) = cmp_small(CmpOp::CaseEq.min_for(op), va, xa, vb, xb, 64);
                        all_eq &= r == 1;
                    }
                }
            }
            match op {
                CmpOp::Eq | CmpOp::WildEq => {
                    if known_diff {
                        (0, 0)
                    } else if unk {
                        (1, 1)
                    } else {
                        (1, 0)
                    }
                }
                CmpOp::Ne | CmpOp::WildNe => {
                    if known_diff {
                        (1, 0)
                    } else if unk {
                        (1, 1)
                    } else {
                        (0, 0)
                    }
                }
                CmpOp::CaseNe => ((!all_eq) as u64, 0),
                _ => (all_eq as u64, 0),
            }
        }
    }
}

impl CmpOp {
    /// the per-word "match" comparator for the wide path of CaseEq/CaseNe/ZEq/XEq
    fn min_for(self, op: CmpOp) -> CmpOp {
        match op {
            CmpOp::ZEq => CmpOp::ZEq,
            CmpOp::XEq => CmpOp::XEq,
            _ => self,
        }
    }
}

pub fn shift(m: &mut [u64], op: ShOp, d: usize, a: usize, w: u32, b: usize, bw: u32) {
    let amt = match read_u64(m, b, bw) {
        None => {
            set_x(m, d, w);
            return;
        }
        Some(v) => {
            if bw > 64 && !(1..nw(bw)).all(|i| m[b + i] == 0) {
                u64::MAX
            } else {
                v
            }
        }
    };
    if w <= 64 {
        let (va, xa) = (m[a], m[a + 1]);
        let mk = mask(w);
        let (r, x) = match op {
            ShOp::Shl => {
                if amt >= w as u64 {
                    (0, 0)
                } else {
                    ((va << amt) & mk, (xa << amt) & mk)
                }
            }
            ShOp::Shr => {
                if amt >= w as u64 {
                    (0, 0)
                } else {
                    (va >> amt, xa >> amt)
                }
            }
            ShOp::AShr => {
                let sh = amt.min(63) as u32;
                let (sa, sx) = (sext(va, w), sext(xa, w));
                if amt >= w as u64 {
                    ((sa >> 63) as u64 & mk, (sx >> 63) as u64 & mk)
                } else {
                    (((sa >> sh) as u64) & mk, ((sx >> sh) as u64) & mk)
                }
            }
        };
        m[d] = r;
        m[d + 1] = x;
        return;
    }
    let n = nw(w);
    let (va, xa) = load(m, a, w);
    let sh = |p: &[u64], fill: u64| -> Vec<u64> {
        let mut out = vec![0u64; n];
        for bit in 0..w as u64 {
            let src: i128 = match op {
                ShOp::Shl => bit as i128 - amt as i128,
                _ => bit as i128 + amt as i128,
            };
            let v = if src < 0 {
                0
            } else if src >= w as i128 {
                fill
            } else {
                (p[(src / 64) as usize] >> (src % 64)) & 1
            };
            out[(bit / 64) as usize] |= v << (bit % 64);
        }
        out
    };
    let (fa, fx) = if op == ShOp::AShr { (w_sign(&va, w) as u64, w_sign(&xa, w) as u64) } else { (0, 0) };
    let ra = sh(&va, fa);
    let rx = sh(&xa, fx);
    store_plane(m, d, w, &ra, &rx);
}

pub fn pow(m: &mut [u64], d: usize, a: usize, b: usize, w: u32, bw: u32, signed: bool) {
    if !is_known(m, a, w) || !is_known(m, b, bw) {
        set_x(m, d, w);
        return;
    }
    let n = nw(w);
    let base: Vec<u64> = m[a..a + n].to_vec();
    let e_neg = signed && bw <= 64 && sext(m[b], bw) < 0;
    let mk = tmask(w);
    if e_neg {
        // a ** negative: 1 if a == 1, -1 if a == -1 (alternating), 0 otherwise, X if a == 0
        let mut av = base.clone();
        w_sext(&mut av, w);
        let one = {
            let mut v = vec![0u64; n];
            v[0] = 1;
            v
        };
        let minus_one = vec![!0u64; n];
        let r = if w_is_zero(&base) {
            set_x(m, d, w);
            return;
        } else if av == one {
            one
        } else if av == minus_one {
            if m[b] & 1 == 1 {
                minus_one
            } else {
                one
            }
        } else {
            vec![0u64; n]
        };
        store_plane(m, d, w, &r, &[]);
        return;
    }
    let mut e = if bw <= 64 { m[b] } else { m[b] };
    let mut result = vec![0u64; n];
    result[0] = 1;
    let mut bb = base;
    while e > 0 {
        if e & 1 == 1 {
            result = w_mul(&result, &bb);
        }
        bb = w_mul(&bb, &bb);
        e >>= 1;
    }
    let _ = mk;
    store_plane(m, d, w, &result, &[]);
}

pub fn mux(m: &mut [u64], d: usize, c: usize, cw: u32, t: usize, f: usize, w: u32) {
    match truth(m, c, cw) {
        Some(true) => copy(m, d, t, w),
        Some(false) => copy(m, d, f, w),
        None => {
            let n = nw(w);
            for i in 0..n {
                let (ta, tb, fa, fb) = (m[t + i], m[t + n + i], m[f + i], m[f + n + i]);
                let x = tb | fb | (ta ^ fa);
                m[d + i] = ta | x;
                m[d + n + i] = x;
            }
        }
    }
}

#[inline]
pub fn copy(m: &mut [u64], d: usize, s: usize, w: u32) {
    if w <= 64 {
        m[d] = m[s];
        m[d + 1] = m[s + 1];
    } else {
        let n = 2 * nw(w);
        m.copy_within(s..s + n, d);
    }
}

/// Extract `w` bits starting at signed bit offset `off` of the `sw`-bit value at `s`.
/// Bits outside [0, sw) read as X.
pub fn extract(m: &mut [u64], d: usize, s: usize, sw: u32, off: i64, w: u32) {
    if off >= 0 && off + w as i64 <= sw as i64 {
        let off = off as u32;
        if sw <= 64 {
            let mk = mask(w);
            m[d] = (m[s] >> off) & mk;
            m[d + 1] = (m[s + 1] >> off) & mk;
            return;
        }
        let ns = nw(sw);
        let nd = nw(w);
        let mut tmp = [0u64; 2];
        for i in 0..nd {
            let len = (w - 64 * i as u32).min(64);
            let bit = off + 64 * i as u32;
            tmp[0] = get_bits(m, s, bit, len);
            tmp[1] = get_bits(m, s + ns, bit, len);
            m[d + i] = tmp[0];
            m[d + nd + i] = tmp[1];
        }
        return;
    }
    // partially / fully out of range: bitwise
    let ns = nw(sw);
    let nd = nw(w);
    for i in 0..nd {
        m[d + i] = 0;
        m[d + nd + i] = 0;
    }
    for i in 0..w as i64 {
        let src = off + i;
        let (a, b) = if src < 0 || src >= sw as i64 {
            (1, 1)
        } else {
            let src = src as usize;
            ((m[s + src / 64] >> (src % 64)) & 1, (m[s + ns + src / 64] >> (src % 64)) & 1)
        };
        let i = i as usize;
        m[d + i / 64] |= a << (i % 64);
        m[d + nd + i / 64] |= b << (i % 64);
    }
}

pub fn concat(m: &mut [u64], d: usize, w: u32, parts: &[(u32, u32)]) {
    if w <= 64 {
        let (mut a, mut b) = (0u64, 0u64);
        let mut off = 0u32;
        for &(s, pw) in parts.iter().rev() {
            let s = s as usize;
            if off < 64 {
                a |= m[s] << off;
                b |= m[s + 1] << off;
            }
            off += pw;
        }
        let mk = mask(w);
        m[d] = a & mk;
        m[d + 1] = b & mk;
        return;
    }
    let nd = nw(w);
    for i in 0..2 * nd {
        m[d + i] = 0;
    }
    let mut off = 0u32;
    for &(s, pw) in parts.iter().rev() {
        let s = s as usize;
        let ns = nw(pw);
        for i in 0..ns {
            let len = (pw - 64 * i as u32).min(64);
            let (a, b) = (m[s + i], m[s + ns + i]);
            set_bits(m, d, off + 64 * i as u32, len, a);
            set_bits(m, d + nd, off + 64 * i as u32, len, b);
        }
        off += pw;
    }
}

/// Deposit the `sw`-bit value at `s` into bits [off, off+sw) of the `dw`-bit value at `d`.
pub fn insert(m: &mut [u64], d: usize, dw: u32, s: usize, sw: u32, off: u32) {
    let (nd, ns) = (nw(dw), nw(sw));
    let len = sw.min(dw.saturating_sub(off));
    for plane in 0..2 {
        let mut i = 0;
        while i < len {
            let l = (len - i).min(64);
            let v = get_bits(m, s + plane * ns, i, l);
            set_bits(m, d + plane * nd, off + i, l, v);
            i += l;
        }
    }
}

pub fn clog2(m: &mut [u64], d: usize, s: usize, w: u32) {
    if !is_known(m, s, w) {
        m[d] = 0xffff_ffff;
        m[d + 1] = 0xffff_ffff;
        return;
    }
    let n = nw(w);
    // ceil(log2(v)) = bit length of (v - 1)
    let v: Vec<u64> = m[s..s + n].to_vec();
    let r = if w_is_zero(&v) {
        0
    } else {
        let mut one = vec![0u64; n];
        one[0] = 1;
        let vm1 = w_sub(&v, &one);
        let mut bl = 0u64;
        for i in (0..n).rev() {
            if vm1[i] != 0 {
                bl = 64 * i as u64 + 64 - vm1[i].leading_zeros() as u64;
                break;
            }
        }
        bl
    };
    m[d] = r;
    m[d + 1] = 0;
}
