use std::{ffi::OsStr, path::{Path, PathBuf}};

use crate::{ast::AvengerFile, error::AvengerLangError, parser::AvengerParser};


#[derive(Debug, Clone)]
pub struct AvengerFilesystemLoader {
    // absolute path to the project base directory
    base_path: PathBuf,
}

impl AvengerFilesystemLoader {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    /// Load an Avenger file from the filesystem
    ///     path: path to the file. If relative, may not refer to file outside of the base path
    pub fn load_file(&self, path: &str) -> Result<AvengerFile, AvengerLangError> {
        let path = Path::new(path);
        if path.is_absolute() {
            return Err(AvengerLangError::InternalError(
                "Import form absolute path not implemented yet".to_string())
            );
        }

        let abs_path = self.base_path.join(path).canonicalize().unwrap();
        if !abs_path.starts_with(&self.base_path) {
            return Err(AvengerLangError::InternalError(
                "Import from outside of base path not allowed".to_string())
            );
        }

        if !abs_path.exists() {
            return Err(AvengerLangError::FileNotFoundError(abs_path));
        }

        if abs_path.extension() != Some(OsStr::new("avgr")) {
            return Err(AvengerLangError::InvalidFileExtensionError(abs_path));
        }

        // Determine component name (which is file name without the .avgr extension)
        let filename = abs_path.file_name().unwrap().to_str().unwrap().to_string();
        let component_name = filename.strip_suffix(".avgr").unwrap();

        let file = std::fs::read_to_string(&abs_path)?;
        let mut parser = AvengerParser::new(
            &file, component_name, abs_path.to_str().unwrap()
        )?;

        let file = parser.parse()?;
        Ok(file)
    }
}