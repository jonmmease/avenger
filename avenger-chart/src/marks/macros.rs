/// Macro to implement common mark methods
#[macro_export]
macro_rules! impl_mark_common {
    ($mark_type:ident, $mark_name:literal) => {
        impl<C: CoordinateSystem> Default for $mark_type<C> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<C: CoordinateSystem> $mark_type<C> {
            pub fn new() -> Self {
                Self {
                    state: $crate::marks::MarkState {
                        data: $crate::transforms::DataContext::default(),
                        data_source: $crate::marks::DataSource::Inherited,
                        facet_strategy: $crate::marks::FacetStrategy::Filter,
                        details: None,
                        zindex: None,
                        shapes: None,
                        adjustments: Vec::new(),
                        derived_marks: Vec::new(),
                    },
                    __phantom: std::marker::PhantomData,
                }
            }

            pub fn data(mut self, dataframe: DataFrame) -> Self {
                self.state.data = $crate::transforms::DataContext::new(dataframe);
                self.state.data_source = $crate::marks::DataSource::Explicit; // Mark as explicit data
                self
            }

            /// Explicitly specify to use plot data (useful for clarity)
            pub fn use_plot_data(mut self) -> Self {
                self.state.data_source = $crate::marks::DataSource::Inherited;
                self
            }

            /// Control faceting behavior
            pub fn facet_strategy(mut self, strategy: $crate::marks::FacetStrategy) -> Self {
                self.state.facet_strategy = strategy;
                self
            }

            /// Convenience method for reference marks that should span all facets
            pub fn broadcast_to_facets(mut self) -> Self {
                self.state.facet_strategy = $crate::marks::FacetStrategy::Broadcast;
                self
            }

            pub fn transform(
                mut self,
                transform: impl $crate::transforms::Transform,
            ) -> Result<Self, $crate::error::AvengerChartError> {
                // Apply transform to existing data context
                let ctx = std::mem::take(&mut self.state.data);

                self.state.data = transform.transform(ctx)?;
                Ok(self)
            }

            pub fn details(mut self, details: Vec<String>) -> Self {
                self.state.details = Some(details);
                self
            }

            pub fn zindex(mut self, zindex: i32) -> Self {
                self.state.zindex = Some(zindex);
                self
            }

            /// Add an adjustment that will be applied to this mark's scaled data
            pub fn adjust(mut self, adjustment: impl $crate::adjust::Adjust + 'static) -> Self {
                self.state.adjustments.push(Box::new(adjustment));
                self
            }

            /// Add a derived mark that will be created from this mark's scaled data
            pub fn derive(mut self, deriver: impl $crate::derive::Derive<C> + 'static) -> Self {
                self.state.derived_marks.push(Box::new(deriver));
                self
            }
        }

        // Don't implement Mark trait here - marks will implement it themselves with specific coordinate systems
    };
}

/// Macro to implement common Mark trait methods
/// Usage: impl_mark_trait_common!(MarkType, CoordSystem, "mark_name")
#[macro_export]
macro_rules! impl_mark_trait_common {
    ($mark_type:ident, $coord:ty, $mark_name:literal) => {
        fn data_context(&self) -> &$crate::transforms::DataContext {
            &self.state.data
        }

        fn data_source(&self) -> $crate::marks::DataSource {
            self.state.data_source.clone()
        }

        fn mark_type(&self) -> &str {
            $mark_name
        }

        fn supported_channels(&self) -> Vec<$crate::marks::ChannelDescriptor> {
            Self::all_channel_descriptors()
        }
    };
}

// encoding_methods! macro removed - use define_common_mark_channels! and define_position_mark_channels! instead
