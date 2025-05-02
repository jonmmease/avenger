use std::{collections::HashMap, ffi::OsStr, path::{Path, PathBuf}};

use crate::{ast::AvengerFile, error::AvengerLangError, parser::AvengerParser};


pub trait AvengerLoader {
    fn load_file(&self, component_name: &str, from_path: &str) -> Result<AvengerFile, AvengerLangError>;
}

/// Loads Avenger files from the filesystem
#[derive(Debug, Clone)]
pub struct AvengerFilesystemLoader {
    // absolute path to the project base directory
    base_path: PathBuf,
    verbose: bool,
}

impl AvengerFilesystemLoader {
    pub fn new(base_path: &Path, verbose: bool) -> Self {
        Self { base_path: base_path.canonicalize().unwrap(), verbose }
    }
}

impl AvengerLoader for AvengerFilesystemLoader {
    /// Load an Avenger file from the filesystem
    ///     path: path to the file. If relative, may not refer to file outside of the base path
    fn load_file(&self, component_name: &str, from_path: &str) -> Result<AvengerFile, AvengerLangError> {
        let path = Path::new(from_path);
        if path.is_absolute() {
            return Err(AvengerLangError::InternalError(
                "Import form absolute path not implemented yet".to_string())
            );
        }

        let abs_from_path = self.base_path.join(path).canonicalize().unwrap();
        if !abs_from_path.starts_with(&self.base_path) {
            return Err(AvengerLangError::InternalError(
                format!(
                    "Import {} from outside of base path ({}) not allowed", 
                    abs_from_path.to_str().unwrap(), 
                    self.base_path.to_str().unwrap()
                )
            ));
        }

        if !abs_from_path.exists() {
            return Err(AvengerLangError::FileNotFoundError(abs_from_path));
        }

        let abs_file_path = abs_from_path.join(format!("{}.avgr", component_name));
        let src = std::fs::read_to_string(&abs_file_path)?;

        let mut parser = AvengerParser::new(
            &src, component_name, abs_from_path.to_str().unwrap()
        )?;

        match parser.parse() {
            Ok(file) => Ok(file),
            Err(e) => {
                if self.verbose {
                    let _ = e.pretty_print(&src, &format!("{}.avgr", component_name));
                }
                return Err(e);
            }
        }
    }
}

/// Loads Avenger files from memory
pub struct AvengerMemoryLoader {
    // (component name, from path) -> file ast
    files: HashMap<(String, String), AvengerFile>,
}

impl AvengerMemoryLoader {
    pub fn new<S: Into<String>>(files: impl Iterator<Item = (S, S, AvengerFile)>) -> Self {
        Self { files: files.map(
            |(component_name, from_path, file)| ((component_name.into(), from_path.into()), file)
        ).collect() }
    }
}

impl AvengerLoader for AvengerMemoryLoader {
    fn load_file(&self, component_name: &str, from_path: &str) -> Result<AvengerFile, AvengerLangError> {
        let file = self.files.get(&(component_name.to_string(), from_path.to_string()))
            .ok_or(AvengerLangError::FileNotFoundError(PathBuf::from(from_path).join(component_name)))?;
        Ok(file.clone())
    }
}
