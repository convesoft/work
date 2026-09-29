mod adapters;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.as_slice() == [std::ffi::OsStr::new("mcp")] {
        std::process::exit(adapters::mcp::run());
    }
    std::process::exit(adapters::cli::run(args.into_iter()));
}
