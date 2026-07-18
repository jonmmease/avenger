fn main() {
    if let Err(error) = avenger_lang_cli::run() {
        eprintln!("avenger: {error}");
        std::process::exit(1);
    }
}
