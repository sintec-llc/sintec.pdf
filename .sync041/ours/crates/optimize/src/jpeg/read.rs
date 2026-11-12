//! Reading a sequential Huffman-coded JPEG into coefficient blocks (T.81 Annex B, F.2).
//!
//! Strict: anything this doesn't fully understand (progressive or arithmetic coding, 12-bit
//! samples, a DNL marker, a scan that runs out of data, a code no table has) is refused with
//! `None`, so the caller keeps the original.

use super::{Component, Jpeg, MAX_BLOCKS};

/// A Huffman table for decoding (T.81 F.2.2.3), with an 8-bit lookahead.
#[derive(Clone)]
struct Huffman {
    /// For each 8-bit prefix: code length (0 if longer than 8 bits) and symbol.
    fast: [(u8, u8); 256],
    /// For each length 1–16: the largest code (-1 when none), and where its symbols start.
    maxcode: [i32; 17],
    mincode: [i32; 17],
    valptr: [usize; 17],
    vals: Vec<u8>,
}

impl Huffman {
    /// A table from the code counts per length and the symbols (T.81 Annex C).
    fn new(counts: &[u8; 16], vals: Vec<u8>) -> Option<Huffman> {
        let total: usize = counts.iter().map(|c| usize::from(*c)).sum();
        if total != vals.len() || total == 0 || total > 256 {
            return None;
        }
        let mut t = Huffman { fast: [(0, 0); 256], maxcode: [-1; 17], mincode: [0; 17], valptr: [0; 17], vals };
        let (mut code, mut k) = (0u32, 0usize);
        for len in 1..=16usize {
            let n = u32::from(counts[len - 1]);
            if n > 0 {
                t.valptr[len] = k;
                t.mincode[len] = code as i32;
                for _ in 0..n {
                    if len <= 8 {
                        let shift = 8 - len;
                        let sym = *t.vals.get(k)?;
                        for low in 0..(1u32 << shift) {
                            let i = usize::try_from((code << shift) | low).ok()?;
                            *t.fast.get_mut(i)? = (len as u8, sym);
                        }
                    }
                    code += 1;
                    k += 1;
                }
                t.maxcode[len] = code as i32 - 1;
            }
            // Codes of each length must fit in it (the all-ones code may not be used, but some
            // encoders do, and it decodes unambiguously).
            if code > (1u32 << len) {
                return None;
            }
            code <<= 1;
        }
        Some(t)
    }
}

/// Bits of an entropy-coded segment, with byte stuffing removed. Reading past its end (a
/// marker or the end of the data) is recorded, not hidden.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    /// Bits in `acc`, the last `fake` of which are padding past the end of the segment.
    n: u32,
    fake: u32,
    at_marker: bool,
    overrun: bool,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Bits { data, pos, acc: 0, n: 0, fake: 0, at_marker: false, overrun: false }
    }

    fn fill(&mut self) {
        while self.n <= 56 {
            let byte = if self.at_marker {
                None
            } else {
                match self.data.get(self.pos) {
                    Some(0xFF) => match self.data.get(self.pos + 1) {
                        Some(0x00) => {
                            self.pos += 2;
                            Some(0xFF)
                        }
                        _ => None,
                    },
                    Some(b) => {
                        self.pos += 1;
                        Some(*b)
                    }
                    None => None,
                }
            };
            let b = byte.unwrap_or_else(|| {
                self.at_marker = true;
                self.fake += 8;
                0
            });
            self.acc |= u64::from(b) << (56 - self.n);
            self.n += 8;
        }
    }

    fn peek(&mut self, k: u32) -> u32 {
        if self.n < k {
            self.fill();
        }
        (self.acc >> (64 - k)) as u32
    }

    fn consume(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
        if self.n < self.fake {
            self.overrun = true;
        }
    }

    fn get(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let v = self.peek(k);
        self.consume(k);
        v
    }

    fn symbol(&mut self, t: &Huffman) -> Option<u8> {
        let (len, sym) = *t.fast.get(self.peek(8) as usize)?;
        if len > 0 {
            self.consume(u32::from(len));
            return Some(sym);
        }
        let code = self.peek(16);
        for len in 9..=16usize {
            let c = (code >> (16 - len)) as i32;
            if c <= t.maxcode[len] {
                let i = t.valptr[len] + usize::try_from(c - t.mincode[len]).ok()?;
                self.consume(len as u32);
                return t.vals.get(i).copied();
            }
        }
        None
    }

    /// A value of size category `s` (T.81 F.2.2.1 EXTEND).
    fn value(&mut self, s: u8) -> i32 {
        let s = u32::from(s);
        let v = self.get(s) as i32;
        if s > 0 && v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
    }

    /// Skip to the byte boundary and past restart marker `RST(m)`.
    fn restart(&mut self, m: u8) -> bool {
        // Only the padding of the last byte may be left unread.
        if self.n.saturating_sub(self.fake) >= 8 {
            return false;
        }
        while self.data.get(self.pos) == Some(&0xFF) && self.data.get(self.pos + 1) == Some(&0xFF) {
            self.pos += 1;
        }
        if self.data.get(self.pos) != Some(&0xFF) || self.data.get(self.pos + 1) != Some(&(0xD0 + m)) {
            return false;
        }
        self.pos += 2;
        (self.acc, self.n, self.fake, self.at_marker) = (0, 0, 0, false);
        true
    }
}

fn be16(data: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(*data.get(at)?) << 8 | usize::from(*data.get(at + 1)?))
}

/// Markers whose segments only carry metadata that no PDF reader draws: Exif and XMP (APP1),
/// Photoshop resources and IPTC (APP13), comments.
fn droppable(marker: u8) -> bool {
    matches!(marker, 0xE1 | 0xED | 0xFE)
}

/// The coefficients of a sequential Huffman-coded JPEG.
pub(crate) fn read(data: &[u8]) -> Option<Jpeg> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut jpeg: Option<Jpeg> = None;
    let mut qtables: [Option<(u8, [u16; 64])>; 4] = [None; 4];
    let mut dc: [Option<Huffman>; 4] = [None, None, None, None];
    let mut ac: [Option<Huffman>; 4] = [None, None, None, None];
    let mut markers = Vec::new();
    let mut restart = 0usize;
    let mut decoded: Vec<bool> = Vec::new();
    let mut pos = 2usize;
    loop {
        // Fill bytes (0xFF) may precede a marker.
        while data.get(pos) == Some(&0xFF) && data.get(pos + 1) == Some(&0xFF) {
            pos += 1;
        }
        if *data.get(pos)? != 0xFF {
            return None;
        }
        let marker = *data.get(pos + 1)?;
        pos += 2;
        if marker == 0xD9 {
            break;
        }
        let len = be16(data, pos)?;
        let seg = data.get(pos + 2..pos.checked_add(len)?)?;
        if len < 2 {
            return None;
        }
        pos += len;
        match marker {
            0xE0..=0xEF | 0xFE => {
                if !droppable(marker) {
                    markers.push((marker, seg.to_vec()));
                }
            }
            0xDB => {
                let mut i = 0;
                while i < seg.len() {
                    let pq = seg.get(i)? >> 4;
                    let tq = usize::from(seg.get(i)? & 15);
                    let size = if pq == 0 { 1 } else { 2 };
                    let mut t = [0u16; 64];
                    for (k, v) in t.iter_mut().enumerate() {
                        let at = i + 1 + k * size;
                        *v = if pq == 0 { u16::from(*seg.get(at)?) } else { (be16(seg, at)? as u16).max(1) };
                    }
                    if pq > 1 || tq > 3 || t.contains(&0) {
                        return None;
                    }
                    qtables[tq] = Some((pq, t));
                    i += 1 + 64 * size;
                }
            }
            0xC4 => {
                let mut i = 0;
                while i < seg.len() {
                    let (tc, th) = (seg.get(i)? >> 4, usize::from(seg.get(i)? & 15));
                    let counts: [u8; 16] = seg.get(i + 1..i + 17)?.try_into().ok()?;
                    let total: usize = counts.iter().map(|c| usize::from(*c)).sum();
                    let vals = seg.get(i + 17..i + 17 + total)?.to_vec();
                    let t = Huffman::new(&counts, vals)?;
                    match (tc, th) {
                        (0, 0..=3) => dc[th] = Some(t),
                        (1, 0..=3) => ac[th] = Some(t),
                        _ => return None,
                    }
                    i += 17 + total;
                }
            }
            0xDD => restart = be16(seg, 0)?,
            0xC0 | 0xC1 => {
                if jpeg.is_some() || *seg.first()? != 8 {
                    return None;
                }
                let height = u16::try_from(be16(seg, 1)?).ok()?;
                let width = u16::try_from(be16(seg, 3)?).ok()?;
                let nf = usize::from(*seg.get(5)?);
                if height == 0 || width == 0 || !(1..=4).contains(&nf) {
                    return None;
                }
                let mut components = Vec::with_capacity(nf);
                for k in 0..nf {
                    let c = seg.get(6 + 3 * k..9 + 3 * k)?;
                    let (id, h, v, tq) = (*c.first()?, c.get(1)? >> 4, c.get(1)? & 15, *c.get(2)?);
                    if !(1..=4).contains(&h) || !(1..=4).contains(&v) || tq > 3 || components.iter().any(|c: &Component| c.id == id) {
                        return None;
                    }
                    components.push(Component { id, h, v, tq, bw: 0, bh: 0, blocks: Vec::new() });
                }
                let mut j = Jpeg { width, height, components, qtables: [None; 4], markers: Vec::new() };
                let (hm, vm) = j.max_sampling();
                let (mx, my) = j.mcus();
                let mut total = 0usize;
                for c in &mut j.components {
                    // Fractional sampling ratios (3 against 2) aren't supported by most decoders.
                    if hm % usize::from(c.h) != 0 || vm % usize::from(c.v) != 0 {
                        return None;
                    }
                    c.bw = mx.checked_mul(usize::from(c.h))?;
                    c.bh = my.checked_mul(usize::from(c.v))?;
                    total = total.checked_add(c.bw.checked_mul(c.bh)?)?;
                }
                if total > MAX_BLOCKS {
                    return None;
                }
                for c in &mut j.components {
                    c.blocks = vec![[0i16; 64]; c.bw * c.bh];
                }
                decoded = vec![false; nf];
                jpeg = Some(j);
            }
            0xDA => {
                let j = jpeg.as_mut()?;
                pos = scan(j, seg, data, pos, restart, &dc, &ac, &mut decoded)?;
            }
            // Other frame types (progressive, lossless, arithmetic), DNL, and anything unknown.
            _ => return None,
        }
    }
    let mut j = jpeg?;
    if !decoded.iter().all(|d| *d) {
        return None;
    }
    for c in &j.components {
        qtables.get(usize::from(c.tq))?.as_ref()?;
    }
    j.qtables = qtables;
    j.markers = markers;
    Some(j)
}

/// Decode one scan whose header is `seg` and whose data starts at `pos`; returns where the next
/// marker starts.
#[allow(clippy::too_many_arguments)]
fn scan(
    j: &mut Jpeg,
    seg: &[u8],
    data: &[u8],
    pos: usize,
    restart: usize,
    dc: &[Option<Huffman>; 4],
    ac: &[Option<Huffman>; 4],
    decoded: &mut [bool],
) -> Option<usize> {
    let ns = usize::from(*seg.first()?);
    if !(1..=4).contains(&ns) || seg.len() != 4 + 2 * ns {
        return None;
    }
    // (component index, DC table, AC table)
    let mut members = Vec::with_capacity(ns);
    for k in 0..ns {
        let id = *seg.get(1 + 2 * k)?;
        let t = *seg.get(2 + 2 * k)?;
        let ci = j.components.iter().position(|c| c.id == id)?;
        let d = dc.get(usize::from(t >> 4))?.as_ref()?;
        let a = ac.get(usize::from(t & 15))?.as_ref()?;
        let done = decoded.get_mut(ci)?;
        if *done {
            return None;
        }
        *done = true;
        members.push((ci, d, a));
    }
    let (ss, se, a) = (*seg.get(1 + 2 * ns)?, *seg.get(2 + 2 * ns)?, *seg.get(3 + 2 * ns)?);
    if (ss, se, a) != (0, 63, 0) {
        return None;
    }
    // The blocks of each MCU, in coding order: (component, block index).
    let mut mcus: Vec<Vec<(usize, usize)>> = Vec::new();
    if ns == 1 {
        let (ci, ..) = *members.first()?;
        let c = j.components.get(ci)?;
        let (bx, by) = j.plain_blocks(c);
        for y in 0..by {
            for x in 0..bx {
                mcus.push(vec![(ci, y * c.bw + x)]);
            }
        }
    } else {
        let (mx, my) = j.mcus();
        let per: usize = members.iter().map(|(ci, ..)| j.components.get(*ci).map_or(0, |c| usize::from(c.h) * usize::from(c.v))).sum();
        if per > 10 {
            return None;
        }
        for y in 0..my {
            for x in 0..mx {
                let mut m = Vec::with_capacity(per);
                for (ci, ..) in &members {
                    let c = j.components.get(*ci)?;
                    let (h, v) = (usize::from(c.h), usize::from(c.v));
                    for by in 0..v {
                        for bx in 0..h {
                            m.push((*ci, (y * v + by) * c.bw + x * h + bx));
                        }
                    }
                }
                mcus.push(m);
            }
        }
    }
    let mut bits = Bits::new(data, pos);
    let mut pred = [0i32; 4];
    let mut next_rst = 0u8;
    for (i, mcu) in mcus.into_iter().enumerate() {
        if restart > 0 && i > 0 && i % restart == 0 {
            if !bits.restart(next_rst) {
                return None;
            }
            next_rst = (next_rst + 1) & 7;
            pred = [0; 4];
        }
        for (ci, bi) in mcu {
            let (slot, d, a) = members.iter().enumerate().find(|(_, m)| m.0 == ci).map(|(s, m)| (s, m.1, m.2))?;
            let mut block = [0i16; 64];
            let s = bits.symbol(d)?;
            if s > 11 {
                return None;
            }
            let p = pred.get_mut(slot)?;
            *p = p.checked_add(bits.value(s))?;
            block[0] = i16::try_from(*p).ok()?;
            let mut k = 1usize;
            while k < 64 {
                let rs = bits.symbol(a)?;
                let (r, s) = (usize::from(rs >> 4), rs & 15);
                if s == 0 {
                    match r {
                        0 => break,
                        15 => {
                            k += 16;
                            continue;
                        }
                        // Runs of end-of-bands belong to progressive scans.
                        _ => return None,
                    }
                }
                k += r;
                if k > 63 || s > 10 {
                    return None;
                }
                block[k] = bits.value(s) as i16;
                k += 1;
            }
            if k > 64 {
                return None;
            }
            *j.components.get_mut(ci)?.blocks.get_mut(bi)? = block;
        }
        if bits.overrun {
            return None;
        }
    }
    if bits.overrun || bits.n.saturating_sub(bits.fake) >= 8 {
        return None;
    }
    // The next marker: skip anything left before it (some encoders pad).
    let mut p = bits.pos;
    while p + 1 < data.len() {
        if data.get(p) == Some(&0xFF) && data.get(p + 1).is_some_and(|b| *b != 0 && !(0xD0..=0xD7).contains(b) && *b != 0xFF) {
            return Some(p);
        }
        p += 1;
    }
    None
}
