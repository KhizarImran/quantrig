use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_setfl},
    io::Errno,
    process::{Pid, Signal, WaitOptions, kill_process_group, set_child_subreaper, waitpgid},
};
use std::{
    fs::File,
    io::{self, Read},
    os::{fd::OwnedFd, unix::process::CommandExt},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

pub(super) fn run(
    mut command: Command,
    wall_time: Duration,
    output_limit: usize,
) -> io::Result<Output> {
    let deadline = Instant::now()
        .checked_add(wall_time)
        .ok_or_else(|| io::Error::other("backtest deadline out of range"))?;
    // Adopt descendants if bubblewrap and its namespace reaper die together.
    // Cleanup waits only for children in this run's process group.
    set_child_subreaper(Pid::from_raw(1))?;
    let mut child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let group = Pid::from_child(&child);
    let result = capture(&mut child, deadline, wall_time, output_limit);
    if result.is_err() {
        signal_group(group)?;
        child.wait()?;
    }
    // Bubblewrap's monitor can return the runner's status before its reaper exits.
    reap_group(group)?;
    result
}

fn capture(
    child: &mut Child,
    deadline: Instant,
    wall_time: Duration,
    output_limit: usize,
) -> io::Result<Output> {
    let stdout = child.stdout.take().map(OwnedFd::from);
    let stderr = child.stderr.take().map(OwnedFd::from);
    let mut pipes = [Pipe::new(stdout)?, Pipe::new(stderr)?];
    let mut remaining = output_limit;
    loop {
        let delay = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("backtest timed out after {} seconds", wall_time.as_secs()),
                )
            })?;
        for pipe in &mut pipes {
            if !pipe.read(&mut remaining)? {
                return Err(io::Error::other(format!(
                    "backtest exceeded the {output_limit}-byte stdout and stderr limit"
                )));
            }
        }
        if pipes.iter().all(|pipe| pipe.reader.is_none())
            && let Some(status) = child.try_wait()?
        {
            let [stdout, stderr] = pipes.map(|pipe| pipe.bytes);
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }

        let mut ready: Vec<_> = pipes
            .iter()
            .filter_map(|pipe| pipe.reader.as_ref())
            .map(|reader| PollFd::new(reader, PollFlags::IN))
            .collect();
        // A process can close both streams and continue running.
        let delay = if ready.is_empty() {
            delay.min(Duration::from_millis(20))
        } else {
            delay
        };
        let timeout = Timespec::try_from(delay).map_err(io::Error::other)?;
        if let Err(error) = poll(&mut ready, Some(&timeout))
            && error != Errno::INTR
        {
            return Err(error.into());
        }
    }
}

fn reap_group(group: Pid) -> io::Result<()> {
    loop {
        match waitpgid(group, WaitOptions::empty()) {
            Ok(Some(_)) | Err(Errno::INTR) => continue,
            Ok(None) | Err(Errno::CHILD) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
}

fn signal_group(group: Pid) -> io::Result<()> {
    match kill_process_group(group, Signal::KILL) {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

struct Pipe {
    reader: Option<File>,
    bytes: Vec<u8>,
}

impl Pipe {
    fn new(fd: Option<OwnedFd>) -> io::Result<Self> {
        let fd = fd.ok_or_else(|| io::Error::other("missing sandbox output pipe"))?;
        fcntl_setfl(&fd, OFlags::NONBLOCK)?;
        Ok(Self {
            reader: Some(File::from(fd)),
            bytes: Vec::new(),
        })
    }

    fn read(&mut self, remaining: &mut usize) -> io::Result<bool> {
        let Some(reader) = &mut self.reader else {
            return Ok(true);
        };
        let mut buffer = [0; 8192];
        match reader.read(&mut buffer) {
            Ok(0) => self.reader = None,
            Ok(n) if n > *remaining => return Ok(false),
            Ok(n) => {
                self.bytes.extend_from_slice(&buffer[..n]);
                *remaining -= n;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::process::test_kill_process;
    use std::{fs, time::SystemTime};

    #[test]
    fn deadline_kills_and_reaps_the_process_group() {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let marker = std::env::temp_dir().join(format!("quantrig-group-{stamp}"));
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "sleep 30 & printf '%s %s' \"$$\" \"$!\" > \"$1\"; wait",
            "sh",
        ]);
        command.arg(&marker);

        let error = run(command, Duration::from_secs(1), 128).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let pids = fs::read_to_string(&marker).unwrap();
        fs::remove_file(marker).unwrap();
        let pids: Vec<_> = pids
            .split_whitespace()
            .map(|value| Pid::from_raw(value.parse().unwrap()).unwrap())
            .collect();
        assert_eq!(pids.len(), 2);
        for pid in pids {
            assert_eq!(test_kill_process(pid), Err(Errno::SRCH));
        }
    }

    #[test]
    fn stdout_and_stderr_share_one_output_limit() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 123456; printf 7890 >&2"]);
        let error = run(command, Duration::from_secs(2), 9).unwrap_err();
        assert!(error.to_string().contains("stdout and stderr limit"));

        let mut command = Command::new("sh");
        command.args(["-c", "printf 123456; printf 7890 >&2"]);
        let output = run(command, Duration::from_secs(2), 10).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"123456");
        assert_eq!(output.stderr, b"7890");
    }
}
