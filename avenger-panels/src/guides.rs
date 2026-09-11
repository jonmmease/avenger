use crate::{
    CellAlign, EquivalenceKey, GuideKey, NodeId, PanelDisplay, PanelError, PanelFrames, PanelId,
    PanelTree, Rect, Scope, Side,
};
use std::collections::{BTreeMap, BTreeSet};

/// A reference to caller-owned guide content for one panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideContribution {
    panel: PanelId,
    equivalent: Option<EquivalenceKey>,
}
impl GuideContribution {
    /// Supply a panel contribution without equivalence evidence.
    pub fn new(panel: PanelId) -> Self {
        Self {
            panel,
            equivalent: None,
        }
    }
    /// Certify that matching keys in this guide family can represent each other.
    ///
    /// Label keys include meaning, units, transform, formatting, and tick
    /// positions normalized against the plot rectangle. Legend keys include
    /// entries and their visual mappings. Refresh keys when that content changes.
    pub fn equivalent(mut self, key: EquivalenceKey) -> Self {
        self.equivalent = Some(key);
        self
    }
}

/// Tick-label visibility. Axis lines, tick marks, and grids are unaffected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LabelVisibility {
    /// Keep every eligible contribution.
    #[default]
    All,
    /// Keep an outer representative for each aligned compatible run.
    Outer,
    /// Hide every contribution in this family.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    key: GuideKey,
    side: Side,
    scope: Scope,
    contributions: Vec<GuideContribution>,
}

/// A family of panel-local axis tick labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AxisLabels {
    request: Request,
    visibility: LabelVisibility,
}
impl AxisLabels {
    /// Declare participants, the axis side, and a logical coordination scope.
    pub fn new(
        key: GuideKey,
        side: Side,
        scope: Scope,
        contributions: impl IntoIterator<Item = GuideContribution>,
    ) -> Self {
        Self {
            request: Request {
                key,
                side,
                scope,
                contributions: contributions.into_iter().collect(),
            },
            visibility: LabelVisibility::All,
        }
    }
    /// Select label visibility. The default is `All`.
    pub fn visibility(mut self, visibility: LabelVisibility) -> Self {
        self.visibility = visibility;
        self
    }
}

/// Shared guide kinds and their layout strata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedGuideKind {
    /// Axis title in guide chrome.
    AxisTitle,
    /// Panel or group header in strip chrome.
    Header,
    /// Legend in legend chrome.
    Legend,
}
/// A title, header, or legend attached to a logical scope anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedGuide {
    request: Request,
    kind: SharedGuideKind,
    align: CellAlign,
    order: i32,
}
impl SharedGuide {
    /// Declare a guide that appears once per nonempty group of eligible contributions.
    pub fn new(
        key: GuideKey,
        kind: SharedGuideKind,
        side: Side,
        scope: Scope,
        contributions: impl IntoIterator<Item = GuideContribution>,
    ) -> Self {
        Self {
            request: Request {
                key,
                side,
                scope,
                contributions: contributions.into_iter().collect(),
            },
            kind,
            align: CellAlign::Center,
            order: 0,
        }
    }
    /// Align along the anchor's full content edge. The default is center.
    pub fn align(mut self, align: CellAlign) -> Self {
        self.align = align;
        self
    }
    /// Order guides outward within the same anchor, side, and kind. Ties use the key.
    pub fn order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }
}

/// A typed guide request with an explicit scope and side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuideRequest {
    /// Labels attached to their representative panel's axis.
    AxisLabels(AxisLabels),
    /// A title, header, or legend attached to a scope anchor.
    Shared(SharedGuide),
}
impl From<AxisLabels> for GuideRequest {
    fn from(v: AxisLabels) -> Self {
        Self::AxisLabels(v)
    }
}
impl From<SharedGuide> for GuideRequest {
    fn from(v: SharedGuide) -> Self {
        Self::Shared(v)
    }
}
impl GuideRequest {
    fn request(&self) -> &Request {
        match self {
            Self::AxisLabels(v) => &v.request,
            Self::Shared(v) => &v.request,
        }
    }
}

/// Geometric options for guide planning.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GuideOptions {
    /// Maximum endpoint difference in logical pixels. Defaults to exact matching.
    pub alignment_tolerance: f32,
}

/// The kind of an instance that the caller measures and renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideKind {
    /// Tick labels attached to an axis.
    AxisLabels,
    /// Shared or local axis title.
    AxisTitle,
    /// Shared or local header.
    Header,
    /// Shared or local legend.
    Legend,
}
impl From<SharedGuideKind> for GuideKind {
    fn from(v: SharedGuideKind) -> Self {
        match v {
            SharedGuideKind::AxisTitle => Self::AxisTitle,
            SharedGuideKind::Header => Self::Header,
            SharedGuideKind::Legend => Self::Legend,
        }
    }
}

/// Stable guide identity: family plus representative panel or shared anchor.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GuideInstanceId {
    key: GuideKey,
    anchor: NodeId,
}
impl GuideInstanceId {
    /// Caller-assigned family identity.
    pub fn key(&self) -> &GuideKey {
        &self.key
    }
    /// Panel or group that owns the placement.
    pub fn anchor(&self) -> &NodeId {
        &self.anchor
    }
}

/// One guide to measure and draw, with content and placement kept separate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideInstance {
    id: GuideInstanceId,
    kind: GuideKind,
    source: PanelId,
    members: Vec<PanelId>,
    side: Side,
    align: CellAlign,
    order: i32,
}
impl GuideInstance {
    /// Stable instance identity.
    pub fn id(&self) -> &GuideInstanceId {
        &self.id
    }
    /// Guide family used to retrieve caller-owned content.
    pub fn key(&self) -> &GuideKey {
        &self.id.key
    }
    /// Component kind.
    pub fn kind(&self) -> GuideKind {
        self.kind
    }
    /// Panel supplying the equivalent content for this instance.
    pub fn source_panel(&self) -> &PanelId {
        &self.source
    }
    /// Represented panels in ID order.
    pub fn members(&self) -> &[PanelId] {
        &self.members
    }
    /// Placement anchor, independent of the source panel.
    pub fn anchor(&self) -> &NodeId {
        &self.id.anchor
    }
    /// Side of the anchor's content rectangle.
    pub fn side(&self) -> Side {
        self.side
    }
    /// Alignment along the anchor edge.
    pub fn alignment(&self) -> CellAlign {
        self.align
    }
    /// Outward placement order within an anchor, side, and kind.
    pub fn order(&self) -> i32 {
        self.order
    }
}

/// Why a contribution appears, is represented elsewhere, or is hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecisionReason {
    /// All labels were requested.
    AllLabels,
    /// This panel is the outer representative of a compatible run.
    OuterOwner,
    /// Another instance represents this contribution.
    Represented,
    /// This panel supplies a shared instance's content.
    SharedSource,
    /// No equivalence evidence was supplied.
    UnverifiedEquivalence,
    /// A potential representative has incompatible content.
    IncompatibleContent,
    /// Plot extents do not align.
    Unaligned,
    /// Overlap or an intervening panel prevents representation.
    Obstructed,
    /// The panel is a physical hole.
    Hole,
    /// The panel has no visible plot area.
    ZeroArea,
    /// Label visibility is set to `None`.
    HiddenByPolicy,
}

/// Decision for a submitted contribution. A represented contribution links to its instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideDecision {
    instance: Option<GuideInstanceId>,
    reason: DecisionReason,
}
impl GuideDecision {
    /// Instance representing this contribution, if any.
    pub fn instance(&self) -> Option<&GuideInstanceId> {
        self.instance.as_ref()
    }
    /// Explanation for the decision.
    pub fn reason(&self) -> DecisionReason {
        self.reason
    }
}

/// An immutable collection of instances and a decision for every submitted contribution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuidePlan {
    instances: BTreeMap<GuideInstanceId, GuideInstance>,
    decisions: BTreeMap<(GuideKey, PanelId), GuideDecision>,
}
impl GuidePlan {
    /// Instances in family and anchor order. Use `order()` when allocating chrome.
    pub fn instances(&self) -> impl Iterator<Item = &GuideInstance> {
        self.instances.values()
    }
    /// Find an instance referenced by a decision.
    pub fn instance(&self, id: &GuideInstanceId) -> Option<&GuideInstance> {
        self.instances.get(id)
    }
    /// Decision for a submitted contribution, or `None` if it was not submitted.
    pub fn decision(&self, key: &GuideKey, panel: &PanelId) -> Option<&GuideDecision> {
        self.decisions.get(&(key.clone(), panel.clone()))
    }
    fn decide(
        &mut self,
        key: &GuideKey,
        panel: &PanelId,
        instance: Option<GuideInstanceId>,
        reason: DecisionReason,
    ) {
        self.decisions.insert(
            (key.clone(), panel.clone()),
            GuideDecision { instance, reason },
        );
    }
    fn insert_local(
        &mut self,
        r: &Request,
        panel: &PanelId,
        reason: DecisionReason,
    ) -> GuideInstanceId {
        let id = GuideInstanceId {
            key: r.key.clone(),
            anchor: NodeId::Panel(panel.clone()),
        };
        self.instances.insert(
            id.clone(),
            GuideInstance {
                id: id.clone(),
                kind: GuideKind::AxisLabels,
                source: panel.clone(),
                members: vec![panel.clone()],
                side: r.side,
                align: CellAlign::Center,
                order: 0,
            },
        );
        self.decide(&r.key, panel, Some(id.clone()), reason);
        id
    }
}

impl PanelTree {
    /// Plan guides against current frames. Content equivalence is supplied by the caller.
    ///
    /// All inputs are validated before a complete plan is returned. Unknown IDs,
    /// duplicate families or participants, invalid scopes, and incompatible shared
    /// guides are errors. Unverified outer labels remain visible.
    pub fn plan_guides(
        &self,
        frames: &PanelFrames,
        requests: impl IntoIterator<Item = GuideRequest>,
        options: GuideOptions,
    ) -> Result<GuidePlan, PanelError> {
        if !options.alignment_tolerance.is_finite() || options.alignment_tolerance < 0.0 {
            return Err(PanelError::InvalidTolerance(options.alignment_tolerance));
        }
        if self != &frames.tree {
            return Err(PanelError::MismatchedTree);
        }
        let mut plan = GuidePlan::default();
        let mut keys = BTreeSet::new();
        for request in requests {
            let r = request.request();
            if !keys.insert(r.key.clone()) {
                return Err(PanelError::DuplicateGuide(r.key.clone()));
            }
            let groups = self.group(
                r.contributions.iter().map(|c| c.panel.clone()),
                r.scope.clone(),
            )?;
            let content: BTreeMap<_, _> = r
                .contributions
                .iter()
                .map(|c| (c.panel.clone(), c.equivalent.clone()))
                .collect();
            for c in &r.contributions {
                let reason = if frames.display(&c.panel) == Some(PanelDisplay::Hole) {
                    Some(DecisionReason::Hole)
                } else if !frames.visible(&c.panel) {
                    Some(DecisionReason::ZeroArea)
                } else {
                    None
                };
                if let Some(reason) = reason {
                    plan.decide(&r.key, &c.panel, None, reason);
                }
            }
            for group in groups.iter() {
                let members: Vec<_> = group
                    .members()
                    .iter()
                    .filter(|p| frames.visible(p))
                    .cloned()
                    .collect();
                if members.is_empty() {
                    continue;
                }
                match &request {
                    GuideRequest::Shared(shared) => {
                        let source = members.iter().min().expect("nonempty group");
                        if members.len() > 1
                            && (content[source].is_none()
                                || members.iter().any(|p| content[p] != content[source]))
                        {
                            return Err(PanelError::IncompatibleGuide {
                                key: r.key.clone(),
                                anchor: group.anchor().clone(),
                                panels: members,
                            });
                        }
                        let id = GuideInstanceId {
                            key: r.key.clone(),
                            anchor: group.anchor().clone(),
                        };
                        plan.instances.insert(
                            id.clone(),
                            GuideInstance {
                                id: id.clone(),
                                kind: shared.kind.into(),
                                source: source.clone(),
                                members: members.clone(),
                                side: r.side,
                                align: shared.align,
                                order: shared.order,
                            },
                        );
                        for panel in &members {
                            plan.decide(
                                &r.key,
                                panel,
                                Some(id.clone()),
                                if panel == source {
                                    DecisionReason::SharedSource
                                } else {
                                    DecisionReason::Represented
                                },
                            );
                        }
                    }
                    GuideRequest::AxisLabels(labels) => {
                        plan_labels(
                            &mut plan,
                            labels,
                            frames,
                            &content,
                            members,
                            options.alignment_tolerance,
                        );
                    }
                }
            }
        }
        for instance in plan.instances.values_mut() {
            instance.members.sort();
        }
        Ok(plan)
    }
}

fn plot(frames: &PanelFrames, panel: &PanelId) -> Rect {
    frames
        .rect(&NodeId::Panel(panel.clone()))
        .expect("validated frame")
}

// Compute in f64 so differences between finite f32 endpoints cannot overflow.
fn axes(r: Rect, side: Side) -> (f64, f64, f64, f64) {
    if matches!(side, Side::Top | Side::Bottom) {
        (
            r.x as f64,
            (r.x + r.width) as f64,
            r.y as f64,
            (r.y + r.height) as f64,
        )
    } else {
        (
            r.y as f64,
            (r.y + r.height) as f64,
            r.x as f64,
            (r.x + r.width) as f64,
        )
    }
}
fn aligned(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64), tolerance: f32) -> bool {
    (a.0 - b.0).abs() <= tolerance as f64 && (a.1 - b.1).abs() <= tolerance as f64
}
fn overlaps(a: f64, b: f64, c: f64, d: f64) -> bool {
    a < d && c < b
}

struct LabelRun<'a> {
    frames: &'a PanelFrames,
    content: &'a BTreeMap<PanelId, Option<EquivalenceKey>>,
    members: BTreeSet<PanelId>,
    side: Side,
    tolerance: f32,
}
impl LabelRun<'_> {
    fn can_cover(&self, panel: &PanelId, owner: &PanelId) -> Result<(), DecisionReason> {
        let a = axes(plot(self.frames, panel), self.side);
        let b = axes(plot(self.frames, owner), self.side);
        if !aligned(a, b, self.tolerance) {
            return Err(DecisionReason::Unaligned);
        }
        if overlaps(a.2, a.3, b.2, b.3) {
            return Err(DecisionReason::Obstructed);
        }
        if self.content[panel].is_none() || self.content[panel] != self.content[owner] {
            return Err(DecisionReason::IncompatibleContent);
        }
        let lo = a.2.min(b.2);
        let hi = a.3.max(b.3);
        for other in self
            .frames
            .tree
            .panels()
            .filter(|p| *p != panel && *p != owner && self.frames.visible(p))
        {
            let c = axes(plot(self.frames, other), self.side);
            if !overlaps(a.0, a.1, c.0, c.1) || !overlaps(lo, hi, c.2, c.3) {
                continue;
            }
            if !self.members.contains(other)
                || self.content.get(other) != self.content.get(panel)
                || !aligned(a, c, self.tolerance)
                || !aligned(b, c, self.tolerance)
                || overlaps(a.2, a.3, c.2, c.3)
                || overlaps(b.2, b.3, c.2, c.3)
            {
                return Err(DecisionReason::Obstructed);
            }
        }
        Ok(())
    }
}

fn plan_labels(
    plan: &mut GuidePlan,
    labels: &AxisLabels,
    frames: &PanelFrames,
    content: &BTreeMap<PanelId, Option<EquivalenceKey>>,
    mut members: Vec<PanelId>,
    tolerance: f32,
) {
    let r = &labels.request;
    members.sort_by(|a, b| {
        let x = axes(plot(frames, a), r.side);
        let y = axes(plot(frames, b), r.side);
        let cmp = if matches!(r.side, Side::Bottom | Side::Right) {
            y.3.total_cmp(&x.3)
        } else {
            x.2.total_cmp(&y.2)
        };
        cmp.then_with(|| a.cmp(b))
    });
    let run = LabelRun {
        frames,
        content,
        members: members.iter().cloned().collect(),
        side: r.side,
        tolerance,
    };
    let mut retained: Vec<(PanelId, GuideInstanceId)> = Vec::new();
    for panel in members {
        let mut reason = match labels.visibility {
            LabelVisibility::All => DecisionReason::AllLabels,
            LabelVisibility::None => {
                plan.decide(&r.key, &panel, None, DecisionReason::HiddenByPolicy);
                continue;
            }
            LabelVisibility::Outer if content[&panel].is_none() => {
                DecisionReason::UnverifiedEquivalence
            }
            LabelVisibility::Outer => DecisionReason::OuterOwner,
        };
        let mut represented = false;
        if labels.visibility == LabelVisibility::Outer && content[&panel].is_some() {
            // Nearest retained source is tried first. A farther source still has
            // to pass every intervening panel, including panels outside the scope.
            for (owner, id) in retained.iter().rev() {
                match run.can_cover(&panel, owner) {
                    Ok(()) => {
                        plan.instances
                            .get_mut(id)
                            .expect("retained instance")
                            .members
                            .push(panel.clone());
                        plan.decide(
                            &r.key,
                            &panel,
                            Some(id.clone()),
                            DecisionReason::Represented,
                        );
                        represented = true;
                        break;
                    }
                    Err(why) => reason = why,
                }
            }
        }
        if !represented {
            let id = plan.insert_local(r, &panel, reason);
            retained.push((panel, id));
        }
    }
}
