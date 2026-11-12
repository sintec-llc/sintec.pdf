//! Writing coefficients with Huffman tables made for them (T.81 Annex K.2): one baseline scan,
//! or progressive scans by spectral selection (Annex G, first scans only, no successive
//! approximation), whichever is smaller.

use super::{Jpeg, category};

/// What a scan codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Sequential: DC and all AC coefficients.
    Sequential,
    /// Progressive, first DC scan.
    Dc,
    /// Progressive, first AC scan of the band `Ss..=Se`.
    Ac(u8, u8),
}

/// Receives the symbols and extra bits of a scan: counted in a first pass, written in a second.
trait Sink {
    fn symbol(&mut self, table: usize, symbol: u8);
    fn bits(&mut self, value: u32, n: u8);
}

/// Symbol counts per table, extra bits, and whether a value too large for 8-bit JPEG was seen.
struct Count {
    freq: Vec<[u64; 256]>,
    extra: u64,
    too_large: bool,
}

impl Count {
    fn new(tables: usize) -> Self {
        Count { freq: vec![[0; 256]; tables], extra: 0, too_large: false }
    }
}

impl Sink for Count {
    fn symbol(&mut self, table: usize, symbol: u8) {
        if let Some(f) = self.freq.get_mut(table).and_then(|f| f.get_mut(usize::from(symbol))) {
            *f += 1;
        }
    }
    fn bits(&mut self, _: u32, n: u8) {
        self.extra += u64::from(n);
        // DC differences take at most 11 bits, AC values 10, in 8-bit JPEG.
        if n > 11 {
            self.too_large = true;
        }
    }
}

/// A Huffman table for writing: the code lengths per length and the symbols (as stored in DHT),
/// and each symbol's code.
#[derive(Clone)]
struct Table {
    counts: [u8; 16],
    vals: Vec<u8>,
    code: [(u16, u8); 256],
}

impl Table {
    /// The optimal table for `freq` with codes of at most 16 bits, none of them all ones
    /// (T.81 K.2, Figures K.1 to K.4).
    fn optimal(freq: &[u64; 256]) -> Option<Table> {
        const N: usize = 257;
        let mut f = [0u64; N];
        for (d, s) in f.iter_mut().zip(freq) {
            *d = *s;
        }
        if f.iter().all(|x| *x == 0) {
            // An unused table still needs a symbol to be valid.
            f[0] = 1;
        }
        // The reserved code point: it takes the longest code, which is then dropped.
        f[N - 1] = 1;
        let mut size = [0usize; N];
        let mut others = [usize::MAX; N];
        loop {
            // The least frequent symbol (the highest on ties), then the next.
            let mut v1: Option<usize> = None;
            for (i, x) in f.iter().enumerate() {
                if *x > 0 && v1.is_none_or(|v| *x <= f[v]) {
                    v1 = Some(i);
                }
            }
            let Some(a) = v1 else { break };
            let mut v2: Option<usize> = None;
            for (i, x) in f.iter().enumerate() {
                if *x > 0 && i != a && v2.is_none_or(|v| *x <= f[v]) {
                    v2 = Some(i);
                }
            }
            let Some(b) = v2 else { break };
            f[a] = f[a].saturating_add(f[b]);
            f[b] = 0;
            let mut x = a;
            size[x] += 1;
            while let Some(o) = others.get(x).copied().filter(|o| *o != usize::MAX) {
                x = o;
                *size.get_mut(x)? += 1;
            }
            *others.get_mut(x)? = b;
            let mut x = b;
            size[x] += 1;
            while let Some(o) = others.get(x).copied().filter(|o| *o != usize::MAX) {
                x = o;
                *size.get_mut(x)? += 1;
            }
        }
        let longest = size.iter().copied().max().unwrap_or(0);
        let mut bits = vec![0usize; longest.max(16) + 1];
        for s in size.iter().filter(|s| **s > 0) {
            *bits.get_mut(*s)? += 1;
        }
        // Limit the lengths to 16 bits (Figure K.3).
        let mut i = longest;
        while i > 16 {
            while *bits.get(i)? > 0 {
                let mut j = i.checked_sub(2)?;
                while *bits.get(j)? == 0 {
                    j = j.checked_sub(1)?;
                }
                *bits.get_mut(i)? -= 2;
                *bits.get_mut(i - 1)? += 1;
                *bits.get_mut(j + 1)? += 2;
                *bits.get_mut(j)? -= 1;
            }
            i -= 1;
        }
        // Drop the reserved code point: one code of the longest length.
        let mut i = 16;
        while *bits.get(i)? == 0 {
            i = i.checked_sub(1)?;
        }
        *bits.get_mut(i)? -= 1;
        let mut vals = Vec::new();
        for len in 1..=longest {
            for (s, sz) in size.iter().enumerate().take(256) {
                if *sz == len {
                    vals.push(s as u8);
                }
            }
        }
        let mut counts = [0u8; 16];
        for (c, b) in counts.iter_mut().zip(bits.iter().skip(1)) {
            *c = u8::try_from(*b).ok()?;
        }
        if counts.iter().map(|c| usize::from(*c)).sum::<usize>() != vals.len() {
            return None;
        }
        let mut code = [(0u16, 0u8); 256];
        let (mut next, mut k) = (0u32, 0usize);
        for (len, n) in counts.iter().enumerate() {
            for _ in 0..*n {
                let s = *vals.get(k)?;
                code[usize::from(s)] = (u16::try_from(next).ok()?, len as u8 + 1);
                next += 1;
                k += 1;
            }
            next <<= 1;
        }
        Some(Table { counts, vals, code })
    }

    /// Bits the symbols counted in `freq` take with this table.
    fn cost(&self, freq: &[u64; 256]) -> u64 {
        freq.iter().zip(self.code.iter()).map(|(f, (_, len))| f.saturating_mul(u64::from(*len))).sum()
    }

    /// Bytes of this table in a DHT segment.
    fn stored(&self) -> u64 {
        17 + self.vals.len() as u64
    }
}

/// The entropy-coded bytes of a scan, with byte stuffing.
struct Out<'a> {
    buf: Vec<u8>,
    acc: u64,
    n: u32,
    tables: &'a [Table],
    missing: bool,
}

impl Out<'_> {
    fn put(&mut self, value: u32, n: u32) {
        if n == 0 {
            return;
        }
        self.acc = (self.acc << n) | (u64::from(value) & ((1u64 << n) - 1));
        self.n += n;
        while self.n >= 8 {
            let b = (self.acc >> (self.n - 8)) as u8;
            self.buf.push(b);
            if b == 0xFF {
                self.buf.push(0);
            }
            self.n -= 8;
        }
    }

    /// Pad the last byte with ones.
    fn finish(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
}

impl Sink for Out<'_> {
    fn symbol(&mut self, table: usize, symbol: u8) {
        match self.tables.get(table).map(|t| t.code[usize::from(symbol)]) {
            Some((code, len)) if len > 0 => self.put(u32::from(code), u32::from(len)),
            _ => self.missing = true,
        }
    }
    fn bits(&mut self, value: u32, n: u8) {
        self.put(value, u32::from(n));
    }
}

/// The extra bits of value `v` of size category `c` (T.81 F.1.2.1).
fn extra(v: i32, c: u8) -> u32 {
    let v = if v < 0 { v - 1 } else { v };
    (v as u32) & ((1u32 << c.min(31)) - 1)
}

fn put_value(s: &mut dyn Sink, table: usize, run: u8, v: i32) {
    let c = category(v);
    s.symbol(table, run << 4 | c.min(15));
    s.bits(extra(v, c), c);
}

/// End-of-band run `eobrun` (T.81 G.1.2.2), if any.
fn flush_eob(s: &mut dyn Sink, table: usize, eobrun: &mut u32) {
    if *eobrun > 0 {
        let nb = 31 - eobrun.leading_zeros();
        s.symbol(table, (nb as u8) << 4);
        s.bits(*eobrun & ((1 << nb) - 1), nb as u8);
        *eobrun = 0;
    }
}

/// Call `f` with each block a scan of components `comps` codes, in order.
fn visit(j: &Jpeg, comps: &[usize], f: &mut dyn FnMut(usize, &[i16; 64])) -> Option<()> {
    if let [ci] = comps {
        let c = j.components.get(*ci)?;
        let (bx, by) = j.plain_blocks(c);
        for y in 0..by {
            for x in 0..bx {
                f(*ci, c.blocks.get(y * c.bw + x)?);
            }
        }
    } else {
        let (mx, my) = j.mcus();
        for y in 0..my {
            for x in 0..mx {
                for ci in comps {
                    let c = j.components.get(*ci)?;
                    let (h, v) = (usize::from(c.h), usize::from(c.v));
                    for by in 0..v {
                        for bx in 0..h {
                            f(*ci, c.blocks.get((y * v + by) * c.bw + x * h + bx)?);
                        }
                    }
                }
            }
        }
    }
    Some(())
}

/// One scan: its components, what it codes, and the DC and AC table (sink index) of each
/// component.
#[derive(Clone, Debug)]
struct Scan {
    comps: Vec<usize>,
    kind: Kind,
    dc: [usize; 4],
    ac: [usize; 4],
}

fn encode(j: &Jpeg, scan: &Scan, s: &mut dyn Sink) -> Option<()> {
    let mut pred = [0i32; 4];
    let mut eobrun = 0u32;
    let ac_table = scan.comps.first().map_or(0, |c| scan.ac[c & 3]);
    visit(j, &scan.comps, &mut |ci, b| {
        let ci = ci & 3;
        if matches!(scan.kind, Kind::Sequential | Kind::Dc) {
            let dc = i32::from(b[0]);
            let diff = dc - pred[ci];
            pred[ci] = dc;
            let c = category(diff);
            s.symbol(scan.dc[ci], c);
            s.bits(extra(diff, c), c);
        }
        let (ss, se) = match scan.kind {
            Kind::Sequential => (1, 63),
            Kind::Dc => return,
            Kind::Ac(ss, se) => (usize::from(ss), usize::from(se)),
        };
        let mut run = 0u8;
        for v in b.iter().take(se.min(63) + 1).skip(ss) {
            let v = i32::from(*v);
            if v == 0 {
                run += 1;
                continue;
            }
            if scan.kind != Kind::Sequential {
                flush_eob(s, ac_table, &mut eobrun);
            }
            while run > 15 {
                s.symbol(scan.ac[ci], 0xF0);
                run -= 16;
            }
            put_value(s, scan.ac[ci], run, v);
            run = 0;
        }
        if run > 0 {
            if scan.kind == Kind::Sequential {
                s.symbol(scan.ac[ci], 0);
            } else {
                eobrun += 1;
                if eobrun == 0x7FFF {
                    flush_eob(s, ac_table, &mut eobrun);
                }
            }
        }
    })?;
    flush_eob(s, ac_table, &mut eobrun);
    Some(())
}

/// A scan and the tables (class, id, index into the made tables) defined just before it.
struct Planned {
    scan: Scan,
    tables: Vec<(u8, u8, usize)>,
}

/// Count `scans` (sharing tables `0..tables`), make the tables, and the cost of it all.
fn plan(j: &Jpeg, scans: Vec<Scan>, tables: usize, define: Vec<Vec<(u8, u8, usize)>>) -> Option<(Vec<Planned>, Vec<Table>, u64)> {
    let mut count = Count::new(tables);
    for scan in &scans {
        encode(j, scan, &mut count)?;
    }
    if count.too_large {
        return None;
    }
    let made: Vec<Table> = count.freq.iter().map(Table::optimal).collect::<Option<_>>()?;
    let bits: u64 = made.iter().zip(&count.freq).map(|(t, f)| t.cost(f)).sum::<u64>() + count.extra;
    let mut total = bits / 8 + 1;
    let mut planned = Vec::new();
    for (scan, define) in scans.into_iter().zip(define) {
        let dht: u64 = define.iter().map(|(_, _, i)| made.get(*i).map_or(0, Table::stored)).sum();
        total += dht + 4 + 6 + 2 * scan.comps.len() as u64;
        planned.push(Planned { scan, tables: define });
    }
    Some((planned, made, total))
}

/// Whether components `comps` may share one interleaved scan (at most 10 blocks an MCU).
fn interleavable(j: &Jpeg, comps: &[usize]) -> bool {
    comps.len() > 1 && comps.iter().map(|c| j.components.get(*c).map_or(99, |c| usize::from(c.h) * usize::from(c.v))).sum::<usize>() <= 10
}

/// The baseline layout: one scan (or one per component when they can't be interleaved), a DC
/// and an AC table for the first component and another pair shared by the others.
fn baseline(j: &Jpeg) -> Option<(Vec<Planned>, Vec<Table>, u64)> {
    let all: Vec<usize> = (0..j.components.len()).collect();
    let dc = [0, 1, 1, 1];
    let ac = [2, 3, 3, 3];
    let used = |c: &[usize]| -> Vec<(u8, u8, usize)> {
        let mut t = Vec::new();
        for ci in c {
            let ci = ci & 3;
            for d in [(0u8, (dc[ci]) as u8, dc[ci]), (1, (ac[ci] - 2) as u8, ac[ci])] {
                if !t.contains(&d) {
                    t.push(d);
                }
            }
        }
        t
    };
    let groups: Vec<Vec<usize>> = if all.len() == 1 || interleavable(j, &all) { vec![all] } else { all.iter().map(|c| vec![*c]).collect() };
    // Tables are defined once, before the first scan that uses them.
    let mut defined: Vec<(u8, u8, usize)> = Vec::new();
    let mut define = Vec::new();
    for g in &groups {
        let new: Vec<_> = used(g).into_iter().filter(|t| !defined.contains(t)).collect();
        defined.extend(new.iter().copied());
        define.push(new);
    }
    let scans = groups.into_iter().map(|comps| Scan { comps, kind: Kind::Sequential, dc, ac }).collect();
    plan(j, scans, 4, define)
}

/// Candidate splits of a component's AC coefficients into bands.
const SPLITS: [&[(u8, u8)]; 5] = [&[(1, 63)], &[(1, 2), (3, 63)], &[(1, 5), (6, 63)], &[(1, 9), (10, 63)], &[(1, 2), (3, 9), (10, 63)]];

/// The progressive layout: the DC coefficients first, then each component's AC coefficients in
/// the bands that cost least, each scan with its own tables.
fn progressive(j: &Jpeg) -> Option<(Vec<Planned>, Vec<Table>, u64)> {
    let n = j.components.len();
    let all: Vec<usize> = (0..n).collect();
    let mut scans = Vec::new();
    let mut define = Vec::new();
    let mut tables = 0usize;
    let dc_groups: Vec<Vec<usize>> = if n == 1 || interleavable(j, &all) { vec![all.clone()] } else { all.iter().map(|c| vec![*c]).collect() };
    for g in dc_groups {
        let mut dc = [0usize; 4];
        let mut d = Vec::new();
        for ci in &g {
            dc[ci & 3] = tables;
            d.push((0u8, (ci & 3) as u8, tables));
            tables += 1;
        }
        scans.push(Scan { comps: g, kind: Kind::Dc, dc, ac: [0; 4] });
        define.push(d);
    }
    // Each band's cost on its own: the bits with its own table, the table and the headers.
    let mut bands: Vec<(usize, (u8, u8))> = Vec::new();
    for ci in 0..n {
        let mut costs: Vec<((u8, u8), u64)> = Vec::new();
        for split in SPLITS {
            for band in split.iter() {
                if costs.iter().any(|(b, _)| b == band) {
                    continue;
                }
                let scan = Scan { comps: vec![ci], kind: Kind::Ac(band.0, band.1), dc: [0; 4], ac: [0; 4] };
                let (_, _, cost) = plan(j, vec![scan], 1, vec![vec![(1, 0, 0)]])?;
                costs.push((*band, cost));
            }
        }
        let cost_of =
            |split: &[(u8, u8)]| -> u64 { split.iter().map(|b| costs.iter().find(|(x, _)| x == b).map_or(u64::MAX / 8, |(_, c)| *c)).sum() };
        let best = SPLITS.iter().min_by_key(|s| cost_of(s))?;
        bands.extend(best.iter().map(|b| (ci, *b)));
    }
    // The first band of every component, then the rest, so a partial file shows all colours.
    bands.sort_by_key(|(ci, (ss, _))| (*ss > 1, *ci));
    for (ci, (ss, se)) in bands {
        let mut ac = [0usize; 4];
        ac[ci & 3] = tables;
        scans.push(Scan { comps: vec![ci], kind: Kind::Ac(ss, se), dc: [0; 4], ac });
        define.push(vec![(1, 0, tables)]);
        tables += 1;
    }
    plan(j, scans, tables, define)
}

fn segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) -> Option<()> {
    let len = u16::try_from(payload.len() + 2).ok()?;
    out.extend([0xFF, marker]);
    out.extend(len.to_be_bytes());
    out.extend_from_slice(payload);
    Some(())
}

/// `j` stored with tables made for it: baseline or progressive, whichever is smaller.
pub(crate) fn write(j: &Jpeg) -> Option<Vec<u8>> {
    write_as(j, None)
}

/// [`write`], or only baseline (`Some(false)`) or only progressive (`Some(true)`).
pub(crate) fn write_as(j: &Jpeg, progressive_only: Option<bool>) -> Option<Vec<u8>> {
    let n = j.components.len();
    if !(1..=4).contains(&n) || j.components.iter().any(|c| c.blocks.len() != c.bw.saturating_mul(c.bh)) {
        return None;
    }
    let base = if progressive_only == Some(true) { None } else { baseline(j) };
    let prog = if progressive_only == Some(false) { None } else { progressive(j) };
    let (layout, sof) = match (base, prog) {
        (Some(b), Some(p)) => {
            if p.2 < b.2 {
                (p, 0xC2)
            } else {
                (b, 0xC0)
            }
        }
        (Some(b), None) => (b, 0xC0),
        (None, Some(p)) => (p, 0xC2),
        (None, None) => return None,
    };
    let (planned, tables, _) = layout;
    let mut out = vec![0xFF, 0xD8];
    for (m, payload) in &j.markers {
        segment(&mut out, *m, payload)?;
    }
    let mut dqt = Vec::new();
    let mut wide = false;
    for (id, t) in j.qtables.iter().enumerate() {
        let Some((pq, values)) = t else { continue };
        if !j.components.iter().any(|c| usize::from(c.tq) == id) {
            continue;
        }
        dqt.push(pq << 4 | id as u8);
        for v in values {
            if *pq == 0 {
                dqt.push(u8::try_from(*v).ok()?);
            } else {
                wide = true;
                dqt.extend(v.to_be_bytes());
            }
        }
    }
    segment(&mut out, 0xDB, &dqt)?;
    // Baseline allows only 8-bit quantization values; extended sequential allows more.
    let sof = if sof == 0xC0 && wide { 0xC1 } else { sof };
    let mut frame = vec![8];
    frame.extend(j.height.to_be_bytes());
    frame.extend(j.width.to_be_bytes());
    frame.push(n as u8);
    for c in &j.components {
        frame.extend([c.id, c.h << 4 | c.v, c.tq]);
    }
    segment(&mut out, sof, &frame)?;
    for p in planned {
        if !p.tables.is_empty() {
            let mut dht = Vec::new();
            for (class, id, i) in &p.tables {
                let t = tables.get(*i)?;
                dht.push(class << 4 | id);
                dht.extend(t.counts);
                dht.extend(&t.vals);
            }
            segment(&mut out, 0xC4, &dht)?;
        }
        let mut sos = vec![p.scan.comps.len() as u8];
        for ci in &p.scan.comps {
            let c = j.components.get(*ci)?;
            // The table ids this scan uses: as defined for this component.
            let (dc_id, ac_id) = match p.scan.kind {
                Kind::Sequential => (u8::from(*ci > 0), u8::from(*ci > 0)),
                Kind::Dc => ((ci & 3) as u8, 0),
                Kind::Ac(..) => (0, 0),
            };
            sos.extend([c.id, dc_id << 4 | ac_id]);
        }
        let (ss, se) = match p.scan.kind {
            Kind::Sequential => (0, 63),
            Kind::Dc => (0, 0),
            Kind::Ac(ss, se) => (ss, se),
        };
        sos.extend([ss, se, 0]);
        segment(&mut out, 0xDA, &sos)?;
        let mut w = Out { buf: Vec::new(), acc: 0, n: 0, tables: &tables, missing: false };
        encode(j, &p.scan, &mut w)?;
        w.finish();
        if w.missing {
            return None;
        }
        out.extend(w.buf);
    }
    out.extend([0xFF, 0xD9]);
    Some(out)
}
