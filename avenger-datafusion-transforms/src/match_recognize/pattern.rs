//! The pattern of `MATCH_RECOGNIZE`, and its compiler to the program that [`super::matcher`]
//! runs.
//!
//! A pattern matches rows by their pattern variables: the matcher sees, for each row and each
//! variable, whether the row meets the variable's condition. The program is GoogleSQL's NFA, in
//! the shape that gives GoogleSQL's results where an iteration can match no rows: `x*` is
//! `L: Split(x, exit); x; Jump L`, `x+` is `L: x; Split(L, exit)`, `x{n,}` is `n - 1` copies of
//! `x` and then `x+`, `x{n,m}` is `n` copies and then `m - n` nested optional copies, and a
//! reluctant quantifier prefers the exit at each split. With the matcher, which reaches each
//! instruction once at each row, `(() | a)*` takes the rows of `a`, and `(() | a)+`, `(a??)?` and
//! `(() | a){0,3}` match no rows, as in GoogleSQL.

/// An anchor of a pattern, which matches no rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// `^`, the start of the partition.
    Start,
    /// `$`, the end of the partition, after its last row.
    End,
}

/// A pattern of `MATCH_RECOGNIZE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// `()`, which matches no rows.
    Empty,
    /// A pattern variable, by its index, which matches one row that meets its condition.
    Variable(usize),
    Anchor(Anchor),
    /// The patterns in order.
    Concat(Vec<Pattern>),
    /// The patterns as alternatives, the first preferred.
    Alternation(Vec<Pattern>),
    /// `body{min,max}`, without a most for `None`: greedy repetitions prefer more iterations,
    /// and reluctant ones fewer.
    Repeat {
        body: Box<Pattern>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
}

/// An instruction of a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Inst {
    /// Matches one row that meets the condition of the variable.
    Label(usize),
    /// Continues at both, the first preferred.
    Split(usize, usize),
    Jump(usize),
    Anchor(Anchor),
    Match,
}

/// The most instructions of a program, which bounded quantifiers can multiply.
pub const MAX_INSTRUCTIONS: usize = 100_000;

/// A compiled pattern, which starts at its first instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub(crate) insts: Vec<Inst>,
}

impl Program {
    /// The program of `pattern`, or an error where it is over [`MAX_INSTRUCTIONS`].
    pub fn compile(pattern: &Pattern) -> Result<Program, String> {
        let mut compiler = Compiler { insts: Vec::new() };
        compiler.emit(pattern)?;
        compiler.push(Inst::Match)?;
        Ok(Program {
            insts: compiler.insts,
        })
    }

    /// The number of instructions of the program.
    pub fn len(&self) -> usize {
        self.insts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.insts.is_empty()
    }
}

struct Compiler {
    insts: Vec<Inst>,
}

/// A placeholder target, which a later patch sets.
const HOLE: usize = usize::MAX;

impl Compiler {
    fn push(&mut self, inst: Inst) -> Result<usize, String> {
        if self.insts.len() >= MAX_INSTRUCTIONS {
            return Err(format!(
                "The PATTERN of MATCH_RECOGNIZE compiles to more than {MAX_INSTRUCTIONS} instructions; write fewer or smaller bounded quantifiers"
            ));
        }
        self.insts.push(inst);
        Ok(self.insts.len() - 1)
    }

    fn next(&self) -> usize {
        self.insts.len()
    }

    /// Sets the holes of the instruction at `at` to `target`.
    fn patch(&mut self, at: usize, target: usize) {
        let fill = |slot: &mut usize| {
            if *slot == HOLE {
                *slot = target;
            }
        };
        match &mut self.insts[at] {
            Inst::Split(a, b) => {
                fill(a);
                fill(b);
            }
            Inst::Jump(a) => fill(a),
            _ => {}
        }
    }

    fn emit(&mut self, pattern: &Pattern) -> Result<(), String> {
        match pattern {
            Pattern::Empty => {}
            Pattern::Variable(variable) => {
                self.push(Inst::Label(*variable))?;
            }
            Pattern::Anchor(anchor) => {
                self.push(Inst::Anchor(*anchor))?;
            }
            Pattern::Concat(parts) => {
                for part in parts {
                    self.emit(part)?;
                }
            }
            Pattern::Alternation(parts) => {
                let mut jumps = Vec::new();
                for (index, part) in parts.iter().enumerate() {
                    if index + 1 == parts.len() {
                        self.emit(part)?;
                        break;
                    }
                    let split = self.push(Inst::Split(HOLE, HOLE))?;
                    let first = self.next();
                    self.emit(part)?;
                    jumps.push(self.push(Inst::Jump(HOLE))?);
                    let rest = self.next();
                    self.insts[split] = Inst::Split(first, rest);
                }
                let end = self.next();
                for jump in jumps {
                    self.patch(jump, end);
                }
            }
            Pattern::Repeat {
                body,
                min,
                max,
                greedy,
            } => self.repeat(body, *min, *max, *greedy)?,
        }
        Ok(())
    }

    /// `body{min,max}`, in GoogleSQL's shape: copies of the body, and a loop at the end.
    fn repeat(
        &mut self,
        body: &Pattern,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    ) -> Result<(), String> {
        let split = |taken: usize, skipped: usize| match greedy {
            true => Inst::Split(taken, skipped),
            false => Inst::Split(skipped, taken),
        };
        match max {
            // `body*`: `L: Split(body, exit); body; Jump L`.
            None if min == 0 => {
                let top = self.push(Inst::Split(HOLE, HOLE))?;
                let iteration = self.next();
                self.emit(body)?;
                self.push(Inst::Jump(top))?;
                let end = self.next();
                self.insts[top] = split(iteration, end);
            }
            // `body{n,}`: `n - 1` copies, then `L: body; Split(L, exit)`.
            None => {
                for _ in 1..min {
                    self.emit(body)?;
                }
                let top = self.next();
                self.emit(body)?;
                let at = self.push(Inst::Split(HOLE, HOLE))?;
                let end = self.next();
                self.insts[at] = split(top, end);
            }
            // `body{n,m}`: `n` copies, then `m - n` nested optional copies.
            Some(max) => {
                for _ in 0..min {
                    self.emit(body)?;
                }
                let mut exits = Vec::new();
                for _ in min..max {
                    let top = self.push(Inst::Split(HOLE, HOLE))?;
                    let iteration = self.next();
                    self.insts[top] = split(iteration, HOLE);
                    exits.push(top);
                    self.emit(body)?;
                }
                let end = self.next();
                for exit in exits {
                    self.patch(exit, end);
                }
            }
        }
        Ok(())
    }
}
