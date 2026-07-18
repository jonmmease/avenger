use avenger_chart_lang_registry::builtins::stock_registry;

fn main() {
    let registry = stock_registry().expect("stock v1 registry must build");
    println!("{}", registry.snapshot().markdown_reference());
}
