/// Macro to implement base mark functionality.
///
/// This lives in core because it only depends on core mark state/data contracts.
/// Coordinate-specific `Mark<C>` implementations still live with the owning
/// coordinate package.
#[macro_export]
macro_rules! impl_mark_base {
    ($mark_type:ident) => {
        $crate::impl_mark_base!(@with_extra_fields $mark_type {});
    };

    (@with_extra_fields $mark_type:ident {$($extra_field:ident: $extra_value:expr),* $(,)?}) => {
        impl<C> Default for $mark_type<C>
        where
            $mark_type<C>: Sized,
        {
            fn default() -> Self {
                Self {
                    state: $crate::MarkState {
                        id: None,
                        data: $crate::DataContext::default(),
                        data_mode: $crate::MarkDataMode::Inherit,
                        facet_data_scope: $crate::FacetDataScope::FILTERED,
                        exclude_from_scale_domains: false,
                        visible: None,
                        details: None,
                        zindex: None,
                        geometry_space: None,
                        axis_configs: std::collections::HashMap::new(),
                    },
                    _phantom: std::marker::PhantomData,
                    $($extra_field: $extra_value,)*
                }
            }
        }

        impl<C> $mark_type<C>
        where
            $mark_type<C>: Sized,
        {
            /// Create a new mark with default settings.
            pub fn new() -> Self {
                Self::default()
            }

            /// Set a structural id used by chart interaction targeting.
            pub fn id(mut self, id: impl Into<String>) -> Self {
                self.state.id = Some(id.into());
                self
            }

            /// Set explicit data for this mark.
            pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
                self.state.data = $crate::DataContext::new(dataframe);
                self.state.data_mode = $crate::MarkDataMode::Inherit;
                self
            }

            /// Set this mark's data source to rows from a mutable chart store.
            pub fn data_store(mut self, data: $crate::StoreData) -> Self {
                self.state.data = $crate::DataContext::store_data(data);
                self.state.data_mode = $crate::MarkDataMode::Inherit;
                self
            }

            /// Render this mark once without inheriting plot or facet data.
            pub fn unit_data(mut self) -> Self {
                self.state.data_mode = $crate::MarkDataMode::Unit;
                self
            }

            /// Prevent this mark's channels from contributing to inferred scale domains.
            pub fn exclude_from_scale_domains(mut self) -> Self {
                self.state.exclude_from_scale_domains = true;
                self
            }

            /// Toggle rendering of this mark with a scalar boolean expression.
            pub fn visible(mut self, visible: impl $crate::IntoExpr) -> Self {
                self.state.visible = Some(
                    $crate::DefaultLogicalExprNodeExt::from_default_expr(visible.into_expr())
                        .expect("Failed to serialize mark visible expr"),
                );
                self
            }

            /// Apply a data transform and configure this mark using the transform output handle.
            pub fn transform<T, F>(mut self, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform,
                F: FnOnce(Self, T::Output) -> Self,
            {
                self.transform_free(transform, f)
            }

            /// Apply a data transform at fully filtered/free facet scope.
            pub fn transform_free<T, F>(self, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform,
                F: FnOnce(Self, T::Output) -> Self,
            {
                self.transform_with_scope($crate::CoordinationScope::Free, transform, f)
            }

            /// Apply a data transform at a specific logical facet sharing level.
            pub fn transform_level<T, F>(self, level: u8, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform,
                F: FnOnce(Self, T::Output) -> Self,
            {
                self.transform_with_scope($crate::CoordinationScope::Level(level), transform, f)
            }

            /// Apply a data transform at shared/global facet scope.
            pub fn transform_shared<T, F>(self, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform,
                F: FnOnce(Self, T::Output) -> Self,
            {
                self.transform_with_scope($crate::CoordinationScope::Shared, transform, f)
            }

            /// Apply a data transform at the specified facet sharing scope.
            pub fn transform_with_scope<T, F>(
                mut self,
                scope: $crate::CoordinationScope,
                transform: T,
                f: F,
            ) -> Self
            where
                T: $crate::DataTransform,
                F: FnOnce(Self, T::Output) -> Self,
            {
                let scope = scope.to_normalized();
                let (compiled_transform, output) = transform
                    .into_compiled_and_output($crate::DataTransformCompileContext::new(scope))
                    .expect("Failed to build data transform");
                self.state.data = self
                    .state
                    .data
                    .with_transform_stage(scope, compiled_transform);
                f(self, output)
            }

            /// Apply a no-output data transform and configure this mark without a dummy output argument.
            pub fn transform_no_output<T, F>(self, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform<Output = ()>,
                F: FnOnce(Self) -> Self,
            {
                self.transform_free_no_output(transform, f)
            }

            /// Apply a no-output data transform at fully filtered/free facet scope.
            pub fn transform_free_no_output<T, F>(self, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform<Output = ()>,
                F: FnOnce(Self) -> Self,
            {
                self.transform_with_scope_no_output($crate::CoordinationScope::Free, transform, f)
            }

            /// Apply a no-output data transform at a specific logical facet sharing level.
            pub fn transform_level_no_output<T, F>(self, level: u8, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform<Output = ()>,
                F: FnOnce(Self) -> Self,
            {
                self.transform_with_scope_no_output(
                    $crate::CoordinationScope::Level(level),
                    transform,
                    f,
                )
            }

            /// Apply a no-output data transform at shared/global facet scope.
            pub fn transform_shared_no_output<T, F>(self, transform: T, f: F) -> Self
            where
                T: $crate::DataTransform<Output = ()>,
                F: FnOnce(Self) -> Self,
            {
                self.transform_with_scope_no_output($crate::CoordinationScope::Shared, transform, f)
            }

            /// Apply a no-output data transform at the specified facet sharing scope.
            pub fn transform_with_scope_no_output<T, F>(
                self,
                scope: $crate::CoordinationScope,
                transform: T,
                f: F,
            ) -> Self
            where
                T: $crate::DataTransform<Output = ()>,
                F: FnOnce(Self) -> Self,
            {
                self.transform_with_scope(scope, transform, |mark, ()| f(mark))
            }

            /// Control faceting behavior.
            pub fn facet_data_scope(mut self, scope: $crate::FacetDataScope) -> Self {
                self.state.facet_data_scope = scope;
                self
            }

            /// Set the faceting data scope by level.
            pub fn facet_data_level(mut self, level: u8) -> Self {
                self.state.facet_data_scope = $crate::FacetDataScope::level(level);
                self
            }

            /// Make this mark appear in all facets.
            pub fn broadcast_to_facets(mut self) -> Self {
                self.state.facet_data_scope = $crate::FacetDataScope::BROADCAST;
                self
            }

            /// Set detail fields for tooltips, interactions, and path partitioning.
            pub fn details<I, S>(mut self, details: I) -> Self
            where
                I: IntoIterator<Item = S>,
                S: Into<String>,
            {
                self.state.details = Some(details.into_iter().map(Into::into).collect());
                self
            }

            /// Set rendering order.
            pub fn zindex(mut self, zindex: i32) -> Self {
                self.state.zindex = Some(zindex);
                self
            }

            /// Get the mark state.
            #[inline]
            pub fn state(&self) -> &$crate::MarkState {
                &self.state
            }

            /// Get mutable mark state.
            #[inline]
            pub fn state_mut(&mut self) -> &mut $crate::MarkState {
                &mut self.state
            }

            /// Get the mark state without colliding with trait method names.
            #[doc(hidden)]
            #[inline]
            pub fn mark_state(&self) -> &$crate::MarkState {
                &self.state
            }

            /// Get mutable mark state without colliding with trait method names.
            #[doc(hidden)]
            #[inline]
            pub fn mark_state_mut(&mut self) -> &mut $crate::MarkState {
                &mut self.state
            }

            /// Get the z-index for rendering.
            #[inline]
            pub fn get_zindex(&self) -> Option<i32> {
                self.state.zindex
            }

            /// Get the data context.
            #[inline]
            pub fn get_data_context(&self) -> &$crate::DataContext {
                &self.state.data
            }

            /// Set a channel value.
            #[doc(hidden)]
            pub fn with_channel_value(mut self, name: &str, value: $crate::ChannelValue) -> Self {
                self.state.data = self.state.data.with_channel_value(name, value);
                self
            }
        }

        impl<C> $crate::IntoPlotMark<C> for $mark_type<C>
        where
            C: $crate::CoordinateSystemCore,
            $mark_type<C>: $crate::Mark<C> + Send + Sync + 'static,
        {
            fn into_plot_marks(self) -> Vec<$crate::PlotMark<C>> {
                vec![$crate::PlotMark::from_mark(self)]
            }
        }
    };
}

/// Implement the standard mark builder surface for primitive marks with
/// additional primitive-owned state.
#[macro_export]
macro_rules! impl_mark_base_with_extra_fields {
    ($mark_type:ident {$($extra_field:ident: $extra_value:expr),* $(,)?}) => {
        $crate::impl_mark_base!(@with_extra_fields $mark_type {$($extra_field: $extra_value),*});
    };
}

/// Macro to implement common `Mark` trait accessors.
#[macro_export]
macro_rules! impl_mark_trait_common {
    ($mark_type:ident, $renderer_type:ident) => {
        fn state(&self) -> &$crate::MarkState {
            self.mark_state()
        }

        fn state_mut(&mut self) -> &mut $crate::MarkState {
            self.mark_state_mut()
        }

        fn data_context(&self) -> &$crate::DataContext {
            self.get_data_context()
        }

        async fn compile(
            &self,
            compiled_state: $crate::CompiledMarkState,
            _session_context: &datafusion::prelude::SessionContext,
        ) -> Result<std::sync::Arc<dyn $crate::CompiledMark>, $crate::AvengerChartError> {
            Ok(std::sync::Arc::new($renderer_type {
                state: compiled_state,
            }))
        }
    };

    ($mark_type:ident) => {
        fn state(&self) -> &$crate::MarkState {
            self.mark_state()
        }

        fn state_mut(&mut self) -> &mut $crate::MarkState {
            self.mark_state_mut()
        }

        fn data_context(&self) -> &$crate::DataContext {
            self.get_data_context()
        }
    };
}

/// Macro to generate a `supported_channels` method from common and position
/// channel names.
///
/// This macro only depends on core channel descriptors, so external custom mark
/// crates can use it without depending on the built-in mark crate or facade.
#[macro_export]
macro_rules! impl_supported_channels {
    (
        common: [$($common:ident),* $(,)?],
        position: [$($position:ident),* $(,)?]
    ) => {
        fn supported_channels(&self) -> Vec<$crate::ChannelDescriptor> {
            vec![
                $(
                    $crate::ChannelDescriptor {
                        name: stringify!($position),
                        required: false,
                        default_value: None,
                        allow_column_ref: true,
                    },
                )*
                $(
                    $crate::ChannelDescriptor {
                        name: stringify!($common),
                        required: false,
                        default_value: None,
                        allow_column_ref: true,
                    },
                )*
            ]
        }
    };
}

/// Macro to define common channels shared across coordinate systems.
///
/// Scale and legend fluent methods are supplied by their owning extension
/// traits; this macro only needs core channel configuration contracts.
#[macro_export]
macro_rules! define_common_mark_channels {
    (@generate_with_method $mark:ident, $name:ident, with_config: $config_type:ty $(,$rest:tt)*) => {
        $crate::__private::paste::paste! {
            pub fn [<$name _with>]<V, F>(self, value: V, f: F) -> Self
            where
                V: Into<$crate::ChannelValue>,
                F: FnOnce($config_type) -> $config_type,
            {
                use $crate::ChannelConfig;

                let channel_value = value.into();
                let config = <$config_type>::new(channel_value);
                let configured = f(config);
                self.with_channel_value(stringify!($name), configured.into_inner())
            }
        }
    };

    (@generate_with_method $mark:ident, $name:ident $(,$rest:tt)*) => {
        // No `_with` method for this channel.
    };

    (
        $mark:ident {
            $(
                $name:ident: {
                    $(default: $default:expr,)?
                    $(allow_column: $allow_column:expr,)?
                    $(required: $required:expr,)?
                    $(with_config: $config_type:ty,)?
                    $(axis_config: $is_axis:expr,)?
                }
            ),* $(,)?
        }
    ) => {
        impl<C> $mark<C>
        where
            $mark<C>: Sized,
        {
            $(
                pub fn $name<V: Into<$crate::ChannelValue>>(self, value: V) -> Self {
                    let channel_value = value.into();
                    self.with_channel_value(stringify!($name), channel_value)
                }

                $crate::define_common_mark_channels!(@generate_with_method
                    $mark,
                    $name
                    $(, with_config: $config_type)?
                    $(, axis_config: $is_axis)?
                );
            )*

            /// Get common channel descriptors for this mark type.
            pub fn common_channel_descriptors() -> Vec<$crate::ChannelDescriptor> {
                vec![
                    $(
                        $crate::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            default_value: None $(.or(Some($default)))?,
                            allow_column_ref: true $(&& $allow_column)?,
                        },
                    )*
                ]
            }
        }
    };
}

/// Macro to define position-specific channels for marks in coordinate systems.
///
/// This lives in core because it only depends on core mark, channel, guide, and
/// coordinate contracts. Coordinate crates can use it to add position-channel
/// builders for their own coordinate systems without depending on the top-level
/// chart facade.
#[macro_export]
macro_rules! define_position_channels {
    (@generate_with_method $mark:ident, $coord:ty, $name:ident, with_config: $config_type:ty) => {
        $crate::__private::paste::paste! {
            pub fn [<$name _with>]<V, F>(self, value: V, f: F) -> Self
            where
                V: Into<$crate::ChannelValue>,
                F: FnOnce($config_type) -> $config_type,
                $config_type: $crate::PositionConfig<Axis = <<$coord as $crate::CoordinateSystem>::Guide as $crate::CoordinateGuide>::Axis>,
            {
                use $crate::PositionConfig;

                let channel_value = value.into();
                let config = <$config_type>::new(channel_value);
                let configured = f(config);

                let (channel_value, axis_config) = configured.take_axis_config();
                let mut mark = self.with_channel_value(stringify!($name), channel_value);

                if let Some(axis_config) = axis_config {
                    mark.state_mut()
                        .axis_configs
                        .insert(stringify!($name).to_string(), std::sync::Arc::new(axis_config));
                }

                mark
            }
        }
    };

    (@generate_with_method $mark:ident, $coord:ty, $name:ident) => {};

    (
        $mark:ident<$coord:ty> {
            $(
                $name:ident: {
                    $(default: $default:expr,)?
                    $(required: $required:expr,)?
                    $(with_config: $config_type:ty,)?
                }
            ),* $(,)?
        }
    ) => {
        impl $mark<$coord> {
            $(
                pub fn $name<V: Into<$crate::ChannelValue>>(self, value: V) -> Self {
                    self.with_channel_value(stringify!($name), value.into())
                }

                $crate::define_position_channels!(@generate_with_method
                    $mark,
                    $coord,
                    $name
                    $(, with_config: $config_type)?
                );
            )*

            /// Get position channel descriptors for this mark type.
            pub fn position_channel_descriptors() -> Vec<$crate::ChannelDescriptor> {
                vec![
                    $(
                        $crate::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            default_value: None $(.or(Some($default)))?,
                            allow_column_ref: true,
                        },
                    )*
                ]
            }

            /// Get all channel descriptors (common + position).
            pub fn all_channel_descriptors() -> Vec<$crate::ChannelDescriptor> {
                let mut descriptors = Self::common_channel_descriptors();
                descriptors.extend(Self::position_channel_descriptors());
                descriptors
            }
        }
    };
}
