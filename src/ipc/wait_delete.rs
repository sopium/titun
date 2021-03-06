// Copyright 2019 Guanhao Yin <sopium@mysterious.site>

// This file is part of TiTun.

// TiTun is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// TiTun is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with TiTun.  If not, see <https://www.gnu.org/licenses/>.

#![cfg(unix)]

use std::path::Path;
use tokio::sync::oneshot::Sender;

// Polling on BSD.
//
// It is not possible to use kqueue to watch delete events on a socket:
// https://bugs.freebsd.org/bugzilla/show_bug.cgi?id=170177
#[cfg(not(target_os = "linux"))]
pub async fn wait_delete(path: &Path, ready: Sender<()>) -> anyhow::Result<()> {
    use nix::dir::Dir;
    use nix::fcntl::OFlag;
    use nix::sys::stat::Mode;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    // Use nix dir because it can be rewind, so it works with privilege dropping.
    let mut dir = Dir::open(
        path.parent().unwrap(),
        OFlag::O_DIRECTORY | OFlag::O_RDONLY,
        Mode::empty(),
    )?;
    let file_name = path.file_name().unwrap().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = ready.send(());
    std::thread::spawn(move || {
        loop {
            let mut found = false;
            for f in dir.iter() {
                let f = f.unwrap();
                let os_str = OsStr::from_bytes(f.file_name().to_bytes());
                let f_name = Path::new(os_str);
                if f_name == file_name {
                    found = true;
                    break;
                }
            }
            if !found {
                break;
            }

            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        tx.send(Ok(())).unwrap();
    });
    rx.await.unwrap()
}

#[cfg(target_os = "linux")]
pub async fn wait_delete(p: &Path, ready: Sender<()>) -> anyhow::Result<()> {
    // Use inotify on linux.
    use anyhow::Context;
    use futures::StreamExt;
    use inotify::{EventMask, Inotify, WatchMask};

    let file_name = p.file_name().unwrap().into();
    let parent_dir = p.parent().unwrap();
    let mut inotify = Inotify::init().context("init")?;
    inotify
        .add_watch(parent_dir, WatchMask::DELETE)
        .context("add_watch")?;
    let _ = ready.send(());
    let buf = vec![0u8; 1024];
    let mut stream = inotify.event_stream(buf).context("event_stream")?;
    loop {
        let event = stream.next().await.unwrap().context("next")?;
        if event.mask == EventMask::DELETE && event.name.as_ref() == Some(&file_name) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // XXX: On mips/mipsel, `INotify::init` (`inotify_init1`) returns an
    // `EINVAL` error *in CI*. Ignore the test for now.
    #[cfg(not(target_arch = "mips"))]
    #[tokio::test]
    async fn test_wait_delete() {
        use super::*;
        use nix::unistd::{mkstemp, unlink};

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let mut file = std::env::temp_dir();
        file.push("test_wait_delete_XXXXXX");
        let (_, tmp_path) = mkstemp(&file).expect("mkstemp");
        {
            let tmp_path = tmp_path.clone();
            tokio::spawn(async move {
                ready_rx.await.unwrap();
                unlink(&tmp_path).expect("unlink");
            });
        }
        wait_delete(&tmp_path, ready_tx).await.expect("wait delete");
        assert!(!tmp_path.exists());
    }
}
