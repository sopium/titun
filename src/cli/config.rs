// Copyright 2019 Yin Guanhao <sopium@mysterious.site>

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

use crate::wireguard::{X25519Key, X25519Pubkey};
use anyhow::Context;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};

/// Read and parse configuration from the file at the specified path.
///
/// `print_warnings`: Print warnings to stderr directly instead of go through
/// the logger.
pub fn load_config_from_path(p: &Path, print_warnings: bool) -> anyhow::Result<Config<SocketAddr>> {
    let file = OpenOptions::new()
        .read(true)
        .open(p)
        .context("failed to open config file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match file.metadata() {
            Err(_) => (),
            Ok(m) => {
                if m.mode() & 0o004 != 0 {
                    if print_warnings {
                        eprintln!(
                            "[WARN  titun::cli::config] configuration file is world readable"
                        );
                    } else {
                        warn!("configuration file is world readable");
                    }
                }
            }
        }
    }
    let config = load_config_from_file(&file, print_warnings)?;
    #[cfg(unix)]
    let mut config = config;
    #[cfg(unix)]
    {
        config.general.config_file_path = Some(p.into());
    }
    Ok(config)
}

/// Read and parse configuration from file.
///
/// `print_warnings`: Print warnings to stderr directly instead of go through
/// the logger.
fn load_config_from_file(
    mut file: &File,
    print_warnings: bool,
) -> anyhow::Result<Config<SocketAddr>> {
    let mut file_content = String::new();
    file.read_to_string(&mut file_content)
        .context("failed to read config file")?;
    file_content = super::transform::maybe_transform(file_content);
    let config: Config<String> =
        toml::from_str(&file_content).context("failed to parse config file")?;

    // Verify that there are no duplicated peers. And warn about duplicated routes.
    let mut previous_peers = HashSet::new();
    let mut previous_routes = HashSet::new();

    for p in &config.peers {
        if !previous_peers.insert(p.public_key) {
            bail!(
                "invalid config file: peer {} appeared multiple times",
                base64::encode(&p.public_key)
            );
        }
        for &route in &p.allowed_ips {
            if !previous_routes.insert(route) {
                if print_warnings {
                    eprintln!(
                        "[WARN  titun::cli::config] allowed IP {}/{} appeared multiple times",
                        route.0, route.1
                    );
                } else {
                    warn!("allowed IP {}/{} appeared multiple time", route.0, route.1);
                }
            }
        }
    }

    // Verify that `network.prefix_len` is valid.
    #[cfg(windows)]
    {
        if let Some(ref n) = config.network {
            if n.prefix_len > 32 {
                bail!(
                    "invalid config file: prefix length {} is too large, should be <= 32",
                    n.prefix_len,
                );
            }
        }
    }

    Ok(config.resolve_addresses(print_warnings)?)
}

// Endpoint is the type of peer endpoints. It is expected to be either `String`
// or `SocketAddr`. First we parse config using `String`, then we parse and/or
// resolve the endpoints, and turn it into `SocketAddr`.
#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Config<Endpoint> {
    #[serde(default)]
    pub general: GeneralConfig,

    pub interface: InterfaceConfig,

    #[cfg(windows)]
    pub network: Option<NetworkConfig>,

    #[serde(default, rename = "Peer")]
    pub peers: Vec<PeerConfig<Endpoint>>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct GeneralConfig {
    pub log: Option<String>,

    // Change to this user.
    #[cfg(unix)]
    pub user: Option<String>,

    // Change to this group.
    #[cfg(unix)]
    pub group: Option<String>,

    // Only command line option.
    #[serde(skip)]
    pub exit_stdin_eof: bool,

    #[serde(skip)]
    pub config_file_path: Option<PathBuf>,

    #[serde(default)]
    pub foreground: bool,

    pub threads: Option<usize>,
}

impl Eq for GeneralConfig {}

impl PartialEq<GeneralConfig> for GeneralConfig {
    /// config_file is ignored in comparison.
    fn eq(&self, other: &GeneralConfig) -> bool {
        #[cfg(unix)]
        let ug = self.user == other.user && self.group == other.group;
        #[cfg(not(unix))]
        let ug = true;
        self.log == other.log
            && ug
            && self.exit_stdin_eof == other.exit_stdin_eof
            && self.foreground == other.foreground
            && self.threads == other.threads
    }
}

#[cfg(windows)]
#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct NetworkConfig {
    pub address: std::net::Ipv4Addr,
    pub prefix_len: u32,
}

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct InterfaceConfig {
    #[serde(default, with = "os_string_actually_string")]
    pub name: Option<OsString>,

    #[serde(alias = "Key", with = "base64_u8_array")]
    pub private_key: X25519Key,

    #[serde(alias = "Port")]
    pub listen_port: Option<u16>,

    #[serde(rename = "FwMark", alias = "Mark")]
    pub fwmark: Option<u32>,
}

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct PeerConfig<Endpoint> {
    /// Peer public key.
    #[serde(with = "base64_u8_array")]
    pub public_key: X25519Pubkey,

    /// Pre-shared key.
    #[serde(alias = "PSK", default, with = "base64_u8_array_optional")]
    pub preshared_key: Option<[u8; 32]>,

    /// Peer endpoint.
    pub endpoint: Option<Endpoint>,

    /// Allowed source IPs.
    #[serde(
        rename = "AllowedIPs",
        alias = "AllowedIP",
        alias = "AllowedIp",
        alias = "AllowedIps",
        alias = "Route",
        alias = "Routes",
        default,
        with = "ip_prefix_len"
    )]
    pub allowed_ips: BTreeSet<(IpAddr, u32)>,

    /// Persistent keep-alive interval.
    /// Valid values: 1 - 0xfffe.
    #[serde(alias = "PersistentKeepalive")]
    pub keepalive: Option<NonZeroU16>,
}

impl Config<String> {
    fn resolve_addresses(self, print_warnings: bool) -> anyhow::Result<Config<SocketAddr>> {
        let mut peers = Vec::with_capacity(self.peers.len());
        for p in self.peers {
            let endpoint = if let Some(endpoint) = p.endpoint {
                use std::net::ToSocketAddrs;
                match endpoint.to_socket_addrs() {
                    Ok(mut addrs) => Some(addrs.next().unwrap()),
                    Err(e) => {
                        // Reject invalid syntax, but warn and ignore resolution failures.
                        if e.kind() == std::io::ErrorKind::InvalidInput {
                            bail!("Invalid endpoint: {}", endpoint);
                        }
                        if print_warnings {
                            eprintln!(
                                "[WARN  titun::cli::config] failed to resolve endpoint {}: {}",
                                endpoint, e
                            );
                        } else {
                            warn!("failed to resolve {}: {}", endpoint, e);
                        }
                        None
                    }
                }
            } else {
                None
            };
            peers.push(PeerConfig {
                public_key: p.public_key,
                preshared_key: p.preshared_key,
                endpoint,
                allowed_ips: p.allowed_ips,
                keepalive: p.keepalive,
            });
        }
        Ok(Config {
            general: self.general,
            #[cfg(windows)]
            network: self.network,
            interface: self.interface,
            peers,
        })
    }
}

mod os_string_actually_string {
    use super::*;

    pub fn serialize<S: Serializer>(v: &Option<OsString>, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;

        if let Some(ref v) = v {
            s.serialize_some(v.to_str().ok_or_else(|| Error::custom("not utf-8"))?)
        } else {
            s.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<OsString>, D::Error> {
        let v: String = Deserialize::deserialize(d)?;
        Ok(Some(v.into()))
    }
}

mod base64_u8_array {
    use super::*;
    use noise_protocol::U8Array;

    pub fn serialize<T: U8Array, S: Serializer>(t: &T, s: S) -> Result<S::Ok, S::Error> {
        let mut result = [0u8; 64];
        let len = base64::encode_config_slice(t.as_slice(), base64::STANDARD, &mut result[..]);
        s.serialize_str(std::str::from_utf8(&result[..len]).unwrap())
    }

    pub fn deserialize<'de, T: U8Array, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
        use serde::de::Error;

        let string: Cow<'_, str> = Deserialize::deserialize(d)?;
        let vec =
            base64::decode(string.as_ref()).map_err(|_| Error::custom("base64 decode failed"))?;

        if vec.len() != T::len() {
            return Err(Error::custom("invalid length"));
        }

        Ok(T::from_slice(vec.as_slice()))
    }
}

mod ip_prefix_len {
    use super::*;

    pub fn serialize<S: Serializer>(t: &BTreeSet<(IpAddr, u32)>, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;

        let mut seq = s.serialize_seq(t.len().into())?;

        struct IpAndPrefixLen {
            ip: IpAddr,
            prefix_len: u32,
        }

        impl Serialize for IpAndPrefixLen {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let max_prefix_len = if self.ip.is_ipv4() { 32 } else { 128 };

                if self.prefix_len == max_prefix_len {
                    s.collect_str(&self.ip)
                } else {
                    s.collect_str(&format_args!("{}/{}", self.ip, self.prefix_len))
                }
            }
        }

        for &(ip, prefix_len) in t {
            seq.serialize_element(&IpAndPrefixLen { ip, prefix_len })?;
        }

        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeSet<(IpAddr, u32)>, D::Error> {
        use serde::de::{Error, SeqAccess, Visitor};
        use std::fmt;

        struct AllowedIPsVisitor;

        impl AllowedIPsVisitor {
            fn parse<E: Error>(v: &str) -> Result<(IpAddr, u32), E> {
                let mut parts = v.splitn(2, '/');
                let ip: IpAddr = parts
                    .next()
                    .unwrap()
                    .parse()
                    .map_err(|_| Error::custom("failed to parse allowed IPs"))?;
                let max_prefix_len = if ip.is_ipv4() { 32 } else { 128 };
                let prefix_len: u32 = parts
                    .next()
                    .map(|x| x.parse())
                    .unwrap_or(Ok(max_prefix_len))
                    .map_err(|_| Error::custom("failed to parse allowed IPs"))?;
                if prefix_len > max_prefix_len {
                    return Err(Error::custom("prefix length is too large"));
                }
                Ok((ip, prefix_len))
            }
        }

        impl<'de> Visitor<'de> for AllowedIPsVisitor {
            type Value = BTreeSet<(IpAddr, u32)>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "an allowed IP (IP/PREFIX_LEN) or an array of allowed IPs"
                )
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let v = Self::parse(v)?;
                let mut r = BTreeSet::new();
                r.insert(v);
                Ok(r)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, <A as SeqAccess<'de>>::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut result = BTreeSet::new();
                while let Some(v) = seq.next_element()? {
                    let v: Cow<'_, str> = v;
                    let p = Self::parse(&v)?;
                    result.insert(p);
                }
                Ok(result)
            }
        }

        d.deserialize_any(AllowedIPsVisitor)
    }
}

mod base64_u8_array_optional {
    use super::*;
    use noise_protocol::U8Array;

    pub fn serialize<T: U8Array, S: Serializer>(t: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        if let Some(x) = t.as_ref() {
            let mut result = [0u8; 64];
            let len = base64::encode_config_slice(x.as_slice(), base64::STANDARD, &mut result[..]);
            s.serialize_some(std::str::from_utf8(&result[..len]).unwrap())
        } else {
            s.serialize_none()
        }
    }

    pub fn deserialize<'de, T: U8Array, D: Deserializer<'de>>(d: D) -> Result<Option<T>, D::Error> {
        super::base64_u8_array::deserialize(d).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noise_protocol::U8Array;

    const EXAMPLE_CONFIG: &str = r##"[Interface]
Name = "tun7"
ListenPort = 7777
PrivateKey = "2BJtcgPUjHfKKN3yMvTiVQbJ/UgHj2tcZE6xU/4BdGM="
FwMark = 33

[Network]
Address = "192.168.77.0"
PrefixLen = 24

[[Peer]]
PublicKey = "Ck8P+fUguLIf17zmb3eWxxS7PqgN3+ciMFBlSwqRaw4="
PresharedKey = "w64eiHxoUHU8DcFexHWzqILOvbWx9U+dxxh8iQqJr+k="
AllowedIPs = "192.168.77.1"
Endpoint = "192.168.3.1:7777"
PersistentKeepalive = 17
"##;

    #[test]
    fn deserialization() {
        let config: Config<String> = toml::from_str(EXAMPLE_CONFIG).unwrap();
        let config = config.resolve_addresses(true).unwrap();
        assert_eq!(
            config,
            Config {
                general: GeneralConfig::default(),
                interface: InterfaceConfig {
                    name: Some("tun7".into()),
                    listen_port: Some(7777),
                    private_key: U8Array::from_slice(
                        &base64::decode("2BJtcgPUjHfKKN3yMvTiVQbJ/UgHj2tcZE6xU/4BdGM=").unwrap()
                    ),
                    fwmark: Some(33),
                },
                #[cfg(windows)]
                network: Some(NetworkConfig {
                    address: "192.168.77.0".parse().unwrap(),
                    prefix_len: 24,
                }),
                peers: vec![PeerConfig {
                    public_key: U8Array::from_slice(
                        &base64::decode("Ck8P+fUguLIf17zmb3eWxxS7PqgN3+ciMFBlSwqRaw4=").unwrap()
                    ),
                    preshared_key: Some(U8Array::from_slice(
                        &base64::decode("w64eiHxoUHU8DcFexHWzqILOvbWx9U+dxxh8iQqJr+k=").unwrap()
                    )),
                    endpoint: Some("192.168.3.1:7777".parse().unwrap()),
                    allowed_ips: [("192.168.77.1".parse().unwrap(), 32)]
                        .iter()
                        .cloned()
                        .collect(),
                    keepalive: NonZeroU16::new(17),
                }],
            }
        );
    }
}
