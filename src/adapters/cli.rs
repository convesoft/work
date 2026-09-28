//! Initial CLI adapter. Item and MCP commands belong to later Work items.

use std::ffi::OsString;
use std::fmt::Write;
use std::os::unix::ffi::OsStrExt;
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
            println!("worktree_root={}", encode_path(&project.worktree_root));
            println!("git_common_dir={}", encode_path(&project.git_common_dir));
        }
        _ => return Err("usage: work --version | work discover [PATH]".into()),
    }
    Ok(())
}

// Keep each value on one line and preserve every Unix path byte. Percent
// escapes are uppercase hexadecimal; '%' itself is always escaped.
fn encode_path(path: &Path) -> String {
    let mut encoded = String::new();
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'.' | b'_' | b'-' | b'~') {
            encoded.push(*byte as char);
        } else {
            write!(encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}
