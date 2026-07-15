use avenger_chart_lang_registry::builtins::bootstrap_registry;

fn main() {
    let registry = bootstrap_registry().expect("bootstrap registry must build");
    println!("---JSON---");
    println!(
        "{}",
        String::from_utf8(registry.canonical_schema_json().unwrap()).unwrap()
    );
    println!("---MARKDOWN---");
    print!("{}", registry.snapshot().markdown_reference());
}
