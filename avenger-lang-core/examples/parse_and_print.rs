use std::{env, fs, path::PathBuf};

use avenger_lang_core::{
    SourceFile, SourceId, SourceOrigin, interchange::canonical_json, print::print_file,
    syntax::parse_file,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: parse_and_print <chart.avenger>")?;
    let source = SourceFile::new(
        SourceId::new(0),
        SourceOrigin::File(path.clone()),
        fs::read_to_string(path)?,
    );
    let parsed = parse_file(&source)?;
    println!("{}", canonical_json(&parsed.ast)?);
    println!("\n{}", print_file(&parsed.ast));
    Ok(())
}
