mod adapters;

fn main() {
    std::process::exit(adapters::cli::run(std::env::args_os().skip(1)));
}
