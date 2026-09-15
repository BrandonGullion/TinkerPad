//! A stand-in "language runtime" for runner tests: prints what it was given so
//! tests can check placeholder substitution on every OS without a shell. Not
//! part of the app.
//!
//! Output, one item per line:
//!   arg=<each argument>
//!   cwd=<working directory>
//!   env:<NAME>=<value>   for each NAME listed in TINKERPAD_ECHO_ENV (comma-separated)

fn main() {
    for arg in std::env::args().skip(1) {
        println!("arg={arg}");
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    println!("cwd={}", cwd.display());
    if let Ok(names) = std::env::var("TINKERPAD_ECHO_ENV") {
        for name in names.split(',').filter(|n| !n.is_empty()) {
            let value = std::env::var(name).unwrap_or_else(|_| "<unset>".into());
            println!("env:{name}={value}");
        }
    }
}
