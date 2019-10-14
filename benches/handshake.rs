use criterion::{criterion_group, criterion_main, Criterion};
use rand::{rngs::OsRng, RngCore};
use std::collections::BTreeSet;
use titun::wireguard::handshake::*;
use titun::wireguard::re_exports::*;
use titun::wireguard::types::Id;
use titun::wireguard::*;

criterion_group!(benches, register_benches,);
criterion_main!(benches);

fn register_benches(c: &mut Criterion) {
    c.bench_function("handshake init", |b| {
        let k = X25519::genkey();
        let init = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let k = X25519::genkey();
        let resp = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let init_peer = PeerInfo {
            public_key: *resp.pubkey(),
            psk: None,
            endpoint: None,
            allowed_ips: BTreeSet::new(),
            keepalive: None,
            roaming: true,
        };

        b.iter(|| {
            let si = Id::gen();
            initiate(&init, &init_peer, si)
        });
    });

    c.bench_function("handshake respond", |b| {
        let k = X25519::genkey();
        let init = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let k = X25519::genkey();
        let resp = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let init_peer = PeerInfo {
            public_key: *resp.pubkey(),
            psk: None,
            endpoint: None,
            allowed_ips: BTreeSet::new(),
            keepalive: None,
            roaming: true,
        };

        let si = Id::gen();
        let (m0, _) = initiate(&init, &init_peer, si).unwrap();

        b.iter(|| {
            let mut result0 = process_initiation(&resp, &m0).unwrap();
            result0.handshake_state.push_psk(&[0u8; 32]);
            let ri = Id::gen();
            responde(&resp, &mut result0, ri)
        });
    });

    c.bench_function("handshake process response", |b| {
        let k = X25519::genkey();
        let init = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let k = X25519::genkey();
        let resp = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let init_peer = PeerInfo {
            public_key: *resp.pubkey(),
            psk: None,
            endpoint: None,
            allowed_ips: BTreeSet::new(),
            keepalive: None,
            roaming: true,
        };

        let si = Id::gen();
        let (m0, ihs) = initiate(&init, &init_peer, si).unwrap();
        assert!(verify_mac1(&resp, &m0));
        let mut result0 = process_initiation(&resp, &m0).unwrap();
        result0.handshake_state.push_psk(&[0u8; 32]);
        let ri = Id::gen();
        let m1 = responde(&resp, &mut result0, ri).unwrap();
        assert!(verify_mac1(&init, &m1));

        b.iter(|| {
            let mut hs = ihs.clone();
            process_response(&mut hs, &m1).unwrap();
        });
    });

    c.bench_function("verify mac1", |b| {
        let k = X25519::genkey();
        let init = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let k = X25519::genkey();
        let resp = WgInfo {
            key: k,
            fwmark: 0,
            port: 0,
        };

        let init_peer = PeerInfo {
            public_key: *resp.pubkey(),
            psk: None,
            endpoint: None,
            allowed_ips: BTreeSet::new(),
            keepalive: None,
            roaming: true,
        };

        let si = Id::gen();
        let (m0, _) = initiate(&init, &init_peer, si).unwrap();
        b.iter(|| verify_mac1(&resp, &m0));
    });

    c.bench_function("cookie reply", |b| {
        let mut rng = OsRng;

        let mut pk = [0u8; 32];
        rng.fill_bytes(&mut pk);

        let mut mac1 = [0u8; 16];
        rng.fill_bytes(&mut mac1);

        let mut secret = [0u8; 32];
        rng.fill_bytes(&mut secret);

        b.iter(|| {
            use titun::wireguard::cookie::*;

            let cookie = calc_cookie(&secret, b"1.2.3.4");

            let reply = cookie_reply(&pk, &cookie, Id::gen(), &mac1);

            reply
        });
    });
}
