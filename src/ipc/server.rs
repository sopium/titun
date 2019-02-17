// Copyright 2018 Guanhao Yin <sopium@mysterious.site>

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

// XXX: named pipe security???

use crate::ipc::commands::*;
use crate::ipc::parse::*;
use crate::wireguard::re_exports::U8Array;
use crate::wireguard::{SetPeerCommand, WgState, WgStateOut};
use failure::{Error, ResultExt};
use futures::channel::mpsc::Sender;
use futures::prelude::SinkExt;
use hex::encode;
use std::io::{BufWriter, Read, Write};
use std::marker::Unpin;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::thread::Builder;
use std::time::SystemTime;

#[cfg(windows)]
pub fn start_ipc_server(
    wg: Weak<WgState>,
    dev_name: &str,
    sender: Sender<Box<FnMut() + Send + 'static>>,
) -> Result<(), Error> {
    use crate::ipc::windows_named_pipe::*;

    let mut path = Path::new(r#"\\.\pipe\wireguard"#).join(dev_name);
    path.set_extension("sock");
    let mut listener = PipeListener::bind(path).context("Bind IPC socket")?;
    Builder::new()
        .name("ipc-server".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                // We only serve one connection at a time.
                serve(&wg, &stream.unwrap(), sender.clone()).unwrap_or_else(|e| {
                    warn!("Error serving IPC connection: {:?}", e);
                });
            }
        })?;
    Ok(())
}

#[cfg(not(windows))]
pub fn start_ipc_server(
    wg: Weak<WgState>,
    dev_name: &str,
    sender: Sender<Box<FnMut() + Send + 'static>>,
) -> Result<(), Error> {
    use nix::sys::stat::{umask, Mode};
    use std::fs::{create_dir_all, remove_file};
    use std::os::unix::net::UnixListener;

    umask(Mode::from_bits(0o077).unwrap());
    let dir = Path::new(r#"/run/wireguard"#);
    create_dir_all(&dir).context("Create directory /run/wireguard")?;
    let mut path = dir.join(dev_name);
    path.set_extension("sock");
    let _ = remove_file(path.as_path());
    let listener = UnixListener::bind(path.as_path()).context("Bind IPC socket.")?;
    Builder::new()
        .name("ipc-server".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                // We only serve one connection at a time.
                serve(&wg, &stream.unwrap(), sender.clone()).unwrap_or_else(|e| {
                    warn!("Error serving IPC connection: {:?}", e);
                });
            }
        })?;
    Ok(())
}

fn write_wg_state(w: impl Write, state: &WgStateOut) -> Result<(), ::std::io::Error> {
    let mut w = BufWriter::with_capacity(4096, w);
    writeln!(w, "private_key={}", encode(state.private_key.as_slice()))?;
    writeln!(w, "listen_port={}", state.listen_port)?;
    if state.fwmark != 0 {
        writeln!(w, "fwmark={}", state.fwmark)?;
    }
    for p in &state.peers {
        writeln!(w, "public_key={}", encode(p.public_key.as_slice()))?;
        if let Some(ref psk) = p.preshared_key {
            writeln!(w, "preshared_key={}", encode(psk))?;
        }
        for a in &p.allowed_ips {
            writeln!(w, "allowed_ip={}/{}", a.0, a.1)?;
        }
        writeln!(
            w,
            "persistent_keepalive_interval={}",
            p.persistent_keepalive_interval.unwrap_or(0)
        )?;
        if let Some(ref e) = p.endpoint {
            writeln!(w, "endpoint={}", e)?;
        }
        writeln!(w, "rx_bytes={}", p.rx_bytes)?;
        writeln!(w, "tx_bytes={}", p.tx_bytes)?;
        if let Some(ref t) = p.last_handshake_time {
            let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap();
            let secs = d.as_secs();
            let nanos = d.subsec_nanos();
            writeln!(w, "last_handshake_time_sec={}", secs)?;
            writeln!(w, "last_handshake_time_nsec={}", nanos)?;
        }
    }
    writeln!(w, "errno=0")?;
    writeln!(w)?;
    w.flush()
}

fn write_error(stream: impl Write, errno: i32) -> Result<(), ::std::io::Error> {
    let mut writer = BufWriter::with_capacity(128, stream);
    writeln!(writer, "errno={}", errno)?;
    writeln!(writer)?;
    writer.flush()
}

fn process_wg_set(wg: &Arc<WgState>, command: WgSetCommand) {
    if let Some(key) = command.private_key {
        wg.set_key(key);
    }
    if let Some(p) = command.listen_port {
        wg.set_port(p).unwrap_or_else(|e| {
            warn!("Failed to set port: {}", e);
        });
    }
    if let Some(fwmark) = command.fwmark {
        wg.set_fwmark(fwmark).unwrap_or_else(|e| {
            warn!("Failed to set fwmark: {}", e);
        });
    }
    if command.replace_peers {
        wg.remove_all_peers();
    }
    for p in command.peers {
        if p.remove {
            wg.remove_peer(&p.public_key);
            continue;
        }
        if !wg.peer_exists(&p.public_key) {
            wg.add_peer(&p.public_key).unwrap();
        }
        wg.set_peer(SetPeerCommand {
            public_key: p.public_key,
            preshared_key: p.preshared_key,
            endpoint: p.endpoint,
            allowed_ips: p.allowed_ips,
            persistent_keepalive_interval: p.persistent_keepalive_interval,
            replace_allowed_ips: p.replace_allowed_ips,
        })
        .unwrap();
    }
}

pub fn serve<S>(
    wg: &Weak<WgState>,
    stream: S,
    mut sender: Sender<Box<FnMut() + Send + 'static>>,
) -> Result<(), Error>
where
    S: Read + Write + Clone + Unpin,
{
    let c = match parse_command_sync(stream.clone().take(1024 * 1024)) {
        Ok(Some(c)) => c,
        Ok(None) => return Ok(()),
        Err(e) => {
            drop(write_error(stream.clone(), /* EINVAL */ 22));
            return Err(e);
        }
    };
    let wg = match wg.upgrade() {
        None => {
            write_error(stream.clone(), /* ENXIO */ 6)?;
            bail!("WgState no longer available");
        }
        Some(wg) => wg,
    };
    match c {
        WgIpcCommand::Get => {
            write_wg_state(stream.clone(), &wg.get_state())?;
        }
        WgIpcCommand::Set(sc) => {
            // FnMut hack.
            let mut sc = Some(sc);
            futures::executor::block_on(sender.send(Box::new(move || {
                process_wg_set(&wg, sc.take().unwrap());
            })))
            .unwrap();
            write_error(stream.clone(), 0)?;
        }
    }
    Ok(())
}
