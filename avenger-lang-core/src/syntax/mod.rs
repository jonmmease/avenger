//! Strict, schema-free parsing and lossless concrete source ownership.

mod format;
mod parser;
mod tolerant;

pub use format::{format_parsed, format_source};
pub use parser::{
    ConcreteFile, ConcreteNode, ConcreteNodeKind, ParseError, ParsedFile, SqlIslandContext,
    ImportClauseSyntax, ImportSpecifierSyntax, ImportSyntax, ModuleItemSyntax, ModuleSyntaxMap,
    QualifiedNameSyntax, SqlIslandRoot, SqlIslandSite, SyntaxLimits, SyntaxNodeId, parse_file,
    parse_file_with_limits,
};
pub use tolerant::{
    ParseMode, ParseModeOutput, TolerantImportClauseSyntax, TolerantImportSpecifierSyntax,
    TolerantImportSyntax, TolerantModuleItemSyntax, TolerantModuleSyntax, TolerantParsedFile,
    TolerantRecoveryContext, TolerantRecoverySyntax, TolerantSpannedText, TolerantSyntaxNode,
    TolerantSyntaxNodeId, TolerantSyntaxNodeKind, TolerantVersionSyntax, parse_file_tolerant,
    parse_file_tolerant_with_limits, parse_file_with_mode,
};
