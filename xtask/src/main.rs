mod import_translations;
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("import-translations") => {
            let source = args
                .windows(2)
                .find(|p| p[0] == "--source")
                .map(|p| p[1].as_str())
                .ok_or("Use --source <MO directory>")?;
            import_translations::run(std::path::Path::new(source))
        }
        _ => Err("Use: cargo xtask import-translations --source <MO directory>".into()),
    }
}
