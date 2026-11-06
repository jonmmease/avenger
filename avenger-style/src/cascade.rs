//! The cascade: the computed value of every property of every node of a style tree.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::OnceLock,
};

use avenger_color::{choose_contrast_color, mix_colors, AbsoluteColor, ColorSpace};
use cssparser::{ParseError, Parser, Token};
use selectors::{
    context::{
        MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
        SelectorCaches,
    },
    matching::matches_selector,
    parser::{Component, Selector},
};

use crate::{
    computed::{AutoPaint, Computed, ComputedStyle, ComputedStyles, StyleWarning},
    condition::Condition,
    conditions::holds,
    host::HostInputs,
    palette::Palette,
    properties::Property,
    selector::Selectors,
    stylesheet::{expand, Declaration, Declared, LayerDeclaration, LayerName, Name, PaletteRule},
    table::{Entry, Origin, Sheet, StyleTable},
    tree::{NodeId, StyleTree},
    values::{parse_color, Calc, Category, Color, UnitSizes, Value, Wide},
    vocab::Element,
    Env,
};

/// The rules of a style table, sorted into buckets, which computes the styles of trees.
#[derive(Clone, Debug)]
pub struct Cascade {
    rules: RuleSet,
    envs: Vec<Env>,
}

impl Cascade {
    pub fn new(table: &StyleTable) -> Cascade {
        let mut builder = Builder::default();
        for (index, Entry { origin, sheet, .. }) in table.entries.iter().enumerate() {
            builder.add_sheet(*origin, sheet, None, index);
        }
        Cascade {
            rules: builder.finish(),
            envs: table
                .entries
                .iter()
                .map(|entry| entry.env.clone())
                .collect(),
        }
    }

    /// Computes the styles of every node of `tree` at `host`.
    pub fn compute(&self, tree: &StyleTree, host: &HostInputs) -> ComputedStyles {
        let theme = host.theme.as_ref().map(|theme| {
            let mut builder = Builder::default();
            builder.add_sheet(Origin::User, &theme.sheet, None, self.envs.len());
            builder.finish()
        });
        let mut envs: Vec<&Env> = self.envs.iter().collect();
        if let Some(theme) = &host.theme {
            envs.push(&theme.env);
        }
        let sets: Vec<&RuleSet> = std::iter::once(&self.rules).chain(theme.as_ref()).collect();
        let outcomes: Vec<Vec<bool>> = (sets.iter())
            .map(|set| {
                (set.rules.iter())
                    .map(|rule| {
                        rule.conditions
                            .iter()
                            .all(|condition| holds(condition, host))
                    })
                    .collect()
            })
            .collect();
        let keys: Vec<LayerKeys> = sets.iter().map(|set| layer_keys(set, host)).collect();
        let conditions: Vec<(Condition, bool)> = (sets.iter())
            .flat_map(|set| {
                (set.rules.iter())
                    .flat_map(|rule| rule.conditions.iter())
                    .chain(set.layers.iter().flat_map(|layer| layer.conditions.iter()))
            })
            .map(|condition| (condition.clone(), holds(condition, host)))
            .collect();

        let mut caches = SelectorCaches::default();
        let mut context = MatchingContext::new(
            MatchingMode::Normal,
            None,
            &mut caches,
            QuirksMode::NoQuirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        let mut computer = Computer {
            tree,
            host,
            envs: &envs,
            styles: Vec::with_capacity(tree.len()),
            customs: Vec::with_capacity(tree.len()),
            warnings: Vec::new(),
        };
        for node in tree.nodes() {
            let element = tree.element(node);
            let mut matched: Vec<Matched> = Vec::new();
            for (set_index, set) in sets.iter().enumerate() {
                let candidates = set.candidates(tree, node);
                // A rule that matches through several of its selectors counts at the highest
                // specificity among them.
                let mut index = 0;
                while index < candidates.len() {
                    let rule_index = candidates[index].0;
                    let mut specificity = None;
                    while index < candidates.len() && candidates[index].0 == rule_index {
                        let rule = &set.rules[rule_index];
                        let selector = &rule.selectors[candidates[index].1];
                        if outcomes[set_index][rule_index]
                            && matches_selector(selector, 0, None, element, &mut context)
                        {
                            specificity = specificity.max(Some(selector.specificity()));
                        }
                        index += 1;
                    }
                    let Some(specificity) = specificity else {
                        continue;
                    };
                    let rule = &set.rules[rule_index];
                    for (index, declaration) in rule.declarations.iter().enumerate() {
                        matched.push(Matched {
                            rank: rank(rule.origin, declaration.important),
                            layer: &keys[set_index].rules[rule_index],
                            specificity,
                            order: rule.order,
                            index: index as u32,
                            source: Source::Sheet(declaration, rule.env),
                        });
                    }
                }
            }
            for (index, (property, value)) in tree.node(node).inline.iter().enumerate() {
                matched.push(Matched {
                    rank: INLINE,
                    layer: &[],
                    specificity: u32::MAX,
                    order: u32::MAX,
                    index: index as u32,
                    source: Source::Inline(*property, value),
                });
            }
            matched.sort_by(compare);
            computer.node(node, &matched);
        }
        let (palettes, palette_warnings) = palettes(&sets, &keys, &envs, host);
        computer.warnings.extend(palette_warnings);
        ComputedStyles {
            styles: computer.styles,
            warnings: computer.warnings,
            palettes,
            host: host.clone(),
            conditions,
        }
    }
}

/// The rank of a declaration's origin and importance in the cascade, lowest first. Inline styles
/// rank between the normal and the important declarations of the author origin.
fn rank(origin: Origin, important: bool) -> u8 {
    match (origin, important) {
        (Origin::UserAgent, false) => 0,
        (Origin::User, false) => 1,
        (Origin::Author, false) => 2,
        (Origin::Author, true) => 4,
        (Origin::User, true) => 5,
        (Origin::UserAgent, true) => 6,
    }
}

const INLINE: u8 = 3;

/// The URL of the base stylesheet.
const BASE_URL: &str = "std:base.css";

/// A declaration that applies to a node, with what orders it in the cascade.
struct Matched<'a> {
    rank: u8,
    layer: &'a [u32],
    specificity: u32,
    order: u32,
    index: u32,
    source: Source<'a>,
}

#[derive(Clone, Copy)]
enum Source<'a> {
    /// A declaration of a stylesheet, with the index of its entry's `env()` values.
    Sheet(&'a Declaration, usize),
    Inline(Property, &'a Value),
}

impl Source<'_> {
    fn name(&self) -> Name {
        match self {
            Source::Sheet(declaration, _) => declaration.name.clone(),
            Source::Inline(property, _) => Name::Property(*property),
        }
    }
}

/// The order of two declarations by cascade precedence, lowest first: origin and importance,
/// then layers, which reverse for important declarations, then specificity, then order.
fn compare(a: &Matched, b: &Matched) -> Ordering {
    a.rank
        .cmp(&b.rank)
        .then_with(|| match a.rank >= 4 {
            false => a.layer.cmp(b.layer),
            true => b.layer.cmp(a.layer),
        })
        .then(a.specificity.cmp(&b.specificity))
        .then(a.order.cmp(&b.order))
        .then(a.index.cmp(&b.index))
}

#[derive(Clone, Debug, Default)]
struct RuleSet {
    rules: Vec<CompiledRule>,
    palettes: Vec<PaletteSource>,
    /// The declarations of layers in cascade order, which rank the layers at the host inputs
    /// that their conditions read.
    layers: Vec<LayerEvent>,
    by_id: HashMap<String, Vec<(usize, usize)>>,
    by_class: HashMap<String, Vec<(usize, usize)>>,
    by_element: HashMap<String, Vec<(usize, usize)>>,
    universal: Vec<(usize, usize)>,
}

impl RuleSet {
    /// The selectors that can match `node`, by the id, the classes, and the element of their last
    /// compound.
    fn candidates(&self, tree: &StyleTree, node: NodeId) -> Vec<(usize, usize)> {
        let data = tree.node(node);
        let mut candidates: Vec<(usize, usize)> = self.universal.clone();
        if let Some(found) = self.by_element.get(data.element.name()) {
            candidates.extend(found);
        }
        if let Some(id) = &data.id {
            if let Some(found) = self.by_id.get(id) {
                candidates.extend(found);
            }
        }
        for class in &data.classes {
            if let Some(found) = self.by_class.get(class) {
                candidates.extend(found);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

#[derive(Clone, Debug)]
struct CompiledRule {
    selectors: Vec<Selector<Selectors>>,
    declarations: Vec<Declaration>,
    origin: Origin,
    /// The full name of the rule's layer, or `None` outside layers.
    layer: Option<LayerName>,
    order: u32,
    conditions: Vec<Condition>,
    env: usize,
}

#[derive(Clone, Debug)]
struct PaletteSource {
    rule: PaletteRule,
    origin: Origin,
    layer: Option<LayerName>,
    order: u32,
    env: usize,
}

/// A declaration of a layer, by its full name, which counts when its conditions hold.
#[derive(Clone, Debug)]
struct LayerEvent {
    origin: Origin,
    name: LayerName,
    conditions: Vec<Condition>,
}

/// Flattens stylesheets into compiled rules with the full names of their layers.
#[derive(Default)]
struct Builder {
    set: RuleSet,
    order: u32,
    sheets: usize,
}

impl Builder {
    fn add_sheet(&mut self, origin: Origin, sheet: &Sheet, prefix: Option<LayerName>, env: usize) {
        self.sheets += 1;
        let uid = self.sheets;
        let rename = |name: &LayerName| -> LayerName {
            (name.iter())
                .map(|part| match part.starts_with(' ') {
                    true => format!("{part}/{uid}"),
                    false => part.clone(),
                })
                .collect()
        };
        // The layers rank in the order of their first declarations: those that the stylesheet
        // declares before each import, then the import's, and the rest after the last import.
        let layers = &sheet.stylesheet().layers;
        let mut declared = 0;
        for (import, imported) in sheet.stylesheet().imports.iter().zip(sheet.imports()) {
            let before = import.layers_before.min(layers.len());
            for declaration in &layers[declared.min(before)..before] {
                self.declare(origin, &prefix, &rename(&declaration.name), declaration);
            }
            declared = declared.max(before);
            // The base stylesheet is in the user-agent origin, whichever stylesheet imports it,
            // so a host theme and the author stylesheets win over it.
            if import.url == BASE_URL {
                self.add_sheet(Origin::UserAgent, imported, None, env);
                continue;
            }
            let layer = match (&prefix, &import.layer) {
                (_, Some(layer)) => Some(self.full(&prefix, &rename(layer))),
                (prefix, None) => prefix.clone(),
            };
            self.add_sheet(origin, imported, layer, env);
        }
        for declaration in &layers[declared..] {
            self.declare(origin, &prefix, &rename(&declaration.name), declaration);
        }
        let full_layer = |layer: &Option<LayerName>| -> Option<LayerName> {
            match (layer, &prefix) {
                (Some(layer), _) => {
                    let mut full = prefix.clone().unwrap_or_default();
                    full.extend(rename(layer));
                    Some(full)
                }
                (None, Some(prefix)) => Some(prefix.clone()),
                (None, None) => None,
            }
        };
        for rule in &sheet.stylesheet().rules {
            let layer = full_layer(&rule.layer);
            self.order += 1;
            let index = self.set.rules.len();
            let selectors: Vec<Selector<Selectors>> = rule.selectors.slice().to_vec();
            for (selector_index, selector) in selectors.iter().enumerate() {
                self.bucket(selector, (index, selector_index));
            }
            self.set.rules.push(CompiledRule {
                selectors,
                declarations: rule.declarations.clone(),
                origin,
                layer,
                order: self.order,
                conditions: rule.conditions.clone(),
                env,
            });
        }
        for palette in &sheet.stylesheet().palettes {
            let layer = full_layer(&palette.layer);
            self.order += 1;
            self.set.palettes.push(PaletteSource {
                rule: palette.clone(),
                origin,
                layer,
                order: self.order,
                env,
            });
        }
    }

    fn declare(
        &mut self,
        origin: Origin,
        prefix: &Option<LayerName>,
        name: &LayerName,
        declaration: &LayerDeclaration,
    ) {
        self.set.layers.push(LayerEvent {
            origin,
            name: self.full(prefix, name),
            conditions: declaration.conditions.clone(),
        });
    }

    fn full(&self, prefix: &Option<LayerName>, name: &LayerName) -> LayerName {
        let mut full = prefix.clone().unwrap_or_default();
        full.extend(name.iter().cloned());
        full
    }

    /// Adds a selector to the bucket of the id, a class, or the element of its last compound.
    fn bucket(&mut self, selector: &Selector<Selectors>, at: (usize, usize)) {
        let mut element = None;
        let mut class = None;
        for component in selector.iter_raw_match_order() {
            match component {
                Component::Combinator(_) => break,
                Component::ID(id) => {
                    self.set.by_id.entry(id.0.clone()).or_default().push(at);
                    return;
                }
                Component::Class(name) => class = Some(name.0.clone()),
                Component::LocalName(name) => element = Some(name.name.0.clone()),
                _ => {}
            }
        }
        match (class, element) {
            (Some(class), _) => self.set.by_class.entry(class).or_default().push(at),
            (None, Some(element)) => self.set.by_element.entry(element).or_default().push(at),
            (None, None) => self.set.universal.push(at),
        }
    }

    fn finish(self) -> RuleSet {
        self.set
    }
}

/// The keys of the layers of the rules and the palettes of a rule set.
struct LayerKeys {
    rules: Vec<Vec<u32>>,
    palettes: Vec<Vec<u32>>,
}

/// The keys of the layers of `set` at `host`, where each layer ranks by its first declaration
/// whose conditions hold, as CSS Cascade 5 orders conditional layers.
fn layer_keys(set: &RuleSet, host: &HostInputs) -> LayerKeys {
    let mut trees: HashMap<Origin, LayerTree> = HashMap::new();
    for layer in &set.layers {
        if layer
            .conditions
            .iter()
            .all(|condition| holds(condition, host))
        {
            trees.entry(layer.origin).or_default().key(&layer.name);
        }
    }
    let mut key = |origin: Origin, layer: &Option<LayerName>| match layer {
        Some(name) => trees.entry(origin).or_default().key(name),
        None => vec![u32::MAX],
    };
    LayerKeys {
        rules: (set.rules.iter())
            .map(|rule| key(rule.origin, &rule.layer))
            .collect(),
        palettes: (set.palettes.iter())
            .map(|palette| key(palette.origin, &palette.layer))
            .collect(),
    }
}

/// The order of the layers of one origin: each layer ranks among its siblings by its first
/// declaration, and `avenger` ranks first at the top.
#[derive(Default, Debug)]
struct LayerTree {
    children: Vec<(String, LayerTree)>,
}

impl LayerTree {
    /// The key of the layer `path`: the ranks of its parts, then a part that puts the layer's
    /// own declarations after its sublayers. Unlayered declarations have the key `[MAX]`, after
    /// every layer.
    fn key(&mut self, path: &[String]) -> Vec<u32> {
        let mut key = Vec::with_capacity(path.len() + 1);
        let mut tree = self;
        for (depth, part) in path.iter().enumerate() {
            let position = match tree.children.iter().position(|(name, _)| name == part) {
                Some(position) => position,
                None => {
                    tree.children.push((part.clone(), LayerTree::default()));
                    tree.children.len() - 1
                }
            };
            let rank = match (depth, part.as_str()) {
                (0, "avenger") => 0,
                _ => position as u32 + 1,
            };
            key.push(rank);
            tree = &mut tree.children[position].1;
        }
        key.push(u32::MAX);
        key
    }
}

/// Computes the values of the nodes of a tree in tree order.
struct Computer<'a> {
    tree: &'a StyleTree,
    host: &'a HostInputs,
    envs: &'a [&'a Env],
    styles: Vec<ComputedStyle>,
    customs: Vec<BTreeMap<String, String>>,
    warnings: Vec<StyleWarning>,
}

/// The properties in the order that computes them: `forced-color-adjust`, which decides the
/// declarations of forced colors, `color` before `currentColor`, `font-size` before `em`, and the
/// others in the order of the table, where `orient` comes before `direction`.
fn computation_order() -> &'static [Property] {
    static ORDER: OnceLock<Vec<Property>> = OnceLock::new();
    ORDER.get_or_init(|| {
        let first = [
            Property::ForcedColorAdjust,
            Property::Color,
            Property::FontSize,
        ];
        (first.iter().copied())
            .chain(
                Property::ALL
                    .iter()
                    .copied()
                    .filter(|property| !first.contains(property)),
            )
            .collect()
    })
}

impl Computer<'_> {
    fn node(&mut self, node: NodeId, matched: &[Matched]) {
        let data = self.tree.node(node);
        let parent = self.tree.parent(node).map(|parent| parent.index());
        let mut winners: HashMap<Name, Source> = HashMap::new();
        for declaration in matched {
            winners.insert(declaration.source.name(), declaration.source);
        }
        // Custom properties: the parent's computed values, which the node's declarations replace.
        // `inherit` and `unset` keep the parent's value, and `initial` removes it.
        let mut inherited = parent
            .map(|parent| self.customs[parent].clone())
            .unwrap_or_default();
        let mut specified: BTreeMap<&str, (&str, usize)> = BTreeMap::new();
        for (name, source) in &winners {
            if let (Name::Custom(name), Source::Sheet(declaration, env)) = (name, source) {
                match &declaration.value {
                    Declared::Tokens(text) => {
                        specified.insert(name.as_str(), (text.as_str(), *env));
                    }
                    Declared::Wide(Wide::Initial) => {
                        inherited.remove(name);
                    }
                    _ => {}
                }
            }
        }
        let customs = custom_properties(&specified, inherited, self.envs);

        let mut style = ComputedStyle {
            element: data.element,
            values: Property::ALL
                .iter()
                .map(|_| Computed::Keyword("auto"))
                .collect(),
            auto_paint: auto_paint(self.tree, node),
        };
        for property in computation_order() {
            if *property == Property::Color {
                // Forced colors apply to a node whose `forced-color-adjust` is `auto`.
                if style.keyword(Property::ForcedColorAdjust) == Some("none") {
                    winners.clear();
                    for declaration in matched.iter().filter(|declaration| declaration.rank != 6) {
                        winners.insert(declaration.source.name(), declaration.source);
                    }
                }
            }
            let winner = match property.applies_to(data.element) {
                true => winners.get(&Name::Property(*property)).copied(),
                false => None,
            };
            let value = self.compute(node, &style, parent, &customs, *property, winner);
            style.values[*property as usize] = value;
        }
        self.styles.push(style);
        self.customs.push(customs);
    }

    fn compute(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        parent: Option<usize>,
        customs: &BTreeMap<String, String>,
        property: Property,
        winner: Option<Source>,
    ) -> Computed {
        let inherited = parent.map(|parent| self.styles[parent].get(property).clone());
        let specified = match winner {
            None => None,
            Some(Source::Inline(_, value)) => Some(Specified::Value(value.clone())),
            Some(Source::Sheet(declaration, env)) => match &declaration.value {
                Declared::Value(value) => Some(Specified::Value(value.clone())),
                Declared::Wide(wide) => Some(Specified::Wide(*wide)),
                Declared::Pending { text, shorthand } => {
                    let substituted = substitute(text, customs, self.envs[env]);
                    let parsed = substituted.and_then(|text| pending(property, *shorthand, &text));
                    if parsed.is_none() {
                        self.warn(
                            node,
                            property,
                            format!("`{text}` is not a valid value after substitution"),
                        );
                    }
                    Some(parsed.unwrap_or(Specified::Wide(Wide::Unset)))
                }
                Declared::Tokens(_) => None,
            },
        };
        let specified = match specified {
            Some(specified) => specified,
            None if property.is_inherited() => Specified::Wide(Wide::Inherit),
            None => Specified::Wide(Wide::Initial),
        };
        let value = match specified {
            Specified::Value(value) => value,
            Specified::Wide(wide) => {
                let inherit = match wide {
                    Wide::Inherit => true,
                    Wide::Initial => false,
                    Wide::Unset => property.is_inherited(),
                };
                if inherit {
                    if let Some(value) = inherited.clone() {
                        return self.inherit(style, value);
                    }
                }
                initial(self.tree, node, style, property)
            }
        };
        match self.computed(style, parent, property, &value) {
            // A color that reads `currentColor` keeps its specified form for the nodes that
            // inherit it. `color` itself reads the parent's, so its value is absolute.
            Computed::Color(color) => match &value {
                Value::Color(specified)
                    if property != Property::Color && specified.reads_current_color() =>
                {
                    Computed::CurrentColor {
                        color,
                        specified: specified.clone(),
                    }
                }
                _ => Computed::Color(color),
            },
            computed => computed,
        }
    }

    /// The value that a node inherits from its parent's computed value: a color that reads
    /// `currentColor` resolves against the node's own `color`, which the cascade computes first.
    fn inherit(&self, style: &ComputedStyle, value: Computed) -> Computed {
        match value {
            Computed::CurrentColor { specified, .. } => {
                let current = style.color(Property::Color).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                Computed::CurrentColor {
                    color: resolve_color(&specified, current, self.host),
                    specified,
                }
            }
            value => value,
        }
    }

    /// The computed value of a specified value.
    fn computed(
        &self,
        style: &ComputedStyle,
        parent: Option<usize>,
        property: Property,
        value: &Value,
    ) -> Computed {
        let preferences = &self.host.preferences;
        let rem = 10.0 * preferences.text_scale;
        let parent_style = parent.map(|parent| &self.styles[parent]);
        let em = match property {
            Property::FontSize => parent_style.map_or(rem, ComputedStyle::font_size),
            _ => style.font_size(),
        };
        let sizes = UnitSizes { rem, em };
        let current = match property {
            Property::Color => parent_style.and_then(|parent| parent.color(Property::Color)),
            _ => style.color(Property::Color),
        }
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
        match value {
            Value::Numeric(calc) => numeric(calc, property, &sizes),
            Value::Integer(value) => Computed::Integer(*value),
            Value::Color(color) => Computed::Color(resolve_color(color, current, self.host)),
            Value::Keyword(keyword) => Computed::Keyword(keyword),
            Value::Boolean(value) => Computed::Boolean(*value),
            Value::String(value) => Computed::String(value.clone()),
            Value::Ratio(a, b) => Computed::Ratio(a / b),
            Value::List(items) => Computed::List(
                (items.iter())
                    .map(|item| self.computed(style, parent, property, item))
                    .collect(),
            ),
            Value::Sequence(items) => Computed::Sequence(
                (items.iter())
                    .map(|item| self.computed(style, parent, property, item))
                    .collect(),
            ),
        }
    }

    fn warn(&mut self, node: NodeId, property: Property, message: String) {
        self.warnings.push(StyleWarning::Property {
            node,
            property: property.name().to_string(),
            message,
        });
    }
}

enum Specified {
    Value(Value),
    Wide(Wide),
}

fn numeric(calc: &Calc, property: Property, sizes: &UnitSizes) -> Computed {
    let mut value = calc.resolve(sizes);
    // A `calc()` can resolve outside the range of its property, which clamps it, as in CSS.
    if let Some((min, max)) = calc
        .category()
        .and_then(|category| property.syntax().range(category))
    {
        if let Some(min) = min {
            value = value.max(min);
        }
        if let Some(max) = max {
            value = value.min(max);
        }
    }
    match calc.category() {
        Some(Category::Percentage) => Computed::Percentage(value),
        Some(Category::Number) if property == Property::LineHeight => Computed::Ratio(value),
        _ => Computed::Number(value),
    }
}

/// The specified value of a pending declaration after substitution.
fn pending(
    property: Property,
    shorthand: Option<crate::Shorthand>,
    text: &str,
) -> Option<Specified> {
    let mut input = Parser::new(text);
    if let Some(wide) = Wide::parse(&mut input) {
        return Some(Specified::Wide(wide));
    }
    match shorthand {
        None => property.parse_value(text).map(Specified::Value),
        Some(shorthand) => {
            let expanded = expand(shorthand, text)?;
            match expanded
                .into_iter()
                .find(|(longhand, _)| *longhand == property)?
            {
                (_, Declared::Value(value)) => Some(Specified::Value(value)),
                (_, Declared::Wide(wide)) => Some(Specified::Wide(wide)),
                _ => None,
            }
        }
    }
}

/// The initial value of `property` on `node`: the table's, or the value that depends on the
/// node, such as the `orient` of an axis.
fn initial(tree: &StyleTree, node: NodeId, style: &ComputedStyle, property: Property) -> Value {
    static INITIAL: OnceLock<Vec<Value>> = OnceLock::new();
    let table = INITIAL.get_or_init(|| {
        (Property::ALL.iter())
            .map(|property| {
                property
                    .parse_value(property.initial())
                    .expect("every initial value parses")
            })
            .collect()
    });
    let data = tree.node(node);
    let element = data.element;
    let kind = data.attribute_str("type");
    let channel = data.attribute_str("channel");
    let text = match (property, element) {
        (Property::Orient, Element::Axis) => match channel {
            Some("y" | "radius") => Some("left"),
            _ => Some("bottom"),
        },
        (Property::Orient, Element::Legend) => Some("right"),
        (Property::Orient, Element::Header | Element::Title) => Some("top"),
        (Property::Direction, Element::Legend) => match style.keyword(Property::Orient) {
            Some("top" | "bottom") => Some("horizontal"),
            _ => Some("vertical"),
        },
        (Property::Axes, Element::Plot) => match kind {
            Some(kind) if kind.starts_with("facet_") => Some("outer"),
            _ => Some("each"),
        },
        (Property::Columns, Element::Plot) => match kind {
            Some("hconcat") => Some("none"),
            _ => None,
        },
        (Property::MaxLines, Element::Mark | Element::Description) => Some("none"),
        (Property::Display, Element::Description) => Some("none"),
        (Property::LabelPosition, Element::Mark) if kind == Some("symbol") => Some("top"),
        (Property::Size, Element::Mark) if kind == Some("text") => Some("11"),
        (Property::StrokeWidth, Element::Mark) if kind == Some("line") => Some("2"),
        (Property::LabelOverlap, Element::Axis) => {
            match (data.attribute_str("scale"), data.attribute_str("kind")) {
                (Some("log"), _) => Some("greedy"),
                (_, Some("sequential" | "diverging")) => Some("parity"),
                _ => None,
            }
        }
        (Property::LabelFlush, Element::Axis) => match (channel, data.attribute_str("kind")) {
            (Some("x"), Some("sequential" | "diverging")) => Some("true"),
            _ => None,
        },
        (Property::PaddingOuter, Element::Scale) if kind == Some("point") => Some("0.5"),
        _ => None,
    };
    match text {
        Some(text) => property.parse_value(text).expect("an initial value parses"),
        None => table[property as usize].clone(),
    }
}

/// How `auto` paints a node, by its element and the kind of its mark.
fn auto_paint(tree: &StyleTree, node: NodeId) -> AutoPaint {
    let data = tree.node(node);
    match data.element {
        Element::Mark => match data.attribute_str("type") {
            Some("line" | "rule") => AutoPaint::Stroke,
            _ => AutoPaint::Fill,
        },
        Element::Label => match tree.parent(node).map(|parent| tree.node(parent).element) {
            Some(Element::Mark) => AutoPaint::MarkLabel,
            _ => AutoPaint::Fill,
        },
        Element::Title
        | Element::Subtitle
        | Element::Footer
        | Element::Description
        | Element::Swatch
        | Element::Ramp
        | Element::Attribution => AutoPaint::Fill,
        Element::Tick | Element::Rule | Element::Grid => AutoPaint::Stroke,
        _ => AutoPaint::Nothing,
    }
}

/// The sRGB RGBA of a specified color, with `current` as `currentColor`. A missing component
/// and a missing alpha draw as zero.
fn resolve_color(color: &Color, current: [f32; 4], host: &HostInputs) -> [f32; 4] {
    let (mut color, missing) = absolute(color, current, host);
    for (index, missing) in missing.into_iter().enumerate() {
        if missing {
            set_component(&mut color, index, 0.0);
        }
    }
    color.to_rgba()
}

/// The absolute color of a specified color, with its missing components in the order of its
/// components.
fn absolute(color: &Color, current: [f32; 4], host: &HostInputs) -> (AbsoluteColor, [bool; 4]) {
    match color {
        Color::Absolute { color, missing } => (*color, *missing),
        Color::CurrentColor => (AbsoluteColor::from_rgba(current), [false; 4]),
        Color::System(system) => (
            AbsoluteColor::from_rgba(host.preferences.system_color(*system)),
            [false; 4],
        ),
        Color::Contrast(color) => (
            choose_contrast_color(&absolute(color, current, host).0),
            [false; 4],
        ),
        Color::Mix { space, hue, colors } => {
            let [(first, p1), (second, p2)] = colors.as_ref();
            let (w1, w2) = match (p1, p2) {
                (None, None) => (0.5, 0.5),
                (Some(p1), None) => (*p1, 1.0 - p1),
                (None, Some(p2)) => (1.0 - p2, *p2),
                (Some(p1), Some(p2)) => (*p1, *p2),
            };
            let total = w1 + w2;
            let (mut left, left_missing) = in_space(absolute(first, current, host), *space);
            let (mut right, right_missing) = in_space(absolute(second, current, host), *space);
            // A component that one color lacks takes the other color's value, as CSS Color 4
            // interpolates, and one that both lack stays missing. Two missing alphas interpolate
            // the colors without premultiplying them, as alphas of 1 do.
            let mut missing = [false; 4];
            for index in 0..4 {
                match (left_missing[index], right_missing[index]) {
                    (true, false) => set_component(&mut left, index, component(&right, index)),
                    (false, true) => set_component(&mut right, index, component(&left, index)),
                    (true, true) => {
                        let value = if index == 3 { 1.0 } else { 0.0 };
                        set_component(&mut left, index, value);
                        set_component(&mut right, index, value);
                        missing[index] = true;
                    }
                    (false, false) => {}
                }
            }
            let mut mixed = mix_colors(*space, &left, w1, &right, w2, *hue);
            // A missing component of the mix is zero, also a hue that `longer` interpolation
            // moves away from zero.
            for (index, missing) in missing.iter().take(3).enumerate() {
                if *missing {
                    set_component(&mut mixed, index, 0.0);
                }
            }
            // Weights that sum to less than 100% make the mix translucent, as in CSS.
            if total < 1.0 {
                mixed.alpha *= total;
            }
            (mixed, missing)
        }
    }
}

/// The kinds of components that CSS Color 4 carries across color spaces when a color with a
/// missing component converts for interpolation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Analog {
    Red,
    Green,
    Blue,
    Lightness,
    Colorfulness,
    Hue,
    OpponentA,
    OpponentB,
}

fn analogs(space: ColorSpace) -> [Option<Analog>; 3] {
    use Analog::*;
    match space {
        ColorSpace::Srgb | ColorSpace::SrgbLinear => [Some(Red), Some(Green), Some(Blue)],
        ColorSpace::Hsl => [Some(Hue), Some(Colorfulness), Some(Lightness)],
        ColorSpace::Hwb => [Some(Hue), None, None],
        ColorSpace::Lab | ColorSpace::Oklab => [Some(Lightness), Some(OpponentA), Some(OpponentB)],
        ColorSpace::Lch | ColorSpace::Oklch => [Some(Lightness), Some(Colorfulness), Some(Hue)],
    }
}

/// A color in `space`, with the components that are missing there: its own missing components
/// when it is in `space`, and otherwise those analogous to a missing component of the color, and
/// the alpha when it is missing.
fn in_space(
    (color, missing): (AbsoluteColor, [bool; 4]),
    space: ColorSpace,
) -> (AbsoluteColor, [bool; 4]) {
    if color.color_space == space {
        return (color, missing);
    }
    let from = analogs(color.color_space);
    let to = analogs(space);
    let mut carried = [false, false, false, missing[3]];
    for (index, analog) in to.iter().enumerate() {
        carried[index] = analog.is_some_and(|analog| {
            (0..3).any(|source| missing[source] && from[source] == Some(analog))
        });
    }
    (color.to_color_space(space), carried)
}

fn component(color: &AbsoluteColor, index: usize) -> f32 {
    match index {
        3 => color.alpha,
        index => color.components[index],
    }
}

fn set_component(color: &mut AbsoluteColor, index: usize, value: f32) {
    match index {
        3 => color.alpha = value,
        index => color.components[index] = value,
    }
}

/// The computed custom properties of a node from its declarations, `specified`, each with its
/// text and the index of its stylesheet's `env()` values, and the values that it inherits. A
/// declaration substitutes the computed values of the node's other custom properties and its own
/// stylesheet's `env()` values, so a child inherits a value without references. Custom properties
/// that refer to each other in a cycle, also through a fallback, are invalid, as is one whose
/// references have no value and no fallback, and an invalid property has no value.
fn custom_properties(
    specified: &BTreeMap<&str, (&str, usize)>,
    mut inherited: BTreeMap<String, String>,
    envs: &[&Env],
) -> BTreeMap<String, String> {
    if specified.is_empty() {
        return inherited;
    }
    let references: BTreeMap<&str, Vec<String>> = (specified.iter())
        .map(|(name, (text, _))| (*name, references(text)))
        .collect();
    let cyclic = cycles(&references);
    let mut computed: BTreeMap<String, Option<String>> = BTreeMap::new();
    for name in specified.keys() {
        compute_custom(name, specified, &inherited, &cyclic, envs, &mut computed);
    }
    for (name, value) in computed {
        match value {
            Some(value) => inherited.insert(name, value),
            None => inherited.remove(&name),
        };
    }
    inherited
}

fn compute_custom(
    name: &str,
    specified: &BTreeMap<&str, (&str, usize)>,
    inherited: &BTreeMap<String, String>,
    cyclic: &BTreeSet<String>,
    envs: &[&Env],
    computed: &mut BTreeMap<String, Option<String>>,
) -> Option<String> {
    if let Some(value) = computed.get(name) {
        return value.clone();
    }
    let Some((text, env)) = specified.get(name) else {
        return inherited.get(name).cloned();
    };
    let value = match cyclic.contains(name) {
        true => None,
        false => {
            // The references come first, and they have no cycle, so the recursion ends.
            let mut values = BTreeMap::new();
            for reference in references(text) {
                if let Some(value) =
                    compute_custom(&reference, specified, inherited, cyclic, envs, computed)
                {
                    values.insert(reference, value);
                }
            }
            substitute(text, &values, envs[*env])
        }
    };
    computed.insert(name.to_string(), value.clone());
    value
}

/// The custom properties that `text` reads with `var()`, also in fallbacks.
fn references(text: &str) -> Vec<String> {
    fn walk(input: &mut Parser<'_>, found: &mut Vec<String>) {
        while let Ok(token) = input.next() {
            let token = token.clone();
            let is_var =
                matches!(&token, Token::Function(name) if name.eq_ignore_ascii_case("var"));
            if matches!(
                token,
                Token::Function(_)
                    | Token::ParenthesisBlock
                    | Token::SquareBracketBlock
                    | Token::CurlyBracketBlock
            ) {
                let _ = input.parse_nested_block(|block| {
                    if is_var {
                        if let Ok(name) = block.expect_ident() {
                            found.push(name.to_string());
                        }
                    }
                    walk(block, found);
                    Ok::<_, ParseError<()>>(())
                });
            }
        }
    }
    let mut found = Vec::new();
    walk(&mut Parser::new(text), &mut found);
    found.sort();
    found.dedup();
    found
}

/// The custom properties of `references` that lie on a cycle: in a strongly connected component
/// of more than one property, or referring to themselves.
fn cycles(references: &BTreeMap<&str, Vec<String>>) -> BTreeSet<String> {
    // Tarjan's algorithm over the properties that the node declares.
    struct Search<'a> {
        references: &'a BTreeMap<&'a str, Vec<String>>,
        index: BTreeMap<&'a str, usize>,
        low: BTreeMap<&'a str, usize>,
        stack: Vec<&'a str>,
        on_stack: BTreeSet<&'a str>,
        cyclic: BTreeSet<String>,
    }
    impl<'a> Search<'a> {
        fn visit(&mut self, name: &'a str) {
            let index = self.index.len();
            self.index.insert(name, index);
            self.low.insert(name, index);
            self.stack.push(name);
            self.on_stack.insert(name);
            let references = self.references;
            for reference in &references[name] {
                let Some((reference, _)) = references.get_key_value(reference.as_str()) else {
                    continue;
                };
                if !self.index.contains_key(reference) {
                    self.visit(reference);
                    let low = self.low[name].min(self.low[reference]);
                    self.low.insert(name, low);
                } else if self.on_stack.contains(reference) {
                    let low = self.low[name].min(self.index[reference]);
                    self.low.insert(name, low);
                }
            }
            if self.low[name] == self.index[name] {
                let mut component = Vec::new();
                while let Some(member) = self.stack.pop() {
                    self.on_stack.remove(member);
                    component.push(member);
                    if member == name {
                        break;
                    }
                }
                let refers_to_itself = references[name].iter().any(|reference| reference == name);
                if component.len() > 1 || refers_to_itself {
                    self.cyclic
                        .extend(component.into_iter().map(str::to_string));
                }
            }
        }
    }
    let mut search = Search {
        references,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        cyclic: BTreeSet::new(),
    };
    for name in references.keys() {
        if !search.index.contains_key(name) {
            search.visit(name);
        }
    }
    search.cyclic
}

/// Substitutes the `var()` and `env()` functions of `text` with the computed custom properties
/// `customs` and the values `env`, or `None` when a reference has no value and no fallback.
fn substitute(text: &str, customs: &BTreeMap<String, String>, env: &Env) -> Option<String> {
    let mut input = Parser::new(text);
    let mut out = String::new();
    tokens(&mut input, customs, env, &mut out)?;
    Some(out)
}

fn tokens(
    input: &mut Parser<'_>,
    customs: &BTreeMap<String, String>,
    env: &Env,
    out: &mut String,
) -> Option<()> {
    loop {
        let start = input.position();
        let token = match input.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(_) => return Some(()),
        };
        match token {
            Token::Function(name)
                if name.eq_ignore_ascii_case("var") || name.eq_ignore_ascii_case("env") =>
            {
                let is_var = name.eq_ignore_ascii_case("var");
                let mut replaced = None;
                let _ = input.parse_nested_block(|block| {
                    let reference = block.expect_ident().map(|name| name.to_string());
                    let fallback = match block.next() {
                        Ok(Token::Comma) => {
                            let start = block.position();
                            while block.next_including_whitespace_and_comments().is_ok() {}
                            Some(block.slice_from(start).trim().to_string())
                        }
                        _ => None,
                    };
                    replaced = reference.ok().and_then(|reference| {
                        let value = match is_var {
                            true => customs.get(&reference).cloned(),
                            false => env.get(&reference).cloned(),
                        };
                        value.or_else(|| {
                            fallback.and_then(|fallback| substitute(&fallback, customs, env))
                        })
                    });
                    Ok::<_, ParseError<()>>(())
                });
                out.push(' ');
                out.push_str(&replaced?);
                out.push(' ');
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                out.push_str(input.slice_from(start));
                let closer = match token {
                    Token::SquareBracketBlock => ']',
                    Token::CurlyBracketBlock => '}',
                    _ => ')',
                };
                let mut inner = String::new();
                let mut failed = false;
                let _ = input.parse_nested_block(|block| {
                    failed = tokens(block, customs, env, &mut inner).is_none();
                    Ok::<_, ParseError<()>>(())
                });
                if failed {
                    return None;
                }
                out.push_str(&inner);
                out.push(closer);
            }
            _ => out.push_str(input.slice_from(start)),
        }
    }
}

/// The palettes of the rule sets by cascade precedence, then the built-in palettes that they do
/// not redefine.
fn palettes(
    sets: &[&RuleSet],
    keys: &[LayerKeys],
    envs: &[&Env],
    host: &HostInputs,
) -> (Vec<Palette>, Vec<StyleWarning>) {
    let mut sources: Vec<(&PaletteSource, &[u32])> = (sets.iter().zip(keys))
        .flat_map(|(set, keys)| (set.palettes.iter()).zip(keys.palettes.iter().map(Vec::as_slice)))
        .collect();
    sources.sort_by(|(a, a_layer), (b, b_layer)| {
        rank(a.origin, false)
            .cmp(&rank(b.origin, false))
            .then(a_layer.cmp(b_layer))
            .then(a.order.cmp(&b.order))
    });
    let mut defined: Vec<Palette> = Vec::new();
    let mut warnings = Vec::new();
    for (source, _) in sources.into_iter().rev() {
        let rule = &source.rule;
        if defined.iter().any(|palette| palette.name == rule.name) {
            continue;
        }
        let colors = match &rule.colors {
            Declared::Value(Value::List(colors)) => (colors.iter())
                .filter_map(|color| match color {
                    Value::Color(color) => Some(color.clone()),
                    _ => None,
                })
                .collect(),
            // A palette with `env()` is checked after substitution, as a literal one is when it
            // parses, and an invalid one is a warning and defines no palette.
            Declared::Pending { text, .. } => {
                let warn = |message: String| StyleWarning::Palette {
                    name: rule.name.clone(),
                    span: rule.span,
                    message,
                };
                let Some(text) = substitute(text, &BTreeMap::new(), envs[source.env]) else {
                    warnings.push(warn(format!(
                        "the colors of @palette `{}` read an env() name without a value",
                        rule.name
                    )));
                    continue;
                };
                let mut input = Parser::new(&text);
                let colors = input
                    .parse_comma_separated(|input| {
                        parse_color(input).map_err(|()| ParseError::<()>::custom(()))
                    })
                    .ok()
                    .filter(|_| input.is_exhausted());
                let Some(colors) = colors else {
                    warnings.push(warn(format!(
                        "`{}` is not a list of colors for @palette `{}`",
                        text.trim(),
                        rule.name
                    )));
                    continue;
                };
                let (least, message) = rule.kind.min_colors();
                if colors.len() < least {
                    warnings.push(warn(message.to_string()));
                    continue;
                }
                colors
            }
            _ => continue,
        };
        defined.push(Palette {
            name: rule.name.clone(),
            kind: rule.kind,
            colors: (colors.iter())
                .map(|color| resolve_color(color, [0.0, 0.0, 0.0, 1.0], host))
                .collect(),
            interpolate: rule.interpolate,
            cvd_safe: rule.cvd_safe,
        });
    }
    for palette in Palette::builtin() {
        if !defined.iter().any(|defined| defined.name == palette.name) {
            defined.push(palette);
        }
    }
    (defined, warnings)
}
