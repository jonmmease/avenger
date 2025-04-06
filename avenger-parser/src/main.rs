use avenger_parser::parse;
use std::env;
use std::fs;
use std::process;

fn main() {
    // If a file was provided, parse it
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        // No file provided, show usage and a simple example
        eprintln!("Usage: {} <file>", args[0]);
        
        // Simple inline example when no file is provided
        let example = r#"
Chart {
    width: 100;
    height: 100;
    
    Rule {
        x: 10;
        y: 20;
    }
}
        "#;
        
        match parse(example) {
            Ok(file) => {
                println!("Successfully parsed example with {} component(s)", file.components.len());
                // Display the component names
                for comp in &file.components {
                    println!("Component: {}{}", 
                        if comp.exported { "export " } else { "" },
                        comp.component.name);
                }
            }
            Err(e) => {
                eprintln!("Parse error: {}", e);
                process::exit(1);
            }
        }
        
        process::exit(0);
    }
    
    // Parse the provided file
    let filename = &args[1];
    let source = match fs::read_to_string(filename) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading file {}: {}", filename, e);
            process::exit(1);
        }
    };
    
    // Parse the source code
    match parse(&source) {
        Ok(file) => {
            println!("Successfully parsed {} with {} component(s) and {} enum(s)", 
                filename, file.components.len(), file.enums.len());
            
            // Display import information
            if !file.imports.is_empty() {
                println!("Imports:");
                for import in &file.imports {
                    let components = import.components.join(", ");
                    println!("  {} from '{}'", components, import.path);
                }
                println!("");
            }
            
            // Display enum definitions
            if !file.enums.is_empty() {
                println!("Enums:");
                for enum_def in &file.enums {
                    let export_prefix = if enum_def.exported { "export " } else { "" };
                    let values_str = enum_def.values.join("', '");
                    println!("  {}{}: [ '{}' ]", export_prefix, enum_def.name, values_str);
                }
                println!("");
            }
            
            // Display the component names
            for comp in &file.components {
                println!("Component: {}{}", 
                    if comp.exported { "export " } else { "" },
                    comp.component.name);
                
                // Count item types
                let properties = comp.component.items.iter()
                    .filter(|item| matches!(item, avenger_parser::ComponentItem::Property(_)))
                    .count();
                
                let parameters = comp.component.items.iter()
                    .filter(|item| matches!(item, avenger_parser::ComponentItem::Parameter(_)))
                    .count();
                
                let nested = comp.component.items.iter()
                    .filter(|item| matches!(item, avenger_parser::ComponentItem::ComponentInstance(_)))
                    .count();
                
                let bindings = comp.component.items.iter()
                    .filter(|item| matches!(item, avenger_parser::ComponentItem::ComponentBinding(_, _)))
                    .count();
                
                let if_statements = comp.component.items.iter()
                    .filter(|item| matches!(item, avenger_parser::ComponentItem::IfStatement(_)))
                    .count();
                
                let match_statements = comp.component.items.iter()
                    .filter(|item| matches!(item, avenger_parser::ComponentItem::MatchStatement(_)))
                    .count();
                
                println!("  Contains: {} properties, {} parameters, {} nested components, {} bindings, {} if statements, {} match statements",
                    properties, parameters, nested, bindings, if_statements, match_statements);
            }
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
            process::exit(1);
        }
    }
} 