//! The matcher of `MATCH_RECOGNIZE`: a priority-ordered NFA, a Pike VM, over the rows of one
//! partition.
//!
//! The matcher tries each start row in order and takes the most preferred match there: the
//! first alternative, a greedy quantifier with more iterations, a reluctant one with fewer, or
//! with `longest`, the longest match, ties broken by preference. Threads advance one row at a
//! time in preference order, and a thread that reaches an instruction that a more preferred
//! thread reached at the same row stops, as in GoogleSQL's NFA. Since a row's variables don't
//! depend on the match in progress, the two threads have the same future, and each row costs at
//! most the program's size.

use super::pattern::{Anchor, Inst, Program};

/// Where matching resumes after a match, `AFTER MATCH SKIP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Skip {
    /// After the match's last row, the default.
    #[default]
    PastLastRow,
    /// At the row after the match's first row.
    ToNextRow,
}

/// A match of a partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The match's number in the partition, from 1, counting empty matches.
    pub number: u64,
    /// The match's first row, or for an empty match, the row where it starts.
    pub start: usize,
    /// The variable that each row of the match matched, in the rows' order from `start`.
    pub variables: Vec<usize>,
}

/// A thread of the VM: its instruction and its path.
#[derive(Clone, Copy)]
struct Thread {
    pc: usize,
    path: Option<u32>,
}

/// An entry of a step's list: a thread waiting to match a row, or a match.
enum Entry {
    Thread(Thread),
    Match(Option<u32>),
}

/// The matches of the partition of `rows` rows, whose row `row` meets the condition of the
/// variable `variable` when `meets(row, variable)`.
pub fn matches(
    program: &Program,
    rows: usize,
    meets: impl Fn(usize, usize) -> bool,
    skip: Skip,
    longest: bool,
) -> Vec<Match> {
    let mut vm = Vm {
        program,
        rows,
        meets: &meets,
        longest,
        paths: Vec::new(),
        seen: vec![false; program.insts.len()],
    };
    let mut out = Vec::new();
    let mut start = 0;
    while start < rows {
        match vm.attempt(start) {
            Some(variables) => {
                let length = variables.len();
                out.push(Match {
                    number: out.len() as u64 + 1,
                    start,
                    variables,
                });
                start = match (length, skip) {
                    (0, _) | (_, Skip::ToNextRow) => start + 1,
                    (length, Skip::PastLastRow) => start + length,
                };
            }
            None => start += 1,
        }
    }
    out
}

struct Vm<'a> {
    program: &'a Program,
    rows: usize,
    meets: &'a dyn Fn(usize, usize) -> bool,
    longest: bool,
    /// The arena of paths: each node a row's variable and the node of the rows before it.
    paths: Vec<(Option<u32>, usize)>,
    /// The instructions that a step's threads reached.
    seen: Vec<bool>,
}

impl Vm<'_> {
    /// The variables of the rows of the most preferred match that starts at `start`, or `None`.
    fn attempt(&mut self, start: usize) -> Option<Vec<usize>> {
        self.paths.clear();
        let mut best: Option<Option<u32>> = None;
        let mut list = Vec::new();
        self.seen.fill(false);
        self.add(&mut list, Thread { pc: 0, path: None }, start);
        let mut position = start;
        while !list.is_empty() {
            let mut next = Vec::new();
            self.seen.fill(false);
            let mut matched_here = false;
            for entry in list {
                match entry {
                    Entry::Match(path) => {
                        if !self.longest {
                            // Every thread after this one is less preferred.
                            best = Some(path);
                            break;
                        }
                        // Every thread of a step has matched the same rows, so the first match
                        // of the latest step is the longest and most preferred.
                        if !matched_here {
                            best = Some(path);
                            matched_here = true;
                        }
                    }
                    Entry::Thread(thread) => {
                        let Inst::Label(variable) = self.program.insts[thread.pc] else {
                            unreachable!("a thread waits at a label");
                        };
                        if position < self.rows && (self.meets)(position, variable) {
                            self.paths.push((thread.path, variable));
                            let path = Some(self.paths.len() as u32 - 1);
                            self.add(
                                &mut next,
                                Thread {
                                    pc: thread.pc + 1,
                                    path,
                                },
                                position + 1,
                            );
                        }
                    }
                }
            }
            list = next;
            position += 1;
        }
        best.map(|path| {
            let mut variables = Vec::new();
            let mut node = path;
            while let Some(index) = node {
                let (previous, variable) = self.paths[index as usize];
                variables.push(variable);
                node = previous;
            }
            variables.reverse();
            variables
        })
    }

    /// Adds `thread`, at the position `position`, and the threads that it reaches without
    /// matching a row, to `list` in preference order.
    fn add(&mut self, list: &mut Vec<Entry>, thread: Thread, position: usize) {
        let mut stack = vec![thread];
        while let Some(thread) = stack.pop() {
            if std::mem::replace(&mut self.seen[thread.pc], true) {
                continue;
            }
            let goto = |pc: usize| Thread {
                pc,
                path: thread.path,
            };
            match self.program.insts[thread.pc] {
                Inst::Label(_) => list.push(Entry::Thread(thread)),
                Inst::Match => list.push(Entry::Match(thread.path)),
                Inst::Jump(target) => stack.push(goto(target)),
                // The second branch waits on the stack until the first one is done.
                Inst::Split(first, second) => {
                    stack.push(goto(second));
                    stack.push(goto(first));
                }
                Inst::Anchor(anchor) => {
                    let holds = match anchor {
                        Anchor::Start => position == 0,
                        Anchor::End => position == self.rows,
                    };
                    if holds {
                        stack.push(goto(thread.pc + 1));
                    }
                }
            }
        }
    }
}
