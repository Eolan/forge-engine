//! A split grammar on flat scopes: D-039's control grammar, with CGA's operators (Müller,
//! Wonka, Haegler, Ulmer & Van Gool, "Procedural Modeling of Buildings", SIGGRAPH 2006). A
//! shape is a rectangle in space (its [`Scope`]) with a rule name and attributes; a rule turns
//! it into smaller shapes or a terminal, and the terminals are what a style set's modules are
//! built from. A building's mass is cut into faces before the grammar runs (the component
//! split, `city::facades`): each face starts as one shape with its context as attributes
//! (street or courtyard side, a party wall, a corner, the floors).
//!
//! The text form, one rule after another:
//!
//! ```text
//! # A comment runs to the end of the line.
//! Facade [party] -> insert(blank)
//! Facade -> split(y, level) { ground: Ground | {~floor: Floor}* | 0.9: Top }
//! Floor -> split(x) { ~0.6: Wall | {bay: Bay}* | ~0.6: Wall }
//! Bay [level.index == 0 && balconies] -> split(x) { ~1: Wall | 1.2: Balcony | ~1: Wall }
//! Bay -> choose(level.index) { 0.7: Window | 0.3: Wall }
//! Wall -> insert(wall)
//! ```
//!
//! - **Rules** are tried in order; the first whose condition holds rewrites the shape. A shape
//!   no rule matches is dropped (and counted). Every name a rule uses must have a rule.
//! - **Conditions** join terms with `&&`: an attribute (true when not zero), `!attribute`, or
//!   `attribute OP value` with `<`, `<=`, `>`, `>=`, `==`, `!=` against a number or another
//!   attribute. `width` and `height` are the scope's size.
//! - **`split(x)`, `split(y)`** cut the scope along its width or its height into parts, in
//!   order. A part's size is absolute (`3.2`), relative to the scope (`'0.5`), or floating
//!   (`~1.0`), and may name an attribute instead of a number (`bay`, `~floor`). Absolute and
//!   relative parts take their size; floating parts share what is left in proportion to their
//!   nominal sizes. A split without floating parts is stretched to fit.
//! - **Repeats** `{ ... }*` inside a split hold as many copies as fit: all-absolute repeats as
//!   many as fit whole once the floating parts have their nominal sizes (the floating parts
//!   take the rest), the others the nearest whole number. A repeat's members get `index`,
//!   `count`, `first`, `last` and `middle` (`middle` for `index == (count - 1) / 2`); a
//!   labelled split (`split(y, level)`) also sets `level.index` and the others, which the
//!   shapes below keep.
//! - **`insert(module)`** makes a terminal; **`choose { w: A | w: B }`** picks a rule by weight
//!   from the derivation's seed, the same for every shape of that rule unless attributes are
//!   listed (`choose(level.index)` picks per floor); **`nil`** drops the shape; a bare name
//!   renames it.
//! - **`openings { Wall | Opening }`** cuts the shape at the gaps the derivation was given
//!   ([`Grammar::derive_with_gaps`]: a building's openings, D-060): `Wall` between them,
//!   `Opening` on each, with `opening.kind`, `sill`, `glass` and `lintel` read in the shape's
//!   own terms.
//! - **`let bay = 2.8 | 3.0 | 3.4`** gives each derivation one of the values, picked from its
//!   seed, unless the start shape's attributes set it: the faces of one building, derived with
//!   one seed, share their bays.
//!
//! Everything is a pure function of the text, the start shape and the seed (D-016): sizes are
//! sums and products of the input, choices integer hashes.

use std::collections::HashMap;
use std::fmt;

use forge_core::Seed;
use glam::{DVec2, DVec3};

/// A symbol of a grammar: a rule, an attribute or a module, by its index in the grammar's table.
pub type Symbol = u16;

/// A flat scope: a rectangle in space, `size.x` metres along `x` and `size.y` along `y` from
/// `origin`. Its face looks along `x × y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scope {
    /// Its corner.
    pub origin: DVec3,
    /// The unit direction of its width.
    pub x: DVec3,
    /// The unit direction of its height.
    pub y: DVec3,
    /// Its width and height, metres.
    pub size: DVec2,
}

impl Scope {
    /// The unit normal of its face, `x × y`.
    pub fn normal(&self) -> DVec3 {
        self.x.cross(self.y)
    }

    /// The point `u` metres along its width and `v` up its height.
    pub fn at(&self, u: f64, v: f64) -> DVec3 {
        self.origin + self.x * u + self.y * v
    }
}

/// A shape's attributes: values by symbol, the later entry winning.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs(Vec<(Symbol, f64)>);

impl Attrs {
    /// Sets an attribute.
    pub fn set(&mut self, symbol: Symbol, value: f64) {
        match self.0.iter_mut().find(|(s, _)| *s == symbol) {
            Some(entry) => entry.1 = value,
            None => self.0.push((symbol, value)),
        }
    }

    /// Whether an attribute is set.
    pub fn has(&self, symbol: Symbol) -> bool {
        self.0.iter().any(|(s, _)| *s == symbol)
    }

    /// An attribute's value, 0 when unset.
    pub fn get(&self, symbol: Symbol) -> f64 {
        self.0
            .iter()
            .find(|(s, _)| *s == symbol)
            .map_or(0.0, |&(_, v)| v)
    }
}

/// A terminal of a derivation: a module to place on a scope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Terminal {
    /// The module's symbol ([`Grammar::name`] gives its name).
    pub module: Symbol,
    /// Where it goes.
    pub scope: Scope,
}

/// An opening the derivation's wall holds (D-060: the building's structure puts them, the
/// grammar dresses them): its stretch along the start scope's width, metres from its origin,
/// and its sill and head over the start scope's bottom. `openings { Wall | Opening }` cuts a
/// shape at the gaps it meets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    /// Where it starts along the start scope's width.
    pub from: f64,
    /// Where it ends.
    pub to: f64,
    /// What it is, as the shapes read it in `opening.kind` (the caller's numbering).
    pub kind: f64,
    /// Its sill over the start scope's bottom.
    pub sill: f64,
    /// Its head over the start scope's bottom.
    pub head: f64,
}

/// What a derivation's shapes share: its gaps, and its start scope's frame to place them.
struct Context<'a> {
    gaps: &'a [Gap],
    origin: DVec3,
    x: DVec3,
    y: DVec3,
}

/// What a derivation did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeriveStats {
    /// Shapes rewritten.
    pub shapes: u32,
    /// Terminals made.
    pub terminals: u32,
    /// Shapes no rule matched, dropped.
    pub unmatched: u32,
    /// Shapes dropped at the depth limit.
    pub too_deep: u32,
}

impl std::ops::AddAssign for DeriveStats {
    fn add_assign(&mut self, o: Self) {
        self.shapes += o.shapes;
        self.terminals += o.terminals;
        self.unmatched += o.unmatched;
        self.too_deep += o.too_deep;
    }
}

/// A size in a split.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Number(f64),
    Attr(Symbol),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Absolute,
    Relative,
    Floating,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Size {
    kind: Kind,
    value: Value,
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    One(Size, Symbol),
    Repeat(Vec<(Size, Symbol)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// The attributes a split sets on its parts: index, count, first, last, middle.
type Labels = [Symbol; 5];

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Split {
        axis: Axis,
        labels: Vec<Labels>,
        parts: Vec<Part>,
    },
    Insert(Symbol),
    Choose {
        keys: Vec<Symbol>,
        options: Vec<(f64, Symbol)>,
    },
    Rename(Symbol),
    Nil,
    /// The shape cut at the derivation's gaps: `wall` between them, `opening` on each.
    Openings {
        wall: Symbol,
        opening: Symbol,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Term {
    Truthy(Symbol, bool),
    Compare(Symbol, Cmp, Value),
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    terms: Vec<Term>,
    op: Op,
    /// Its place in the text, mixed into its choices.
    ordinal: u32,
}

/// A parsed grammar.
#[derive(Clone, Debug, PartialEq)]
pub struct Grammar {
    names: Vec<String>,
    lookup: HashMap<String, Symbol>,
    rules: HashMap<Symbol, Vec<Rule>>,
    /// `let` attributes: each derivation picks one of the values from its seed.
    lets: Vec<(Symbol, Vec<f64>)>,
}

/// A grammar text that does not parse: where and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Its line, from 1.
    pub line: u32,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// The scope's width and height, read as attributes.
const WIDTH: Symbol = 0;
const HEIGHT: Symbol = 1;
/// The labels of an unlabelled split.
const PLAIN: Labels = [2, 3, 4, 5, 6];
const FIELDS: [&str; 5] = ["index", "count", "first", "last", "middle"];

/// The deepest a derivation goes before it drops a shape.
const MAX_DEPTH: u32 = 48;
/// The most copies a repeat makes.
const MAX_REPEAT: f64 = 1000.0;

impl Grammar {
    /// Parses a grammar's text.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let tokens = lex(text)?;
        let mut grammar = Self {
            names: Vec::new(),
            lookup: HashMap::new(),
            rules: HashMap::new(),
            lets: Vec::new(),
        };
        for name in ["width", "height"].into_iter().chain(FIELDS) {
            grammar.intern(name);
        }
        let mut parser = Parser {
            tokens: &tokens,
            at: 0,
            grammar: &mut grammar,
            used: Vec::new(),
        };
        let mut ordinal = 0;
        while !parser.done() {
            if matches!(parser.peek(), Some(Token::Ident(s)) if s == "let") {
                parser.at += 1;
                let name = parser.ident()?;
                let name = parser.grammar.intern(&name);
                parser.expect("=")?;
                let mut values = vec![parser.number()?];
                while parser.eat("|") {
                    values.push(parser.number()?);
                }
                parser.grammar.lets.push((name, values));
                continue;
            }
            let (name, rule) = parser.rule(ordinal)?;
            ordinal += 1;
            parser.grammar.rules.entry(name).or_default().push(rule);
        }
        let used = std::mem::take(&mut parser.used);
        for (symbol, line) in used {
            if !grammar.rules.contains_key(&symbol) {
                return Err(ParseError {
                    line,
                    message: format!("rule `{}` is used but has no rule", grammar.name(symbol)),
                });
            }
        }
        Ok(grammar)
    }

    fn intern(&mut self, name: &str) -> Symbol {
        if let Some(&s) = self.lookup.get(name) {
            return s;
        }
        let s = self.names.len() as Symbol;
        self.names.push(name.to_owned());
        self.lookup.insert(name.to_owned(), s);
        s
    }

    /// A symbol's name.
    pub fn name(&self, symbol: Symbol) -> &str {
        &self.names[symbol as usize]
    }

    /// The symbol of a name the text uses, if it does.
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        self.lookup.get(name).copied()
    }

    /// The modules the text inserts, by name.
    pub fn modules(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .rules
            .values()
            .flatten()
            .filter_map(|r| match r.op {
                Op::Insert(m) => Some(self.name(m)),
                _ => None,
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Attributes by name, for a start shape; names the text never reads are skipped.
    pub fn attrs(&self, values: &[(&str, f64)]) -> Attrs {
        let mut attrs = Attrs::default();
        for &(name, value) in values {
            if let Some(s) = self.symbol(name) {
                attrs.set(s, value);
            }
        }
        attrs
    }

    /// Derives a start shape named `start` on `scope` with `attrs`, appending its terminals to
    /// `out`. An unknown start name derives nothing (one unmatched shape).
    pub fn derive(
        &self,
        start: &str,
        scope: Scope,
        attrs: &Attrs,
        seed: u64,
        out: &mut Vec<Terminal>,
    ) -> DeriveStats {
        self.derive_with_gaps(start, scope, attrs, seed, &[], out)
    }

    /// As [`Grammar::derive`], the wall holding `gaps` for `openings { Wall | Opening }` to cut
    /// at. Each opening's shape gets `opening.kind`, `sill` and `glass` (its sill and its height
    /// over the shape's bottom), `lintel` (what is left over its head) and the split fields.
    pub fn derive_with_gaps(
        &self,
        start: &str,
        scope: Scope,
        attrs: &Attrs,
        seed: u64,
        gaps: &[Gap],
        out: &mut Vec<Terminal>,
    ) -> DeriveStats {
        let mut sorted = gaps.to_vec();
        sorted.sort_by(|a, b| a.from.total_cmp(&b.from));
        let context = Context {
            gaps: &sorted,
            origin: scope.origin,
            x: scope.x,
            y: scope.y,
        };
        let mut stats = DeriveStats::default();
        let seed = Seed::new(seed);
        let mut attrs = attrs.clone();
        for (k, (name, values)) in self.lets.iter().enumerate() {
            if !attrs.has(*name) {
                let pick = seed.derive_str("let").derive(k as u64).value();
                attrs.set(*name, values[(pick % values.len() as u64) as usize]);
            }
        }
        match self.symbol(start) {
            Some(s) => self.shape(s, scope, &attrs, seed, 0, &context, out, &mut stats),
            None => stats.unmatched += 1,
        }
        stats
    }

    #[allow(clippy::too_many_arguments)]
    fn shape(
        &self,
        name: Symbol,
        scope: Scope,
        attrs: &Attrs,
        seed: Seed,
        depth: u32,
        context: &Context<'_>,
        out: &mut Vec<Terminal>,
        stats: &mut DeriveStats,
    ) {
        if depth >= MAX_DEPTH {
            stats.too_deep += 1;
            return;
        }
        let read = |s: Symbol| match s {
            WIDTH => scope.size.x,
            HEIGHT => scope.size.y,
            _ => attrs.get(s),
        };
        let value = |v: Value| match v {
            Value::Number(n) => n,
            Value::Attr(s) => read(s),
        };
        let rule = self.rules.get(&name).and_then(|rules| {
            rules.iter().find(|r| {
                r.terms.iter().all(|t| match *t {
                    Term::Truthy(s, negate) => (read(s) != 0.0) != negate,
                    Term::Compare(s, cmp, v) => {
                        let (a, b) = (read(s), value(v));
                        match cmp {
                            Cmp::Lt => a < b,
                            Cmp::Le => a <= b,
                            Cmp::Gt => a > b,
                            Cmp::Ge => a >= b,
                            Cmp::Eq => a == b,
                            Cmp::Ne => a != b,
                        }
                    }
                })
            })
        });
        let Some(rule) = rule else {
            stats.unmatched += 1;
            return;
        };
        stats.shapes += 1;
        match &rule.op {
            Op::Nil => {}
            Op::Openings { wall, opening } => {
                // The shape's place in the start scope, to read the gaps in its own terms.
                let u0 = (scope.origin - context.origin).dot(context.x);
                let v0 = (scope.origin - context.origin).dot(context.y);
                let (width, height) = (scope.size.x, scope.size.y);
                let gaps: Vec<&Gap> = context
                    .gaps
                    .iter()
                    .filter(|g| g.to > u0 + 1e-6 && g.from < u0 + width - 1e-6)
                    .collect();
                let fields = |i: usize, n: usize| {
                    [
                        i as f64,
                        n as f64,
                        f64::from(u8::from(i == 0)),
                        f64::from(u8::from(i + 1 == n)),
                        f64::from(u8::from(n > 0 && i == (n - 1) / 2)),
                    ]
                };
                let part = |from: f64, to: f64| Scope {
                    origin: scope.origin + scope.x * from,
                    size: DVec2::new(to - from, height),
                    ..scope
                };
                // The walls between, then the openings.
                let mut walls = Vec::new();
                let mut holes = Vec::new();
                let mut at = 0.0;
                for g in &gaps {
                    let (a, b) = ((g.from - u0).max(at), (g.to - u0).min(width));
                    if a > at + 1e-6 {
                        walls.push((at, a));
                    }
                    if b > a + 1e-6 {
                        holes.push((a, b, *g));
                    }
                    at = at.max(b);
                }
                if width > at + 1e-6 {
                    walls.push((at, width));
                }
                let set = |child: &mut Attrs, name: &str, v: f64| {
                    if let Some(s) = self.symbol(name) {
                        child.set(s, v);
                    }
                };
                let n = walls.len();
                for (i, &(a, b)) in walls.iter().enumerate() {
                    let mut child = attrs.clone();
                    for (s, v) in PLAIN.iter().zip(fields(i, n)) {
                        child.set(*s, v);
                    }
                    self.shape(
                        *wall,
                        part(a, b),
                        &child,
                        seed,
                        depth + 1,
                        context,
                        out,
                        stats,
                    );
                }
                let n = holes.len();
                for (i, &(a, b, g)) in holes.iter().enumerate() {
                    let mut child = attrs.clone();
                    for (s, v) in PLAIN.iter().zip(fields(i, n)) {
                        child.set(*s, v);
                    }
                    let sill = (g.sill - v0).clamp(0.0, height);
                    let head = (g.head - v0).clamp(sill, height);
                    set(&mut child, "opening.kind", g.kind);
                    set(&mut child, "sill", sill);
                    set(&mut child, "glass", head - sill);
                    set(&mut child, "lintel", height - head);
                    self.shape(
                        *opening,
                        part(a, b),
                        &child,
                        seed,
                        depth + 1,
                        context,
                        out,
                        stats,
                    );
                }
            }
            Op::Insert(module) => {
                stats.terminals += 1;
                out.push(Terminal {
                    module: *module,
                    scope,
                });
            }
            Op::Rename(next) => {
                self.shape(*next, scope, attrs, seed, depth + 1, context, out, stats)
            }
            Op::Choose { keys, options } => {
                let mut pick = seed.derive(u64::from(rule.ordinal));
                for &k in keys {
                    pick = pick.derive(read(k).to_bits());
                }
                let total: f64 = options.iter().map(|o| o.0).sum();
                let mut r = (pick.value() >> 11) as f64 / (1u64 << 53) as f64 * total;
                let mut chosen = options[options.len() - 1].1;
                for &(w, next) in options {
                    if r < w {
                        chosen = next;
                        break;
                    }
                    r -= w;
                }
                self.shape(chosen, scope, attrs, seed, depth + 1, context, out, stats);
            }
            Op::Split {
                axis,
                labels,
                parts,
            } => {
                let length = match axis {
                    Axis::X => scope.size.x,
                    Axis::Y => scope.size.y,
                };
                let mut at = 0.0;
                for piece in layout(parts, length, &value) {
                    let size = piece.size;
                    if size <= 1e-6 {
                        continue;
                    }
                    let child = match axis {
                        Axis::X => Scope {
                            origin: scope.origin + scope.x * at,
                            size: DVec2::new(size, scope.size.y),
                            ..scope
                        },
                        Axis::Y => Scope {
                            origin: scope.origin + scope.y * at,
                            size: DVec2::new(scope.size.x, size),
                            ..scope
                        },
                    };
                    at += size;
                    let mut child_attrs = attrs.clone();
                    let (i, n) = (piece.index, piece.count);
                    let fields = [
                        f64::from(i),
                        f64::from(n),
                        f64::from(u8::from(i == 0)),
                        f64::from(u8::from(i + 1 == n)),
                        f64::from(u8::from(n > 0 && i == (n - 1) / 2)),
                    ];
                    for label in labels {
                        for (s, v) in label.iter().zip(fields) {
                            child_attrs.set(*s, v);
                        }
                    }
                    self.shape(
                        piece.name,
                        child,
                        &child_attrs,
                        seed,
                        depth + 1,
                        context,
                        out,
                        stats,
                    );
                }
            }
        }
    }
}

/// One part of a split laid out: its size, its rule, and its place in its repeat (0 of 1 for a
/// single part).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Piece {
    size: f64,
    name: Symbol,
    index: u32,
    count: u32,
}

/// Lays a split's parts out along `length`.
fn layout(parts: &[Part], length: f64, value: &dyn Fn(Value) -> f64) -> Vec<Piece> {
    let nominal = |s: &Size| match s.kind {
        Kind::Relative => value(s.value) * length,
        _ => value(s.value).max(0.0),
    };
    // What the single parts take, absolute and relative, and the floating parts' nominal sizes.
    let mut fixed = 0.0;
    let mut floating = 0.0;
    for part in parts {
        if let Part::One(size, _) = part {
            match size.kind {
                Kind::Floating => floating += nominal(size),
                _ => fixed += nominal(size),
            }
        }
    }
    let mut free = length - fixed - floating;
    // Absolute repeats first, as many whole copies as fit; then the others, the nearest count.
    let mut counts = vec![0u32; parts.len()];
    for (k, part) in parts.iter().enumerate() {
        if let Part::Repeat(items) = part
            && items.iter().all(|(s, _)| s.kind != Kind::Floating)
        {
            let step: f64 = items.iter().map(|(s, _)| nominal(s)).sum();
            if step > 1e-9 {
                let n = (free.max(0.0) / step + 1e-9).floor().min(MAX_REPEAT);
                counts[k] = n as u32;
                free -= n * step;
            }
        }
    }
    let others: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(k, p)| matches!(p, Part::Repeat(_)) && counts[*k] == 0)
        .filter(|(_, p)| match p {
            Part::Repeat(items) => items.iter().any(|(s, _)| s.kind == Kind::Floating),
            Part::One(..) => false,
        })
        .map(|(k, _)| k)
        .collect();
    for &k in &others {
        if let Part::Repeat(items) = &parts[k] {
            let step: f64 = items.iter().map(|(s, _)| nominal(s)).sum();
            if step > 1e-9 {
                let share = free.max(0.0) / others.len() as f64;
                counts[k] = (share / step).round().clamp(0.0, MAX_REPEAT) as u32;
            }
        }
    }
    // Expand, then size the floating parts from what is left.
    let mut pieces = Vec::new();
    let mut kinds = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        match part {
            Part::One(size, name) => {
                pieces.push(Piece {
                    size: nominal(size),
                    name: *name,
                    index: 0,
                    count: 1,
                });
                kinds.push(size.kind);
            }
            Part::Repeat(items) => {
                for i in 0..counts[k] {
                    for (size, name) in items {
                        pieces.push(Piece {
                            size: nominal(size),
                            name: *name,
                            index: i,
                            count: counts[k],
                        });
                        kinds.push(size.kind);
                    }
                }
            }
        }
    }
    let (mut set, mut float) = (0.0, 0.0);
    for (p, kind) in pieces.iter().zip(&kinds) {
        if *kind == Kind::Floating {
            float += p.size;
        } else {
            set += p.size;
        }
    }
    let left = length - set;
    if float > 1e-9 && left >= 0.0 {
        for (p, kind) in pieces.iter_mut().zip(&kinds) {
            if *kind == Kind::Floating {
                p.size *= left / float;
            }
        }
    } else if set > 1e-9 {
        // Nothing floats, or the set parts overflow: stretch or squeeze them to fit.
        let scale = length / set;
        for (p, kind) in pieces.iter_mut().zip(&kinds) {
            p.size = if *kind == Kind::Floating {
                0.0
            } else {
                p.size * scale
            };
        }
    }
    pieces
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Ident(String),
    Number(f64),
    Symbol(&'static str),
}

/// The text as tokens with their lines.
fn lex(text: &str) -> Result<Vec<(Token, u32)>, ParseError> {
    const SYMBOLS: [&str; 20] = [
        "->", "&&", "==", "!=", "<=", ">=", "<", ">", "!", "{", "}", "(", ")", "[", "]", "|", ":",
        "*", ",", "=",
    ];
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line_no = n as u32 + 1;
        let line = line.split('#').next().unwrap_or("");
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if c.is_whitespace() {
                i += 1;
            } else if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                out.push((Token::Ident(line[start..i].to_owned()), line_no));
            } else if c.is_ascii_digit()
                || (c == '-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
                || (c == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
            {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                let number = line[start..i].parse().map_err(|_| ParseError {
                    line: line_no,
                    message: format!("`{}` is not a number", &line[start..i]),
                })?;
                out.push((Token::Number(number), line_no));
            } else if c == '~' {
                out.push((Token::Symbol("~"), line_no));
                i += 1;
            } else if c == '\'' {
                out.push((Token::Symbol("'"), line_no));
                i += 1;
            } else if let Some(s) = SYMBOLS.iter().find(|s| line[i..].starts_with(**s)) {
                out.push((Token::Symbol(s), line_no));
                i += s.len();
            } else {
                return Err(ParseError {
                    line: line_no,
                    message: format!("unexpected `{c}`"),
                });
            }
        }
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: &'a [(Token, u32)],
    at: usize,
    grammar: &'a mut Grammar,
    /// Rule names used in bodies, with their lines, checked once all rules are read.
    used: Vec<(Symbol, u32)>,
}

impl Parser<'_> {
    fn done(&self) -> bool {
        self.at >= self.tokens.len()
    }

    fn line(&self) -> u32 {
        self.tokens
            .get(self.at)
            .or(self.tokens.last())
            .map_or(1, |t| t.1)
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            line: self.line(),
            message: message.into(),
        })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at).map(|t| &t.0)
    }

    fn is(&self, symbol: &str) -> bool {
        matches!(self.peek(), Some(Token::Symbol(s)) if *s == symbol)
    }

    fn eat(&mut self, symbol: &str) -> bool {
        let found = self.is(symbol);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, symbol: &str) -> Result<(), ParseError> {
        if self.eat(symbol) {
            Ok(())
        } else {
            self.error(format!("expected `{symbol}`"))
        }
    }

    fn ident(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some(Token::Ident(s)) => {
                let s = s.clone();
                self.at += 1;
                Ok(s)
            }
            _ => self.error("expected a name"),
        }
    }

    fn number(&mut self) -> Result<f64, ParseError> {
        match self.peek() {
            Some(&Token::Number(n)) => {
                self.at += 1;
                Ok(n)
            }
            _ => self.error("expected a number"),
        }
    }

    fn rule_name(&mut self) -> Result<Symbol, ParseError> {
        let line = self.line();
        let name = self.ident()?;
        let s = self.grammar.intern(&name);
        self.used.push((s, line));
        Ok(s)
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some(Token::Ident(_)) => {
                let name = self.ident()?;
                Ok(Value::Attr(self.grammar.intern(&name)))
            }
            _ => Ok(Value::Number(self.number()?)),
        }
    }

    fn rule(&mut self, ordinal: u32) -> Result<(Symbol, Rule), ParseError> {
        let name = self.ident()?;
        let name = self.grammar.intern(&name);
        let mut terms = Vec::new();
        if self.eat("[") {
            loop {
                let negate = self.eat("!");
                let attr = self.ident()?;
                let attr = self.grammar.intern(&attr);
                let cmp = [
                    ("<=", Cmp::Le),
                    (">=", Cmp::Ge),
                    ("==", Cmp::Eq),
                    ("!=", Cmp::Ne),
                    ("<", Cmp::Lt),
                    (">", Cmp::Gt),
                ]
                .into_iter()
                .find(|(s, _)| self.is(s));
                match cmp {
                    Some((s, cmp)) if !negate => {
                        self.expect(s)?;
                        terms.push(Term::Compare(attr, cmp, self.value()?));
                    }
                    Some(_) => return self.error("`!` takes an attribute, not a comparison"),
                    None => terms.push(Term::Truthy(attr, negate)),
                }
                if self.eat("]") {
                    break;
                }
                self.expect("&&")?;
            }
        }
        self.expect("->")?;
        let op = self.op()?;
        Ok((name, Rule { terms, op, ordinal }))
    }

    fn op(&mut self) -> Result<Op, ParseError> {
        let word = self.ident()?;
        match word.as_str() {
            "nil" => Ok(Op::Nil),
            "openings" => {
                self.expect("{")?;
                let wall = self.rule_name()?;
                self.expect("|")?;
                let opening = self.rule_name()?;
                self.expect("}")?;
                Ok(Op::Openings { wall, opening })
            }
            "insert" => {
                self.expect("(")?;
                let module = self.ident()?;
                self.expect(")")?;
                Ok(Op::Insert(self.grammar.intern(&module)))
            }
            "choose" => {
                let mut keys = Vec::new();
                if self.eat("(") {
                    loop {
                        let k = self.ident()?;
                        keys.push(self.grammar.intern(&k));
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                self.expect("{")?;
                let mut options = Vec::new();
                loop {
                    let weight = self.number()?;
                    if weight < 0.0 {
                        return self.error("a weight cannot be negative");
                    }
                    self.expect(":")?;
                    options.push((weight, self.rule_name()?));
                    if self.eat("}") {
                        break;
                    }
                    self.expect("|")?;
                }
                if options.iter().map(|o| o.0).sum::<f64>() <= 0.0 {
                    return self.error("a choice needs a weight above zero");
                }
                Ok(Op::Choose { keys, options })
            }
            "split" => {
                self.expect("(")?;
                let axis = match self.ident()?.as_str() {
                    "x" => Axis::X,
                    "y" => Axis::Y,
                    other => return self.error(format!("a split is along x or y, not `{other}`")),
                };
                let mut labels = vec![PLAIN];
                if self.eat(",") {
                    let label = self.ident()?;
                    labels.push(FIELDS.map(|f| self.grammar.intern(&format!("{label}.{f}"))));
                }
                self.expect(")")?;
                self.expect("{")?;
                let mut parts = Vec::new();
                loop {
                    if self.eat("{") {
                        let mut items = Vec::new();
                        loop {
                            items.push(self.sized()?);
                            if self.eat("}") {
                                break;
                            }
                            self.expect("|")?;
                        }
                        self.expect("*")?;
                        parts.push(Part::Repeat(items));
                    } else {
                        let (size, name) = self.sized()?;
                        parts.push(Part::One(size, name));
                    }
                    if self.eat("}") {
                        break;
                    }
                    self.expect("|")?;
                }
                Ok(Op::Split {
                    axis,
                    labels,
                    parts,
                })
            }
            _ => {
                // A bare name renames the shape.
                self.at -= 1;
                Ok(Op::Rename(self.rule_name()?))
            }
        }
    }

    fn sized(&mut self) -> Result<(Size, Symbol), ParseError> {
        let kind = if self.eat("~") {
            Kind::Floating
        } else if self.eat("'") {
            Kind::Relative
        } else {
            Kind::Absolute
        };
        let value = self.value()?;
        self.expect(":")?;
        Ok((Size { kind, value }, self.rule_name()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(width: f64, height: f64) -> Scope {
        Scope {
            origin: DVec3::ZERO,
            x: DVec3::X,
            y: DVec3::Y,
            size: DVec2::new(width, height),
        }
    }

    const FACADE: &str = "
        # Floors, then bays with piers at both ends.
        Facade -> split(y, level) { 4.2: Ground | {~3.2: Floor}* | 0.9: Top }
        Ground -> insert(shop)
        Top -> insert(cornice)
        Floor -> split(x) { ~0.6: Wall | {3.0: Bay}* | ~0.6: Wall }
        Bay [level.index == 0 && balconies] -> split(x) { ~1: Wall | 1.2: Balcony | ~1: Wall }
        Bay -> split(x) { ~1: Wall | 1.2: Window | ~1: Wall }
        Balcony -> insert(balcony)
        Window -> insert(window)
        Wall -> insert(wall)
    ";

    fn count(g: &Grammar, out: &[Terminal], module: &str) -> usize {
        let m = g.symbol(module).unwrap();
        out.iter().filter(|t| t.module == m).count()
    }

    #[test]
    fn a_facade_splits_into_floors_and_bays() {
        let g = Grammar::parse(FACADE).unwrap();
        let mut out = Vec::new();
        // 4.2 + 4 × 3.2 + 0.9 = 17.9 m; 20 m wide: 6 bays of 3 m, piers of 1 m.
        let attrs = g.attrs(&[("balconies", 1.0)]);
        let stats = g.derive("Facade", wall(20.0, 17.9), &attrs, 7, &mut out);
        assert_eq!(stats.unmatched, 0);
        assert_eq!(count(&g, &out, "shop"), 1);
        assert_eq!(count(&g, &out, "cornice"), 1);
        assert_eq!(count(&g, &out, "balcony"), 6, "the first floor's bays");
        assert_eq!(count(&g, &out, "window"), 18);
        // The terminals tile the facade exactly.
        let area: f64 = out.iter().map(|t| t.scope.size.x * t.scope.size.y).sum();
        assert!((area - 20.0 * 17.9).abs() < 1e-6, "{area}");
        let pier = out
            .iter()
            .find(|t| t.scope.origin.x == 0.0 && t.scope.origin.y > 4.0)
            .unwrap();
        assert!((pier.scope.size.x - 1.0).abs() < 1e-9);
    }

    #[test]
    fn floating_parts_take_the_rest_and_repeats_round() {
        let g = Grammar::parse(
            "A -> split(x) { 1: B | ~2: C | ~1: D | '0.25: E }
             B -> insert(b)  C -> insert(c)  D -> insert(d)  E -> insert(e)
             R -> split(x) { {~3: B}* }",
        )
        .unwrap();
        let mut out = Vec::new();
        g.derive("A", wall(10.0, 1.0), &Attrs::default(), 1, &mut out);
        let sizes: Vec<f64> = out.iter().map(|t| t.scope.size.x).collect();
        // 1 fixed, 2.5 relative, 6.5 left for C and D in 2:1.
        let expected = [1.0, 6.5 * 2.0 / 3.0, 6.5 / 3.0, 2.5];
        for (s, e) in sizes.iter().zip(expected) {
            assert!((s - e).abs() < 1e-9, "{sizes:?}");
        }
        out.clear();
        g.derive("R", wall(10.0, 1.0), &Attrs::default(), 1, &mut out);
        assert_eq!(out.len(), 3, "10 m of 3 m repeats round to 3");
        assert!(
            out.iter()
                .all(|t| (t.scope.size.x - 10.0 / 3.0).abs() < 1e-9)
        );
    }

    #[test]
    fn attributes_size_parts_and_label_repeats() {
        let g = Grammar::parse(
            "F -> split(y, level) { {floor: L}* }
             L [level.last] -> insert(top)
             L [level.middle] -> insert(middle)
             L -> insert(plain)",
        )
        .unwrap();
        let mut out = Vec::new();
        let attrs = g.attrs(&[("floor", 3.0), ("unused", 1.0)]);
        g.derive("F", wall(5.0, 15.0), &attrs, 1, &mut out);
        let names: Vec<&str> = out.iter().map(|t| g.name(t.module)).collect();
        assert_eq!(names, ["plain", "plain", "middle", "plain", "top"]);
    }

    #[test]
    fn choices_are_seeded_and_can_follow_attributes() {
        let g = Grammar::parse(
            "F -> split(y, level) { {1: L}* }
             L -> choose(level.index) { 1: A | 1: B }
             A -> insert(a)  B -> insert(b)
             G -> split(y) { {1: M}* }
             M -> choose { 1: A | 1: B }",
        )
        .unwrap();
        let run = |start: &str, seed: u64| {
            let mut out = Vec::new();
            g.derive(start, wall(1.0, 40.0), &Attrs::default(), seed, &mut out);
            out.iter().map(|t| t.module).collect::<Vec<_>>()
        };
        assert_eq!(run("F", 3), run("F", 3));
        let per_floor = run("F", 3);
        assert!(
            per_floor.iter().any(|&m| m != per_floor[0]),
            "varies by floor"
        );
        let per_building = run("G", 3);
        assert!(per_building.iter().all(|&m| m == per_building[0]));
    }

    #[test]
    fn lets_pick_per_seed_unless_set() {
        let g = Grammar::parse(
            "let bay = 2 | 5
             F -> split(x) { {bay: B}* }
             B -> insert(b)",
        )
        .unwrap();
        let bays = |attrs: &Attrs, seed: u64| {
            let mut out = Vec::new();
            g.derive("F", wall(10.0, 1.0), attrs, seed, &mut out);
            out.len()
        };
        let counts: Vec<usize> = (0..16).map(|s| bays(&Attrs::default(), s)).collect();
        assert!(counts.iter().all(|&n| n == 5 || n == 2));
        assert!(counts.contains(&5) && counts.contains(&2), "{counts:?}");
        assert_eq!(bays(&g.attrs(&[("bay", 2.0)]), 1), 5);
    }

    #[test]
    fn bad_texts_say_where() {
        let e = Grammar::parse("A -> split(x) { 1: B }").unwrap_err();
        assert!(e.message.contains("`B`"), "{e}");
        let e = Grammar::parse("A -> insert(a)\nB -> split(z) { 1: A }").unwrap_err();
        assert_eq!(e.line, 2);
        let e = Grammar::parse("A [x -> nil").unwrap_err();
        assert!(e.message.contains("expected"), "{e}");
    }

    #[test]
    fn openings_cut_a_storey_at_its_gaps() {
        let g = Grammar::parse(
            "
            Storey -> split(y) { ~1: Row | 0.3: Band }
            Row -> openings { Wall | Opening }
            Opening [opening.kind == 2] -> split(y) { glass: Hole | ~1: Wall }
            Opening -> split(y) { sill: Wall | glass: Window | lintel: Wall }
            Hole -> nil
            Band -> insert(band)
            Window -> insert(window)
            Wall -> insert(wall)
            ",
        )
        .unwrap();
        // A 10 m storey of 3.2 m: a window at 2-3.2 m (sill 0.9, head 2.4) and a passage at
        // 6-9 m (to 2.5 m), given out of order.
        let gaps = [
            Gap {
                from: 6.0,
                to: 9.0,
                kind: 2.0,
                sill: 0.0,
                head: 2.5,
            },
            Gap {
                from: 2.0,
                to: 3.2,
                kind: 1.0,
                sill: 0.9,
                head: 2.4,
            },
        ];
        let mut out = Vec::new();
        let stats = g.derive_with_gaps(
            "Storey",
            wall(10.0, 3.2),
            &Attrs::default(),
            3,
            &gaps,
            &mut out,
        );
        assert_eq!(stats.unmatched, 0);
        let window: Vec<&Terminal> = out
            .iter()
            .filter(|t| t.module == g.symbol("window").unwrap())
            .collect();
        assert_eq!(window.len(), 1);
        let w = window[0].scope;
        assert!((w.origin.x - 2.0).abs() < 1e-9 && (w.size.x - 1.2).abs() < 1e-9);
        assert!((w.origin.y - 0.9).abs() < 1e-9 && (w.size.y - 1.5).abs() < 1e-9);
        // Everything but the passage is covered: 10 × 3.2 less 3 × 2.5.
        let area: f64 = out.iter().map(|t| t.scope.size.x * t.scope.size.y).sum();
        assert!((area - (32.0 - 7.5)).abs() < 1e-9, "{area}");
    }
}
