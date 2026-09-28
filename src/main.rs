mod adapters;

fn main() {
    if let Err(error) = adapters::cli::run(std::env::args_os().skip(1)) {
        eprintln!("work: {error}");
        std::process::exit(1);
    }
}
