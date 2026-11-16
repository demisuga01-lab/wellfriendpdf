//! Secrets that must not appear on the process command line.
//!
//! Any process running as the same Unix user can read `/proc/<pid>/cmdline`,
//! so a document password passed as `--password=...` is visible to every
//! concurrent job of that user. `--secrets-file PATH` (or `--secrets-fd N`)
//! supplies the password options as a JSON object instead:
//!
//! ```json
//! {"password": "...", "user-pw": "...", "owner-pw": "...", "insert-password": "..."}
//! ```
//!
//! The values are spliced into the in-memory argument list right after the
//! subcommand that accepts them, so clap parses them exactly like the
//! command-line flags, while `/proc/<pid>/cmdline` only ever shows the path
//! or descriptor number. Error messages never contain a secret value.
//!
//! Rules: the file must not be readable by group or others; every key must be
//! a password option accepted by the selected subcommand; a key may not also
//! be given on the command line; `WELLFRIEND_PDF_REQUIRE_SECRETS_FILE=1`
//! refuses password options on the command line altogether.

use std::ffi::OsString;
use std::io::Read;

/// The password options that may be supplied through the secrets channel.
pub const SECRET_FLAGS: [&str; 4] = ["password", "user-pw", "owner-pw", "insert-password"];

const FILE_FLAG: &str = "--secrets-file";
const FD_FLAG: &str = "--secrets-fd";
/// Upper bound for the secrets document; passwords are short.
const MAX_SECRETS_BYTES: u64 = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct SecretArgsError(pub String);

impl std::fmt::Display for SecretArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(msg: impl Into<String>) -> SecretArgsError {
    SecretArgsError(msg.into())
}

enum Source {
    File(OsString),
    Fd(i32),
}

/// Remove the secrets option from `args`, returning where to read the secrets.
fn take_source(args: &mut Vec<OsString>) -> Result<Option<Source>, SecretArgsError> {
    let mut found: Option<Source> = None;
    let mut i = 1;
    while i < args.len() {
        let Some(arg) = args[i].to_str().map(str::to_owned) else {
            i += 1;
            continue;
        };
        if arg == "--" {
            break;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (arg.clone(), None),
        };
        if flag != FILE_FLAG && flag != FD_FLAG {
            i += 1;
            continue;
        }
        if found.is_some() {
            return Err(err("give --secrets-file or --secrets-fd only once"));
        }
        let value: OsString = match inline {
            Some(v) => {
                args.remove(i);
                OsString::from(v)
            }
            None => {
                if i + 1 >= args.len() {
                    return Err(err(format!("{flag} needs a value")));
                }
                let v = args.remove(i + 1);
                args.remove(i);
                v
            }
        };
        found = Some(if flag == FILE_FLAG {
            Source::File(value)
        } else {
            let fd = value
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
                .filter(|fd| *fd >= 3)
                .ok_or_else(|| err("--secrets-fd needs a descriptor number of 3 or more"))?;
            Source::Fd(fd)
        });
    }
    Ok(found)
}

fn read_limited(reader: impl Read) -> Result<String, SecretArgsError> {
    let mut buf = String::new();
    reader
        .take(MAX_SECRETS_BYTES + 1)
        .read_to_string(&mut buf)
        .map_err(|e| err(format!("could not read the secrets: {}", e.kind())))?;
    if buf.len() as u64 > MAX_SECRETS_BYTES {
        return Err(err("the secrets document is too large"));
    }
    Ok(buf)
}

fn read_source(source: Source) -> Result<String, SecretArgsError> {
    match source {
        Source::File(path) => {
            let file = std::fs::File::open(&path)
                .map_err(|e| err(format!("could not open the secrets file: {}", e.kind())))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = file
                    .metadata()
                    .map_err(|e| err(format!("could not stat the secrets file: {}", e.kind())))?
                    .permissions()
                    .mode();
                if mode & 0o077 != 0 {
                    return Err(err(
                        "the secrets file must not be readable by group or others (use mode 0600)",
                    ));
                }
            }
            read_limited(file)
        }
        Source::Fd(fd) => {
            #[cfg(unix)]
            {
                use std::os::fd::FromRawFd;
                // SAFETY: the caller handed this descriptor to the process for
                // exactly this purpose; it is consumed and closed here.
                let file = unsafe { std::fs::File::from_raw_fd(fd) };
                read_limited(file)
            }
            #[cfg(not(unix))]
            {
                let _ = fd;
                Err(err("--secrets-fd is only supported on Unix"))
            }
        }
    }
}

fn parse_secrets(text: &str) -> Result<Vec<(String, String)>, SecretArgsError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| err("the secrets document is not valid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| err("the secrets document must be a JSON object"))?;
    let mut out = Vec::new();
    for (key, value) in object {
        let flag = key.replace('_', "-");
        if !SECRET_FLAGS.contains(&flag.as_str()) {
            return Err(err(format!("unknown secrets key `{key}`")));
        }
        let text = value
            .as_str()
            .ok_or_else(|| err(format!("secrets key `{key}` must be a string")))?;
        out.push((flag, text.to_owned()));
    }
    Ok(out)
}

fn names_secret_flag(arg: &str) -> Option<&'static str> {
    let name = arg.strip_prefix("--")?;
    let name = name.split_once('=').map_or(name, |(f, _)| f);
    SECRET_FLAGS.iter().copied().find(|f| *f == name)
}

/// Find the deepest subcommand named in `args`, returning it and the index
/// right after its name.
fn selected_subcommand(root: &clap::Command, args: &[OsString]) -> (clap::Command, usize) {
    let mut cmd = root.clone();
    let mut insert_at = 1;
    let mut i = 1;
    while i < args.len() {
        let Some(arg) = args[i].to_str() else {
            i += 1;
            continue;
        };
        if arg == "--" {
            break;
        }
        if !arg.starts_with('-') {
            if let Some(sub) = cmd.find_subcommand(arg).cloned() {
                cmd = sub;
                insert_at = i + 1;
            }
        }
        i += 1;
    }
    (cmd, insert_at)
}

fn accepts_long(cmd: &clap::Command, root: &clap::Command, flag: &str) -> bool {
    let local = cmd.get_arguments().any(|a| a.get_long() == Some(flag));
    let global = root
        .get_arguments()
        .any(|a| a.is_global_set() && a.get_long() == Some(flag));
    local || global
}

/// Expand `--secrets-file` / `--secrets-fd` into in-memory password options.
pub fn expand(
    root: &clap::Command,
    mut args: Vec<OsString>,
) -> Result<Vec<OsString>, SecretArgsError> {
    let require_channel =
        std::env::var("WELLFRIEND_PDF_REQUIRE_SECRETS_FILE").as_deref() == Ok("1");
    let source = take_source(&mut args)?;
    let on_command_line: Vec<&'static str> = args
        .iter()
        .take_while(|a| a.to_str() != Some("--"))
        .filter_map(|a| a.to_str().and_then(names_secret_flag))
        .collect();
    if require_channel {
        if let Some(flag) = on_command_line.first() {
            return Err(err(format!(
                "--{flag} is refused on the command line; pass it through --secrets-file or --secrets-fd"
            )));
        }
    }
    let Some(source) = source else {
        return Ok(args);
    };
    let secrets = parse_secrets(&read_source(source)?)?;
    let (cmd, insert_at) = selected_subcommand(root, &args);
    let mut injected = Vec::new();
    for (flag, value) in secrets {
        if on_command_line.contains(&flag.as_str()) {
            return Err(err(format!(
                "--{flag} is given both on the command line and in the secrets"
            )));
        }
        if !accepts_long(&cmd, root, &flag) {
            return Err(err(format!(
                "the `{}` command does not take --{flag}",
                cmd.get_name()
            )));
        }
        let mut arg = OsString::from(format!("--{flag}="));
        arg.push(value);
        injected.push(arg);
    }
    args.splice(insert_at..insert_at, injected);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Arg, Command};

    fn root() -> Command {
        Command::new("wellfriendpdf")
            .subcommand(
                Command::new("encrypt")
                    .arg(Arg::new("pdf"))
                    .arg(Arg::new("user-pw").long("user-pw"))
                    .arg(Arg::new("password").long("password")),
            )
            .subcommand(Command::new("info").arg(Arg::new("pdf")))
    }

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    fn secrets_file(name: &str, body: &str, mode: u32) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "wfpdf-secrets-{}-{name}.json",
            std::process::id()
        ));
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(not(unix))]
        let _ = mode;
        path
    }

    #[test]
    fn secrets_are_spliced_after_the_subcommand_and_the_path_is_all_argv_shows() {
        let p = secrets_file("ok", r#"{"user_pw":"s3cret","password":"open"}"#, 0o600);
        let out = expand(
            &root(),
            os(&["wellfriendpdf", "encrypt", "--secrets-file", p.to_str().unwrap(), "in.pdf"]),
        )
        .unwrap();
        let out: Vec<String> = out.into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(out[0..2], ["wellfriendpdf", "encrypt"]);
        assert!(out.contains(&"--user-pw=s3cret".to_string()));
        assert!(out.contains(&"--password=open".to_string()));
        assert_eq!(out.last().unwrap(), "in.pdf");
        assert!(!out.iter().any(|a| a.starts_with("--secrets")));
        let _ = std::fs::remove_file(p);
    }

    #[cfg(unix)]
    #[test]
    fn a_group_or_world_readable_secrets_file_is_refused() {
        let p = secrets_file("loose", r#"{"password":"x"}"#, 0o644);
        let e = expand(
            &root(),
            os(&["wellfriendpdf", "encrypt", "--secrets-file", p.to_str().unwrap()]),
        )
        .unwrap_err();
        assert!(e.0.contains("0600"), "{e}");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn errors_never_contain_the_secret_value() {
        let p = secrets_file("wrongcmd", r#"{"password":"TOPSECRET-VALUE"}"#, 0o600);
        let e = expand(
            &root(),
            os(&["wellfriendpdf", "info", "--secrets-file", p.to_str().unwrap()]),
        )
        .unwrap_err();
        assert!(!e.0.contains("TOPSECRET"), "{e}");
        let p2 = secrets_file("dup", r#"{"password":"TOPSECRET-VALUE"}"#, 0o600);
        let e = expand(
            &root(),
            os(&[
                "wellfriendpdf",
                "encrypt",
                "--password",
                "x",
                "--secrets-file",
                p2.to_str().unwrap(),
            ]),
        )
        .unwrap_err();
        assert!(e.0.contains("both") && !e.0.contains("TOPSECRET"), "{e}");
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p2);
    }

    #[test]
    fn unknown_keys_and_non_string_values_are_refused() {
        let p = secrets_file("unknown", r#"{"output":"x"}"#, 0o600);
        assert!(expand(
            &root(),
            os(&["wellfriendpdf", "encrypt", "--secrets-file", p.to_str().unwrap()])
        )
        .is_err());
        let q = secrets_file("num", r#"{"password":1}"#, 0o600);
        assert!(expand(
            &root(),
            os(&["wellfriendpdf", "encrypt", "--secrets-file", q.to_str().unwrap()])
        )
        .is_err());
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(q);
    }

    #[test]
    fn without_a_secrets_option_the_arguments_are_unchanged() {
        let args = os(&["wellfriendpdf", "encrypt", "in.pdf", "--user-pw", "x"]);
        assert_eq!(expand(&root(), args.clone()).unwrap(), args);
    }
}
