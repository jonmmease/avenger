# Foundation API migration

## Color and formatting

Import `ColorOrGradient`, `Gradient`, `GradientStop`, `LinearGradient`, and `RadialGradient` from `avenger_color`. These types previously lived in `avenger_common::types`. Scene marks, guides, scales, and Vega imports use the shared definitions.
