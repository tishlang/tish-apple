//! `macos.shell.run(command, { cwd, args, timeoutMs }, cb)` -> id: `/bin/sh -c command` on a
//! background thread; `cb({ id, code, stdout, stderr, ms, timedOut })` on the main thread.
//!
//! - `args` are the script's `$1`, `$2`, … Values are never spliced into `command`.
//! - `cwd` defaults to the home folder. PATH gets the usual tool folders (`/opt/homebrew/bin`, …),
//!   which apps started from Finder lack.
//! - stdout and stderr are capped at 1 MB each. A command still running after `timeoutMs` (default
//!   60 s) is killed along with everything it started.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tishlang_core::Value;

use super::{deliver, hold, num_arg, obj, s};

const MAX_OUTPUT: usize = 1 << 20;

extern "C" {
    fn killpg(pgrp: i32, sig: i32) -> i32;
}

pub(crate) struct Output {
    pub id: u64,
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub ms: f64,
    pub timed_out: bool,
}

fn read_capped(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut r).take(MAX_OUTPUT as u64).read_to_end(&mut buf);
        // Drain the rest so the child never blocks on a full pipe.
        let _ = std::io::copy(&mut r, &mut std::io::sink());
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn path_env() -> String {
    let base = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<&str> = base.split(':').filter(|p| !p.is_empty()).collect();
    for extra in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        if !parts.contains(&extra) {
            parts.push(extra);
        }
    }
    parts.join(":")
}

/// Run `cmd` and wait (off the main thread).
pub(crate) fn run_blocking(
    id: u64,
    cmd: &str,
    cwd: &str,
    args: &[String],
    timeout: Duration,
) -> Output {
    let t0 = Instant::now();
    let fail = |stderr: String| Output {
        id,
        code: 127,
        stdout: String::new(),
        stderr,
        ms: 0.0,
        timed_out: false,
    };
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(cmd)
        .arg("sh")
        .args(args)
        .process_group(0)
        .env("PATH", path_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !cwd.is_empty() && std::path::Path::new(cwd).is_dir() {
        command.current_dir(cwd);
    } else if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("cannot run /bin/sh: {e}")),
    };
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return fail("no output pipes".into());
    };
    let (out, err) = (read_capped(out), read_capped(err));
    let mut timed_out = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if t0.elapsed() > timeout => {
                // The shell leads its own process group: kill the whole group, so a pipeline or a
                // backgrounded child can't hold stdout open and leave the readers waiting.
                unsafe { killpg(child.id() as i32, 9) };
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break -1;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break -1,
        }
    };
    let mut stderr = err.join().unwrap_or_default();
    if timed_out {
        stderr.push_str(&format!("killed after {} ms\n", timeout.as_millis()));
    }
    Output {
        id,
        code,
        stdout: out.join().unwrap_or_default(),
        stderr,
        ms: t0.elapsed().as_secs_f64() * 1000.0,
        timed_out,
    }
}

fn value(o: Output) -> Value {
    obj(vec![
        ("id", Value::Number(o.id as f64)),
        ("code", Value::Number(o.code as f64)),
        ("stdout", s(&o.stdout)),
        ("stderr", s(&o.stderr)),
        ("ms", Value::Number(o.ms)),
        ("timedOut", Value::Bool(o.timed_out)),
    ])
}

fn field(o: Option<&Value>, key: &str) -> Option<Value> {
    match o {
        Some(Value::Object(m)) => m.borrow().strings.get(key).cloned(),
        _ => None,
    }
}

pub(super) fn run(args: &[Value]) -> Value {
    let cmd = super::str_arg(args, 0);
    let opts = args.get(1);
    let cwd = match field(opts, "cwd") {
        Some(Value::String(c)) => c.to_string(),
        _ => String::new(),
    };
    let values: Vec<String> = match field(opts, "args") {
        Some(Value::Array(a)) => a.borrow().iter().map(|v| v.to_display_string()).collect(),
        _ => Vec::new(),
    };
    let ms = field(opts, "timeoutMs")
        .map(|v| num_arg(&[v], 0, 60_000.0))
        .unwrap_or(60_000.0)
        .max(1.0);
    let id = hold(args.get(2));
    std::thread::spawn(move || {
        deliver(
            id,
            run_blocking(id, &cmd, &cwd, &values, Duration::from_millis(ms as u64)),
            value,
        )
    });
    Value::Number(id as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn captures_output_status_and_quoting() {
        let o = run_blocking(
            1,
            "printf '%s|' \"$PWD\" 'a b'; echo oops >&2; exit 3",
            "/tmp",
            &[],
            MINUTE,
        );
        assert_eq!(o.code, 3);
        assert!(o.stdout.ends_with("|a b|"), "{}", o.stdout);
        assert_eq!(o.stderr, "oops\n");
        // Values arrive as arguments; shell syntax in them never runs.
        let evil = vec!["it's; $(touch /tmp/tish-pwned) `id`".to_string()];
        assert_eq!(
            run_blocking(1, "printf %s \"$1\"", "", &evil, MINUTE).stdout,
            evil[0]
        );
        assert_eq!(
            run_blocking(1, "printf %s '\"$1\"'", "", &evil, MINUTE).stdout,
            "\"$1\""
        );
    }

    #[test]
    fn timeout_kills_the_whole_pipeline() {
        let t0 = Instant::now();
        let o = run_blocking(
            1,
            "sleep 30 | cat; echo done",
            "",
            &[],
            Duration::from_secs(1),
        );
        assert!(o.timed_out);
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "took {:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn caps_large_output() {
        let o = run_blocking(1, "yes 0123456789 | head -c 3000000", "", &[], MINUTE);
        assert_eq!(o.code, 0);
        assert_eq!(o.stdout.len(), MAX_OUTPUT);
    }
}
