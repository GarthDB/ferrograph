//! Subprocess helper with a hard timeout (port of `fgacc.run`).

use anyhow::{bail, Result};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// Run `cmd` with a hard timeout; the whole process group is killed on expiry.
/// With `check`, a non-zero exit is an error carrying the tail of stderr.
pub fn run(
    cmd: &[&str],
    cwd: Option<&Path>,
    env: &[(&str, &str)],
    timeout: Duration,
    check: bool,
) -> Result<Output> {
    let mut c = Command::new(cmd[0]);
    c.args(&cmd[1..])
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let mut child = c.spawn()?;
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    // Drain both pipes concurrently so a chatty child cannot block on a full pipe.
    let to = thread::spawn(move || {
        let mut s = Vec::new();
        out.read_to_end(&mut s).ok();
        s
    });
    let te = thread::spawn(move || {
        let mut s = Vec::new();
        err.read_to_end(&mut s).ok();
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > timeout {
            // shortcut: shells out to `kill` to signal the group (no libc dep, unsafe is forbidden); fine on unix.
            Command::new("kill")
                .args(["-9", &format!("-{}", child.id())])
                .status()
                .ok();
            child.wait().ok();
            bail!("timeout after {}s: {}", timeout.as_secs(), cmd.join(" "));
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = String::from_utf8_lossy(&to.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&te.join().unwrap_or_default()).into_owned();
    let code = status.code().unwrap_or(-1);
    if check && code != 0 {
        let tail: String = stderr
            .chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        bail!("exit {code}: {}\n{tail}", cmd.join(" "));
    }
    Ok(Output {
        stdout,
        stderr,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Duration = Duration::from_secs(10);

    #[test]
    fn captures_output_and_checks_exit() {
        let o = run(&["sh", "-c", "echo hi; echo oops >&2"], None, &[], T, true).unwrap();
        assert_eq!(
            (o.stdout.as_str(), o.stderr.as_str(), o.code),
            ("hi\n", "oops\n", 0)
        );
        assert!(run(&["sh", "-c", "exit 3"], None, &[], T, true).is_err());
        assert_eq!(
            run(&["sh", "-c", "exit 3"], None, &[], T, false)
                .unwrap()
                .code,
            3
        );
    }

    #[test]
    fn kills_on_timeout() {
        let e = run(
            &["sleep", "30"],
            None,
            &[],
            Duration::from_millis(200),
            true,
        )
        .unwrap_err();
        assert!(e.to_string().starts_with("timeout after"), "{e}");
    }
}
