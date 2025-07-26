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
                    config: MarkConfig {
                        mark_type: $mark_name.to_string(),
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
                self.config.data = $crate::transforms::DataContext::new(dataframe);
                self.config.data_source = $crate::marks::DataSource::Explicit; // Mark as explicit data
                self
            }

            /// Explicitly specify to use plot data (useful for clarity)
            pub fn use_plot_data(mut self) -> Self {
                self.config.data_source = $crate::marks::DataSource::Inherited;
                self
            }

            /// Control faceting behavior
            pub fn facet_strategy(mut self, strategy: $crate::marks::FacetStrategy) -> Self {
                self.config.facet_strategy = strategy;
                self
            }

            /// Convenience method for reference marks that should span all facets
            pub fn broadcast_to_facets(mut self) -> Self {
                self.config.facet_strategy = $crate::marks::FacetStrategy::Broadcast;
                self
            }

            pub fn transform(
                mut self,
                transform: impl $crate::transforms::Transform,
            ) -> Result<Self, $crate::error::AvengerChartError> {
                // Apply transform to existing data context
                let ctx = std::mem::take(&mut self.config.data);

                self.config.data = transform.transform(ctx)?;
                Ok(self)
            }

            pub fn details(mut self, details: Vec<String>) -> Self {
                self.config.details = Some(details);
                self
            }

            pub fn zindex(mut self, zindex: i32) -> Self {
                self.config.zindex = Some(zindex);
                self
            }

            /// Add an adjustment that will be applied to this mark's scaled data
            pub fn adjust(mut self, adjustment: impl $crate::adjust::Adjust + 'static) -> Self {
                self.config.adjustments.push(Box::new(adjustment));
                self
            }

            /// Add a derived mark that will be created from this mark's scaled data
            pub fn derive(mut self, deriver: impl $crate::derive::Derive<C> + 'static) -> Self {
                self.config.derived_marks.push(Box::new(deriver));
                self
            }
        }

        impl<C: CoordinateSystem> Mark<C> for $mark_type<C> {
            fn into_config(self) -> MarkConfig<C> {
                self.config
            }
        }
    };
}

/// Macro to implement encoding methods
#[macro_export]
macro_rules! encoding_methods {
    ($($method:ident),* $(,)?) => {
        $(
            pub fn $method<V: Into<ChannelValue>>(mut self, value: V) -> Self {
                let channel_value = value.into();

                // Update encodings in DataContext
                self.config.data = self.config.data.with_channel_value(stringify!($method), channel_value);

                self
            }
        )*
    };
}
