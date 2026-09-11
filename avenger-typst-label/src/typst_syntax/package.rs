//! Package specification syntax retained from upstream `typst-syntax`.
//!
//! The label engine does not load Typst packages or parse package manifests.
//! These lightweight types remain because the upstream AST can contain package
//! specifications in import-like syntax, and keeping the same node shape makes
//! the parser subset easier to compare to upstream.

use std::fmt::{self, Debug, Display, Formatter};
use std::str::FromStr;

use ecow::{EcoString, eco_format};
use unscanny::Scanner;

use crate::typst_syntax::is_ident;

/// Identifies a package.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct PackageSpec {
    /// The namespace the package lives in.
    pub namespace: EcoString,
    /// The name of the package within its namespace.
    pub name: EcoString,
    /// The package's version.
    pub version: PackageVersion,
}

impl PackageSpec {
    pub fn versionless(&self) -> VersionlessPackageSpec {
        VersionlessPackageSpec {
            namespace: self.namespace.clone(),
            name: self.name.clone(),
        }
    }
}

impl FromStr for PackageSpec {
    type Err = EcoString;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut s = unscanny::Scanner::new(s);
        let namespace = parse_namespace(&mut s)?.into();
        let name = parse_name(&mut s)?.into();
        let version = parse_version(&mut s)?;
        Ok(Self {
            namespace,
            name,
            version,
        })
    }
}

impl Debug for PackageSpec {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for PackageSpec {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "@{}/{}:{}", self.namespace, self.name, self.version)
    }
}

/// Identifies a package, but not a specific version of it.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct VersionlessPackageSpec {
    /// The namespace the package lives in.
    pub namespace: EcoString,
    /// The name of the package within its namespace.
    pub name: EcoString,
}

impl VersionlessPackageSpec {
    /// Fill in the `version` to get a complete [`PackageSpec`].
    pub fn at(self, version: PackageVersion) -> PackageSpec {
        PackageSpec {
            namespace: self.namespace,
            name: self.name,
            version,
        }
    }
}

impl FromStr for VersionlessPackageSpec {
    type Err = EcoString;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut s = unscanny::Scanner::new(s);
        let namespace = parse_namespace(&mut s)?.into();
        let name = parse_name(&mut s)?.into();
        if !s.done() {
            Err("unexpected version in versionless package specification")?;
        }
        Ok(Self { namespace, name })
    }
}

impl Debug for VersionlessPackageSpec {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for VersionlessPackageSpec {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "@{}/{}", self.namespace, self.name)
    }
}

fn parse_namespace<'s>(s: &mut Scanner<'s>) -> Result<&'s str, EcoString> {
    if !s.eat_if('@') {
        Err("package specification must start with '@'")?;
    }

    let namespace = s.eat_until('/');
    if namespace.is_empty() {
        Err("package specification is missing namespace")?;
    } else if !is_ident(namespace) {
        Err(eco_format!(
            "`{namespace}` is not a valid package namespace"
        ))?;
    }

    Ok(namespace)
}

fn parse_name<'s>(s: &mut Scanner<'s>) -> Result<&'s str, EcoString> {
    s.eat_if('/');

    let name = s.eat_until(':');
    if name.is_empty() {
        Err("package specification is missing name")?;
    } else if !is_ident(name) {
        Err(eco_format!("`{name}` is not a valid package name"))?;
    }

    Ok(name)
}

fn parse_version(s: &mut Scanner) -> Result<PackageVersion, EcoString> {
    s.eat_if(':');

    let version = s.after();
    if version.is_empty() {
        Err("package specification is missing version")?;
    }

    version.parse()
}

/// A package's version.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PackageVersion {
    /// The package's major version.
    pub major: u32,
    /// The package's minor version.
    pub minor: u32,
    /// The package's patch version.
    pub patch: u32,
}

impl FromStr for PackageVersion {
    type Err = EcoString;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('.');
        let mut next = |kind| {
            let part = parts
                .next()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| eco_format!("version number is missing {kind} version"))?;
            part.parse::<u32>()
                .map_err(|_| eco_format!("`{part}` is not a valid {kind} version"))
        };

        let major = next("major")?;
        let minor = next("minor")?;
        let patch = next("patch")?;
        if let Some(rest) = parts.next() {
            Err(eco_format!(
                "version number has unexpected fourth component: `{rest}`"
            ))?;
        }

        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl Debug for PackageVersion {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for PackageVersion {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_spec() {
        let spec: PackageSpec = "@preview/cetz:0.3.4".parse().unwrap();
        assert_eq!(spec.namespace.as_str(), "preview");
        assert_eq!(spec.name.as_str(), "cetz");
        assert_eq!(spec.version.to_string(), "0.3.4");
        assert_eq!(spec.to_string(), "@preview/cetz:0.3.4");
    }
}
