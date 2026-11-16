//! Subprocess helper with a hard timeout (port of `fgacc.run`).

use anyhow::{bail, Result};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
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
    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    thread::spawn(move || {
        let mut s = Vec::new();
        out.read_to_end(&mut s).ok();
        tx.send((false, s)).ok();
    });
    thread::spawn(move || {
        let mut s = Vec::new();
        err.read_to_end(&mut s).ok();
        tx2.send((true, s)).ok();
    });
    let start = Instant::now();
    let kill = |child: &mut std::process::Child| {
        // shortcut: shells out to `kill` to signal the group (no libc dep, unsafe is forbidden); fine on unix.
        Command::new("kill")
            .args(["-9", &format!("-{}", child.id())])
            .status()
            .ok();
        child.wait().ok();
    };
    let timed_out = || anyhow::anyhow!("timeout after {}s: {}", timeout.as_secs(), cmd.join(" "));
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > timeout {
            kill(&mut child);
            return Err(timed_out());
        }
        thread::sleep(Duration::from_millis(20));
    };
    // A grandchild may keep the pipes open after the child exits; the deadline covers that too.
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    for _ in 0..2 {
        let left = timeout.saturating_sub(start.elapsed());
        match rx.recv_timeout(left) {
            Ok((false, s)) => stdout = s,
            Ok((true, s)) => stderr = s,
            Err(_) => {
                kill(&mut child);
                return Err(timed_out());
            }
        }
    }
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
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
    fn timeout_covers_background_process_holding_pipes() {
        let t = Instant::now();
        let e = run(
            &["sh", "-c", "sleep 30 &"],
            None,
            &[],
            Duration::from_millis(500),
            true,
        )
        .unwrap_err();
        assert!(e.to_string().starts_with("timeout after"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(10));
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
