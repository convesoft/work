//! Initial CLI adapter. Item and MCP commands belong to later Work items.

use std::ffi::OsString;
use std::path::Path;

use work::core::project::discover;

pub fn run(mut args: impl Iterator<Item = OsString>) -> Result<(), String> {
    match args.next().as_deref() {
        Some(command) if command == "--version" => {
            if args.next().is_some() {
                return Err("usage: work --version | work discover [PATH]".into());
            }
            println!("work {}", env!("CARGO_PKG_VERSION"));
        }
        Some(command) if command == "discover" => {
            let selected = args.next();
            if args.next().is_some() {
                return Err("usage: work discover [PATH]".into());
            }
            let project =
                discover(selected.as_deref().map(Path::new)).map_err(|error| error.to_string())?;
            println!("worktree_root={}", project.worktree_root.display());
            println!("git_common_dir={}", project.git_common_dir.display());
        }
        _ => return Err("usage: work --version | work discover [PATH]".into()),
    }
    Ok(())
}
