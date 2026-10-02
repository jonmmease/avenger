use std::collections::HashSet;

use crate::{
    AggregateOp, Bin, BinOutput, BinParams, Mark, PositionFieldDef, SpecError, Transform, UnitSpec,
};

impl UnitSpec {
    /// Checks supported values without reading data or resolving defaults.
    ///
    /// Call after direct Serde deserialization or edits through public fields.
    /// Returns the first error, including its property path.
    pub fn validate(&self) -> Result<(), SpecError> {
        number(
            self.width,
            "width",
            |v| v >= 0.0,
            "expected a nonnegative finite number",
        )?;
        number(
            self.height,
            "height",
            |v| v >= 0.0,
            "expected a nonnegative finite number",
        )?;
        if let Mark::Def(mark) = &self.mark {
            number(
                mark.opacity,
                "mark.opacity",
                |v| (0.0..=1.0).contains(&v),
                "expected a finite number between 0 and 1",
            )?;
            number(
                mark.size,
                "mark.size",
                |v| v >= 0.0,
                "expected a nonnegative finite number",
            )?;
        }
        if let Some(encoding) = &self.encoding {
            for (name, field) in [("x", &encoding.x), ("y", &encoding.y)] {
                if let Some(field) = field {
                    field.validate(&format!("encoding.{name}"))?;
                }
            }
        }
        let mut names = HashSet::new();
        for (i, parameter) in self.params.iter().flatten().enumerate() {
            let name = &parameter.name;
            let mut chars = name.chars();
            let valid = chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                && !matches!(name.as_str(), "datum" | "event" | "item" | "parent");
            require(
                valid,
                &format!("params[{i}].name"),
                "expected a valid parameter name",
            )?;
            require(
                names.insert(name),
                &format!("params[{i}].name"),
                "duplicate parameter name",
            )?;
            number(
                Some(parameter.value),
                &format!("params[{i}].value"),
                |_| true,
                "expected a finite number",
            )?;
        }
        for (i, transform) in self.transform.iter().flatten().enumerate() {
            let path = format!("transform[{i}]");
            match transform {
                Transform::Filter(t) => {
                    if let crate::PredicateOperand::Number(n) = t.filter.gte {
                        number(
                            Some(n),
                            &format!("{path}.filter.gte"),
                            |_| true,
                            "expected a finite number",
                        )?;
                    }
                }
                Transform::Bin(transform) => {
                    require(
                        matches!(transform.bin, Bin::Bool(true) | Bin::Params(_)),
                        &format!("{path}.bin"),
                        "expected true or a bin parameter object",
                    )?;
                    if let Bin::Params(params) = &transform.bin {
                        params.validate(&format!("{path}.bin"))?;
                    }
                    if let BinOutput::Pair([start, end]) = &transform.as_ {
                        require(
                            start != end,
                            &format!("{path}.as"),
                            "bin boundary aliases must be distinct",
                        )?;
                    }
                }
                Transform::Aggregate(transform) => {
                    require(
                        !transform.aggregate.is_empty(),
                        &format!("{path}.aggregate"),
                        "expected at least one aggregate measure",
                    )?;
                    let mut aliases = HashSet::new();
                    for (j, measure) in transform.aggregate.iter().enumerate() {
                        let path = format!("{path}.aggregate[{j}]");
                        require(
                            measure.field.is_some() || measure.op == AggregateOp::Count,
                            &format!("{path}.field"),
                            "field is required unless op is count",
                        )?;
                        require(
                            aliases.insert(&measure.as_),
                            &format!("{path}.as"),
                            "aggregate output aliases must be distinct",
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl PositionFieldDef {
    fn validate(&self, path: &str) -> Result<(), SpecError> {
        require(
            self.field.is_some() || self.aggregate == Some(AggregateOp::Count),
            &format!("{path}.field"),
            "field is required unless aggregate is count",
        )?;
        if let Some(Bin::Params(params)) = self.bin.as_option() {
            params.validate(&format!("{path}.bin"))?;
        }
        if let Some(axis) = self.axis.as_option() {
            number(
                axis.label_angle,
                &format!("{path}.axis.labelAngle"),
                |v| (-360.0..=360.0).contains(&v),
                "expected a finite number between -360 and 360",
            )?;
        }
        Ok(())
    }
}

impl BinParams {
    fn validate(&self, path: &str) -> Result<(), SpecError> {
        number(
            self.maxbins,
            &format!("{path}.maxbins"),
            |v| v >= 2.0,
            "expected a finite number at least 2",
        )?;
        number(
            self.step,
            &format!("{path}.step"),
            |v| v > 0.0,
            "expected a positive finite number",
        )?;
        number(
            self.minstep,
            &format!("{path}.minstep"),
            |v| v >= 0.0,
            "expected a nonnegative finite number",
        )?;
        number(
            self.base,
            &format!("{path}.base"),
            |v| v > 1.0,
            "expected a finite number greater than 1",
        )?;
        number(
            self.anchor,
            &format!("{path}.anchor"),
            |_| true,
            "expected a finite number",
        )?;
        if let Some(steps) = &self.steps {
            require(
                !steps.is_empty(),
                &format!("{path}.steps"),
                "expected at least one step",
            )?;
            for (i, &step) in steps.iter().enumerate() {
                let item_path = format!("{path}.steps[{i}]");
                number(
                    Some(step),
                    &item_path,
                    |v| v > 0.0,
                    "expected a positive finite number",
                )?;
                require(
                    i == 0 || steps[i - 1] < step,
                    &item_path,
                    "steps must be strictly increasing",
                )?;
            }
        }
        if let Some(divide) = &self.divide {
            require(
                (1..=2).contains(&divide.len()),
                &format!("{path}.divide"),
                "expected one or two divisors",
            )?;
            for (i, &divisor) in divide.iter().enumerate() {
                number(
                    Some(divisor),
                    &format!("{path}.divide[{i}]"),
                    |v| v > 1.0,
                    "expected a finite number greater than 1",
                )?;
            }
        }
        if let Some(extent) = self.extent {
            for (i, value) in extent.into_iter().enumerate() {
                number(
                    Some(value),
                    &format!("{path}.extent[{i}]"),
                    |_| true,
                    "expected a finite number",
                )?;
            }
            require(
                extent[0] <= extent[1],
                &format!("{path}.extent"),
                "extent must be ordered from minimum to maximum",
            )?;
        }
        Ok(())
    }
}

fn require(condition: bool, path: &str, message: &str) -> Result<(), SpecError> {
    if condition {
        Ok(())
    } else {
        Err(SpecError::new(path, message))
    }
}

fn number(
    value: Option<f64>,
    path: &str,
    accepts: impl FnOnce(f64) -> bool,
    message: &str,
) -> Result<(), SpecError> {
    match value {
        Some(value) => require(value.is_finite() && accepts(value), path, message),
        None => Ok(()),
    }
}
