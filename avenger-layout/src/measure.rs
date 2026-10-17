//! Measuring leaves whose size depends on the space they get, and the stored answers that
//! later solves reuse.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::RangeInclusive;

use crate::geometry::Direction;
use crate::node::ContentKey;

/// A measured leaf's minimum size in one direction, and where that size holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// The leaf's minimum size in the measured direction.
    pub size: f32,
    /// The values of the other direction at which `size` and `overhang` stay the same.
    pub holds: Holds,
    /// For a slab, how far its content reaches past the start and the end of its slot, along
    /// its side: past the left and right ends for a top or bottom slab, and past the top and
    /// bottom ends for a left or right slab, as an axis's end labels reach past its plot. The
    /// slab's host counts it as its overflow on that side. Only slabs have one.
    pub overhang: [f32; 2],
}

impl Measurement {
    /// A size that stays the same over `range` of the other direction, which must contain the
    /// value that was asked.
    pub fn over(size: f32, range: RangeInclusive<f32>) -> Self {
        Self {
            size,
            holds: Holds::Over(range),
            overhang: [0.0; 2],
        }
    }

    /// A size that does not depend on the other direction.
    pub fn always(size: f32) -> Self {
        Self {
            size,
            holds: Holds::Always,
            overhang: [0.0; 2],
        }
    }

    /// The measurement with a slab's `overhang` past the start and the end of its slot, which
    /// holds where the size holds.
    pub fn with_overhang(mut self, overhang: [f32; 2]) -> Self {
        self.overhang = overhang;
        self
    }
}

impl From<f32> for Measurement {
    fn from(size: f32) -> Self {
        Self {
            size,
            holds: Holds::At,
            overhang: [0.0; 2],
        }
    }
}

/// The values of the other direction at which a measurement's size stays the same.
#[derive(Clone, Debug, PartialEq)]
pub enum Holds {
    /// Only at the value that was asked.
    At,
    /// Over this range of values, which contains the value that was asked.
    Over(RangeInclusive<f32>),
    /// At every value, because the size does not depend on the other direction.
    Always,
}

/// Measures the leaves whose size depends on the space they get.
///
/// The solver asks only about leaves and slabs that have an id, and never about a slab that
/// another slab draws, except for the slabs of a [`Wrap`](crate::Wrap)'s children, which another
/// count can draw. Widths are solved first, so the two methods differ:
///
/// - [`height`](Measure::height) is asked in the height pass, once the leaf's width is final,
///   and its answer is used as given.
/// - [`width`](Measure::width) is asked after each solve, at the leaf's solved height. A
///   larger answer raises the leaf's minimum width and starts another solve, so widths only
///   grow within one measure.
///
/// A slab's answer can also give its [`overhang`](Measurement::overhang) past the ends of its
/// slot, at the same value. The overhang changes the pass that runs before the answer is
/// asked, the width pass for a top or bottom slab and the height pass for a left or right one,
/// so a larger overhang starts another solve, and overhangs only grow within one measure too.
///
/// For a slab, the value passed in is the length that its content takes along its side: the
/// side's length, unless the slab's maximum length or placement makes its content shorter. For
/// any other leaf, it is the space its slot leaves for its content. An answer must depend only
/// on the id and that value, because the solver reuses answers wherever they hold. For a leaf
/// or slab with a [`ContentKey`], the answer must depend only on the key and the value: nodes
/// with equal keys share answers, and the solver asks with the id of the first node in tree
/// order that needs one.
pub trait Measure<Id> {
    /// The leaf's minimum height at its final width.
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        let _ = (id, width);
        None
    }

    /// The leaf's minimum width at its solved height.
    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        let _ = (id, height);
        None
    }

    /// The leaf's max-content size in `direction`: the size it takes with no constraint, such
    /// as the width of unwrapped text. For a slab, it is asked only along the slab's side. A
    /// max-content size does not depend on the other direction, so it is asked once per leaf
    /// and direction and kept with the other answers.
    fn max_content(&mut self, id: &Id, direction: Direction) -> Option<f32> {
        let _ = (id, direction);
        None
    }
}

/// Answers nothing, for layouts without measured leaves.
impl<Id> Measure<Id> for () {}

/// The most answers kept for one key in one direction.
const ANSWERS_PER_KEY: usize = 8;

/// One stored answer: `size` and `overhang` hold for values from `low` to `high`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Answer {
    pub low: f32,
    pub high: f32,
    pub size: f32,
    pub overhang: [f32; 2],
}

impl Answer {
    /// The answer that `measurement` gives when asked at `value`.
    pub fn new(measurement: &Measurement, value: f32) -> Self {
        let (low, high) = match &measurement.holds {
            Holds::At => (value, value),
            Holds::Over(range) => (*range.start(), *range.end()),
            Holds::Always => (f32::NEG_INFINITY, f32::INFINITY),
        };
        Self {
            low,
            high,
            size: measurement.size,
            overhang: measurement.overhang,
        }
    }

    pub fn holds_at(&self, value: f32) -> bool {
        self.low <= value && value <= self.high
    }

    fn distance(&self, value: f32) -> f32 {
        if value < self.low {
            self.low - value
        } else if value > self.high {
            value - self.high
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug, Default)]
struct KeyAnswers {
    /// By measured direction: heights for vertical, widths for horizontal. Sorted by range,
    /// with no overlaps.
    answers: [Vec<Answer>; 2],
    /// The max-content answer per direction, once asked.
    max_content: [Option<Option<f32>>; 2],
    /// The clock when the answers were last used.
    used: u64,
}

/// What a node's answers are stored under: its content key when it has one, so that nodes that
/// draw the same thing share answers, and otherwise its id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AnswerKey<Id> {
    Id(Id),
    Content(ContentKey),
}

impl<Id: Clone> AnswerKey<Id> {
    pub fn of(id: &Id, content_key: Option<&ContentKey>) -> Self {
        match content_key {
            Some(key) => Self::Content(key.clone()),
            None => Self::Id(id.clone()),
        }
    }
}

/// The answers that measured leaves gave, by key: an [`AnswerKey`] in a solve.
#[derive(Clone, Debug)]
pub(crate) struct History<K> {
    keys: HashMap<K, KeyAnswers>,
    clock: u64,
}

impl<K> Default for History<K> {
    fn default() -> Self {
        Self {
            keys: HashMap::new(),
            clock: 0,
        }
    }
}

impl<K: Clone + Eq + Hash> History<K> {
    /// The stored size for `key` in the `measured` direction that holds at `value`.
    pub fn find(&self, key: &K, measured: Direction, value: f32) -> Option<Answer> {
        let answers = &self.keys.get(key)?.answers[measured.index()];
        let index = answers.partition_point(|answer| answer.high < value);
        answers
            .get(index)
            .filter(|answer| answer.holds_at(value))
            .copied()
    }

    /// The stored size that [`find`](Self::find) gives, which marks the key used.
    pub fn lookup(&mut self, key: &K, measured: Direction, value: f32) -> Option<Answer> {
        let answer = self.find(key, measured, value)?;
        self.touch(key);
        Some(answer)
    }

    fn touch(&mut self, key: &K) {
        self.clock += 1;
        if let Some(answers) = self.keys.get_mut(key) {
            answers.used = self.clock;
        }
    }

    /// Stores `answer`, asked at `value`. Stored answers that overlap it are replaced, and the
    /// answer farthest from `value` is dropped when the key has too many.
    pub fn insert(&mut self, key: &K, measured: Direction, value: f32, answer: Answer) {
        self.clock += 1;
        let entry = self.keys.entry(key.clone()).or_default();
        entry.used = self.clock;
        let answers = &mut entry.answers[measured.index()];
        answers.retain(|stored| stored.high < answer.low || stored.low > answer.high);
        let index = answers.partition_point(|stored| stored.high < answer.low);
        answers.insert(index, answer);
        if answers.len() > ANSWERS_PER_KEY {
            let farthest = answers
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.distance(value).total_cmp(&b.distance(value)))
                .map(|(index, _)| index);
            if let Some(farthest) = farthest {
                answers.remove(farthest);
            }
        }
    }

    /// The stored max-content answer for `key` in `direction`, once it has been asked.
    pub fn find_max_content(&self, key: &K, direction: Direction) -> Option<Option<f32>> {
        self.keys.get(key)?.max_content[direction.index()]
    }

    /// The stored max-content answer that [`find_max_content`](Self::find_max_content) gives,
    /// which marks the key used.
    pub fn max_content(&mut self, key: &K, direction: Direction) -> Option<Option<f32>> {
        let answer = self.find_max_content(key, direction)?;
        self.touch(key);
        Some(answer)
    }

    pub fn insert_max_content(&mut self, key: &K, direction: Direction, answer: Option<f32>) {
        self.clock += 1;
        let entry = self.keys.entry(key.clone()).or_default();
        entry.used = self.clock;
        entry.max_content[direction.index()] = Some(answer);
    }

    pub fn forget(&mut self, key: &K) {
        self.keys.remove(key);
    }

    /// Keeps the answers of keys that `present` accepts, and of at most as many other keys, the
    /// most recently used.
    pub fn retain(&mut self, present: impl Fn(&K) -> bool) {
        let kept = self.keys.keys().filter(|key| present(key)).count();
        let mut absent: Vec<(u64, K)> = self
            .keys
            .iter()
            .filter(|(key, _)| !present(key))
            .map(|(key, answers)| (answers.used, key.clone()))
            .collect();
        if absent.len() <= kept {
            return;
        }
        absent.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, key) in absent.into_iter().skip(kept) {
            self.keys.remove(&key);
        }
    }

    /// The number of answers stored for `key` in the `measured` direction.
    #[cfg(test)]
    pub fn len(&self, key: &K, measured: Direction) -> usize {
        self.keys
            .get(key)
            .map_or(0, |answers| answers.answers[measured.index()].len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: Direction = Direction::Vertical;

    fn answer(low: f32, high: f32, size: f32) -> Answer {
        Answer {
            low,
            high,
            size,
            overhang: [0.0; 2],
        }
    }

    #[test]
    fn lookup_finds_the_range_that_holds() {
        let mut history = History::default();
        history.insert(&"a", H, 300.0, answer(260.0, 410.0, 40.0));
        history.insert(&"a", H, 450.0, answer(420.0, f32::INFINITY, 20.0));
        assert_eq!(history.lookup(&"a", H, 400.0).map(|a| a.size), Some(40.0));
        assert_eq!(history.lookup(&"a", H, 1000.0).map(|a| a.size), Some(20.0));
        assert_eq!(history.lookup(&"a", H, 415.0), None);
        assert_eq!(history.lookup(&"a", Direction::Horizontal, 300.0), None);
    }

    #[test]
    fn an_overlapping_answer_replaces_older_ones() {
        let mut history = History::default();
        history.insert(&"a", H, 10.0, answer(0.0, 20.0, 1.0));
        history.insert(&"a", H, 30.0, answer(25.0, 40.0, 2.0));
        history.insert(&"a", H, 15.0, answer(f32::NEG_INFINITY, f32::INFINITY, 3.0));
        assert_eq!(history.len(&"a", H), 1);
        assert_eq!(history.lookup(&"a", H, 30.0).map(|a| a.size), Some(3.0));
    }

    #[test]
    fn a_full_history_drops_the_farthest_answer() {
        let mut history = History::default();
        for i in 0..9 {
            let value = i as f32 * 10.0;
            history.insert(&"a", H, value, answer(value, value, i as f32));
        }
        assert_eq!(history.len(&"a", H), 8);
        assert_eq!(history.lookup(&"a", H, 0.0), None);
        assert!(history.lookup(&"a", H, 80.0).is_some());
    }

    #[test]
    fn retain_keeps_as_many_absent_ids_as_present_ones() {
        let mut history = History::default();
        for id in ["a", "b", "c", "d"] {
            history.insert(&id, H, 1.0, answer(1.0, 1.0, 1.0));
        }
        history.retain(|id| *id == "a");
        assert!(history.lookup(&"a", H, 1.0).is_some());
        assert!(history.lookup(&"d", H, 1.0).is_some());
        assert_eq!(history.lookup(&"b", H, 1.0), None);
        assert_eq!(history.lookup(&"c", H, 1.0), None);
    }
}
