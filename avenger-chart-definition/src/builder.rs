use crate::Result;
use crate::{dataflow::*, *};
use dataflow::datafusion::common::ScalarValue;

/// Builds a chart or a group template. Callbacks execute only during construction.
pub struct ChartBuilder {
    flow: Dataflow,
    scope: Vec<String>,
    group: Group,
    parameters: Vec<Parameter>,
}
impl ChartBuilder {
    pub(crate) fn new(flow: Dataflow) -> Self {
        Self::at(
            flow,
            vec![],
            "figure".into(),
            Arrangement::column().margin(16.0),
        )
    }
    fn at(flow: Dataflow, scope: Vec<String>, name: String, arrangement: Arrangement) -> Self {
        Self {
            flow,
            scope,
            group: Group {
                name,
                arrangement,
                title: None,
                children: vec![],
                discovery: None,
                key_order: KeyOrder::Ascending,
            },
            parameters: vec![],
        }
    }
    /// Configure this group's direct-child layout.
    pub fn arrange(&mut self, arrangement: Arrangement) {
        self.group.arrangement = arrangement;
    }
    /// Add a title or a facet-key header.
    pub fn title(&mut self, title: impl Into<Text>) {
        self.group.title = Some(title.into());
    }
    /// Choose the ordering of this facet template's discovered instances.
    pub fn key_order(&mut self, order: KeyOrder) {
        self.group.key_order = order;
    }
    /// Request local discovery when visuals use only ancestor outputs.
    pub fn discover_with(&mut self, output: &ScalarOutput) {
        self.group.discovery = Some(*output);
    }
    /// Publish a scalar input and its chart initial value.
    pub fn parameter(
        &mut self,
        name: impl Into<String>,
        input: &ScalarInput,
        initial: ScalarValue,
    ) -> Result<()> {
        self.add_parameter(name.into(), input, Some(initial))
    }
    /// Publish a scalar input that must be supplied by a render request.
    pub fn required_parameter(
        &mut self,
        name: impl Into<String>,
        input: &ScalarInput,
    ) -> Result<()> {
        self.add_parameter(name.into(), input, None)
    }
    fn add_parameter(
        &mut self,
        name: String,
        input: &ScalarInput,
        initial: Option<ScalarValue>,
    ) -> Result<()> {
        if self.flow.interface().scalar_input_reference(input)?.scope != self.scope {
            return Err(invalid(
                name,
                "parameter must be declared in its defining dataflow scope",
            ));
        }
        self.parameters.push(Parameter {
            name,
            input: input.clone(),
            initial,
        });
        Ok(())
    }
    /// Construct one plot in the current dataflow scope.
    pub fn plot(
        &mut self,
        name: impl Into<String>,
        build: impl FnOnce(&mut PlotBuilder) -> Result<()>,
    ) -> Result<()> {
        let mut plot = PlotBuilder::new(name.into());
        build(&mut plot)?;
        self.group.children.push(Node::Plot(plot.plot));
        Ok(())
    }
    /// Construct a fixed group without introducing a dataflow scope.
    pub fn group(
        &mut self,
        name: impl Into<String>,
        arrangement: Arrangement,
        build: impl FnOnce(&mut ChartBuilder) -> Result<()>,
    ) -> Result<()> {
        let mut child = Self::at(
            self.flow.clone(),
            self.scope.clone(),
            name.into(),
            arrangement,
        );
        build(&mut child)?;
        self.parameters.extend(child.parameters);
        self.group.children.push(Node::Group(child.group));
        Ok(())
    }
    /// Repeat a group template for instances of an immediate child dataflow scope.
    pub fn facet(
        &mut self,
        name: impl Into<String>,
        scope: &ScopeHandle,
        arrangement: Arrangement,
        build: impl FnOnce(&mut ChartBuilder) -> Result<()>,
    ) -> Result<()> {
        let path = self.flow.interface().scope_path(scope)?;
        if path.len() != self.scope.len() + 1 || !path.starts_with(&self.scope) {
            return Err(invalid(
                scope.name(),
                "facet must reference an immediate child dataflow scope",
            ));
        }
        let name = name.into();
        let mut child = Self::at(self.flow.clone(), path, name.clone(), Arrangement::column());
        build(&mut child)?;
        self.parameters.extend(child.parameters);
        self.group.children.push(Node::Facet {
            name,
            scope: scope.clone(),
            arrangement,
            template: child.group,
        });
        Ok(())
    }
    /// Validate the definition without evaluating data or initializing text resources.
    pub fn finish(self) -> Result<ChartDefinition> {
        if !self.scope.is_empty() {
            return Err(invalid("chart", "only the root builder can finish a chart"));
        }
        let definition = ChartDefinition {
            dataflow: self.flow,
            root: self.group,
            parameters: self.parameters,
        };
        definition.validate()?;
        Ok(definition)
    }
}
/// Builder for local plot scales, marks, and guides.
pub struct PlotBuilder {
    pub(crate) plot: Plot,
}
impl PlotBuilder {
    fn new(name: String) -> Self {
        Self {
            plot: Plot {
                identity: crate::model::plot_identity(),
                name,
                size: Size::new(320.0, 220.0),
                clip: true,
                scales: vec![],
                marks: vec![],
                axes: vec![],
                guide_reservations: None,
            },
        }
    }
    /// Set the content rectangle size, excluding guides and margins.
    pub fn content_size(&mut self, width: f32, height: f32) {
        self.plot.size = Size::new(width, height);
    }
    /// Clip mark layers to the content rectangle.
    pub fn clip(&mut self, enabled: bool) {
        self.plot.clip = enabled;
    }
    /// Reserve fixed guide space instead of remeasuring it as domains change.
    pub fn guide_reservations(&mut self, edges: Edges<f32>) {
        self.plot.guide_reservations = Some(edges);
    }
    /// Define a named scale local to this plot.
    pub fn scale(&mut self, name: impl Into<String>, scale: Scale) -> Result<ScaleHandle> {
        let name = name.into();
        if self.plot.scales.iter().any(|(n, _)| n == &name) {
            return Err(invalid(&name, "duplicate scale"));
        }
        let handle = ScaleHandle {
            owner: self.plot.identity,
            name: name.clone(),
        };
        self.plot.scales.push((name, scale));
        Ok(handle)
    }
    /// Add a rectangle layer using an existing table output.
    pub fn rect(
        &mut self,
        name: impl Into<String>,
        table: &TableOutput,
        encoding: RectEncoding,
    ) -> Result<()> {
        self.plot.marks.push(Mark {
            name: name.into(),
            table: *table,
            encoding: Encoding::Rect(encoding),
        });
        Ok(())
    }
    /// Add a circle layer using an existing table output.
    pub fn symbol(
        &mut self,
        name: impl Into<String>,
        table: &TableOutput,
        encoding: SymbolEncoding,
    ) -> Result<()> {
        self.plot.marks.push(Mark {
            name: name.into(),
            table: *table,
            encoding: Encoding::Symbol(encoding),
        });
        Ok(())
    }
    /// Add an axis for a local scale.
    pub fn axis(&mut self, axis: Axis) -> Result<()> {
        self.plot.axes.push(axis);
        Ok(())
    }
}
