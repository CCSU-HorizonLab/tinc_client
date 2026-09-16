// Test-only process fixture. Never package as Tinc.
fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("tinc version 1.0.36 (test fixture)");
        return;
    }
    std::process::exit(43);
}
