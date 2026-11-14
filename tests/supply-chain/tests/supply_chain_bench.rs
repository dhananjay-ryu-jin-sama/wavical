#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use wavical_ai::downloader::{DownloadKind, DownloadOptions, download_verified_file};
use wavical_cli::extractor::{ExtractionLimits, extract_zip_safely};
use wavical_ipc::UpdateManifest;
use wavical_shell::updater::{UpdateCheckOptions, check_and_stage_update};
use zip::write::SimpleFileOptions;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_b64(input: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() {
            input[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < input.len() {
            input[i + 2] as u32
        } else {
            0
        };
        let t = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((t >> 18) & 0x3F) as usize] as char);
        out.push(B64[((t >> 12) & 0x3F) as usize] as char);
        out.push(if i + 1 < input.len() {
            B64[((t >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if i + 2 < input.len() {
            B64[(t & 0x3F) as usize] as char
        } else {
            '='
        });
        i += 3;
    }
    out
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("wavical_supply_chain_{name}_{nanos}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn gen_keypair() -> (SigningKey, String) {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).expect("rng");
    let sk = SigningKey::from_bytes(&secret);
    let pk_b64 = encode_b64(&sk.verifying_key().to_bytes());
    (sk, pk_b64)
}

fn sign_bytes(sk: &SigningKey, data: &[u8]) -> Vec<u8> {
    let sig = sk.sign(data);
    format!("{}\n", encode_b64(&sig.to_bytes())).into_bytes()
}

#[derive(Clone)]
enum MaliciousReply {
    Ok(Vec<u8>),
    Redirect(String),
    Truncated(Vec<u8>),
}

struct MaliciousMockServer {
    base_url: String,
    routes: Arc<Mutex<HashMap<String, MaliciousReply>>>,
    stop: Arc<AtomicBool>,
}

impl MaliciousMockServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind malicious mock server");
        let addr = listener.local_addr().expect("local addr");
        listener.set_nonblocking(true).ok();
        let routes: Arc<Mutex<HashMap<String, MaliciousReply>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let routes_bg = routes.clone();
        let stop_bg = stop.clone();

        thread::spawn(move || {
            while !stop_bg.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).ok();
                        let mut req_buf = [0u8; 2048];
                        let n = stream.read(&mut req_buf).unwrap_or(0);
                        let req_str = String::from_utf8_lossy(&req_buf[..n]);
                        let path = req_str
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("/")
                            .to_string();

                        let reply = routes_bg.lock().unwrap().get(&path).cloned();
                        match reply {
                            Some(MaliciousReply::Ok(body)) => {
                                let hdr = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = stream.write_all(hdr.as_bytes());
                                let _ = stream.write_all(&body);
                            }
                            Some(MaliciousReply::Redirect(loc)) => {
                                let hdr = format!(
                                    "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                );
                                let _ = stream.write_all(hdr.as_bytes());
                            }
                            Some(MaliciousReply::Truncated(body)) => {
                                let hdr = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    body.len() + 500
                                );
                                let _ = stream.write_all(hdr.as_bytes());
                                let _ = stream.write_all(&body);
                            }
                            None => {
                                let _ = stream.write_all(
                                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                );
                            }
                        }
                        let _ = stream.flush();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{}", addr.port()),
            routes,
            stop,
        }
    }

    fn set_route(&self, path: &str, reply: MaliciousReply) {
        self.routes.lock().unwrap().insert(path.to_string(), reply);
    }
}

impl Drop for MaliciousMockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, content) in entries {
            zw.start_file(*name, opts).expect("start zip entry");
            zw.write_all(content).expect("write zip entry");
        }
        zw.finish().expect("finish zip");
    }
    buf.into_inner()
}

fn make_symlink_zip(name: &str, target: &str) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        zw.add_symlink(name, target, SimpleFileOptions::default())
            .expect("add_symlink");
        zw.finish().expect("finish zip");
    }
    buf.into_inner()
}

fn assert_installed_state_unchanged(updater_dir: &Path, runtime_dir: &Path) {
    assert_eq!(
        fs::read_to_string(updater_dir.join("current"))
            .unwrap()
            .trim(),
        "1.0.0",
        "installed current pointer must remain 1.0.0"
    );
    assert_eq!(
        fs::read(
            updater_dir
                .join("versions")
                .join("1.0.0")
                .join("index.html")
        )
        .unwrap(),
        b"<html>1.0.0</html>"
    );
    assert!(!updater_dir.join("pending_update.json").exists());
    assert!(!updater_dir.join("versions").join("1.2.0").exists());
    assert!(!updater_dir.join("versions").join("1.2.0.staged").exists());
    assert!(!runtime_dir.join("bad-runtime.zip").exists());
}

#[test]
fn test_supply_chain_bench_all_wp2_and_wp7_bad_cases_rejected() {
    let srv = MaliciousMockServer::start();
    let root = unique_temp_dir("bench");
    let updater_dir = root.join("updater");
    let runtime_dir = root.join("runtimes");
    fs::create_dir_all(updater_dir.join("versions").join("1.0.0")).unwrap();
    fs::create_dir_all(&runtime_dir).unwrap();
    fs::write(
        updater_dir
            .join("versions")
            .join("1.0.0")
            .join("index.html"),
        b"<html>1.0.0</html>",
    )
    .unwrap();
    fs::write(updater_dir.join("current"), "1.0.0\n").unwrap();

    let dl_opts = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    // === WP2 BAD CASES: Runtime downloads & archives ===
    let valid_runtime_bytes = b"good-runtime-archive-bytes";
    let valid_runtime_sha = sha256_hex(valid_runtime_bytes);

    // 1. Runtime wrong size
    srv.set_route("/rt-wrong-size.zip", MaliciousReply::Ok(b"short".to_vec()));
    assert!(
        download_verified_file(
            &format!("{}/rt-wrong-size.zip", srv.base_url),
            &runtime_dir.join("bad-runtime.zip"),
            valid_runtime_bytes.len() as u64,
            &valid_runtime_sha,
            DownloadKind::Runtime,
            &dl_opts,
            None,
        )
        .is_err()
    );
    assert_installed_state_unchanged(&updater_dir, &runtime_dir);

    // 2. Runtime wrong SHA-256 hash
    let mut bad_hash_bytes = valid_runtime_bytes.to_vec();
    bad_hash_bytes[0] ^= 0xFF;
    srv.set_route("/rt-wrong-hash.zip", MaliciousReply::Ok(bad_hash_bytes));
    assert!(
        download_verified_file(
            &format!("{}/rt-wrong-hash.zip", srv.base_url),
            &runtime_dir.join("bad-runtime.zip"),
            valid_runtime_bytes.len() as u64,
            &valid_runtime_sha,
            DownloadKind::Runtime,
            &dl_opts,
            None,
        )
        .is_err()
    );
    assert_installed_state_unchanged(&updater_dir, &runtime_dir);

    // 3. Runtime off-list redirect
    srv.set_route(
        "/rt-redir.zip",
        MaliciousReply::Redirect("https://evil.example.invalid/rt.zip".to_string()),
    );
    assert!(
        download_verified_file(
            &format!("{}/rt-redir.zip", srv.base_url),
            &runtime_dir.join("bad-runtime.zip"),
            valid_runtime_bytes.len() as u64,
            &valid_runtime_sha,
            DownloadKind::Runtime,
            &dl_opts,
            None,
        )
        .is_err()
    );
    assert_installed_state_unchanged(&updater_dir, &runtime_dir);

    // 4. Malicious archives served over mock server & extracted via safe extractor
    let archive_cases: Vec<(&str, Vec<u8>)> = vec![
        ("zip_slip", make_zip(&[("../evil.txt", b"pwn")])),
        ("unc_path", make_zip(&[(r"\\attacker\share\x.txt", b"pwn")])),
        ("abs_path", make_zip(&[("/etc/passwd", b"pwn")])),
        ("drive_letter", make_zip(&[(r"C:\evil.txt", b"pwn")])),
        ("ads", make_zip(&[("foo.txt:stream", b"pwn")])),
        ("reserved_con", make_zip(&[("CON.txt", b"pwn")])),
        ("symlink", make_symlink_zip("link.txt", "/etc/passwd")),
        (
            "case_dup",
            make_zip(&[("node.exe", b"1"), ("NODE.EXE", b"2")]),
        ),
        ("corrupt", b"NOT-A-ZIP".to_vec()),
    ];

    for (name, bytes) in archive_cases {
        let route = format!("/bad-{name}.zip");
        let sha = sha256_hex(&bytes);
        srv.set_route(&route, MaliciousReply::Ok(bytes.clone()));
        let dl_path = root.join(format!("{name}.zip"));
        download_verified_file(
            &format!("{}{route}", srv.base_url),
            &dl_path,
            bytes.len() as u64,
            &sha,
            DownloadKind::Runtime,
            &dl_opts,
            None,
        )
        .expect("download test archive");

        let extract_dest = root.join(format!("extract_{name}"));
        let limits = ExtractionLimits::for_runtime();
        assert!(
            extract_zip_safely(&dl_path, &extract_dest, limits, false).is_err(),
            "malicious archive '{name}' must be rejected by safe extractor"
        );
        let _ = fs::remove_file(&dl_path);
        let _ = fs::remove_dir_all(&extract_dest);
        assert_installed_state_unchanged(&updater_dir, &runtime_dir);
    }

    // 5. Decompression bomb (exceeds tight limit)
    let bomb_zip = make_zip(&[("big.bin", &vec![b'A'; 4096])]);
    let bomb_path = root.join("bomb.zip");
    fs::write(&bomb_path, &bomb_zip).unwrap();
    let tight_limits = ExtractionLimits {
        max_bytes: 1024,
        max_files: 10,
    };
    assert!(extract_zip_safely(&bomb_path, &root.join("bomb_out"), tight_limits, false).is_err());
    let _ = fs::remove_file(&bomb_path);
    let _ = fs::remove_dir_all(root.join("bomb_out"));

    // === WP7 BAD CASES: Manifests, signatures, payloads, redirects, replays, expiry ===
    let (sk, pk) = gen_keypair();
    let (sk_unknown, _) = gen_keypair();
    let good_payload = make_zip(&[("index.html", b"<html>1.2.0</html>")]);
    let good_sha = sha256_hex(&good_payload);
    let now = now_secs();

    let make_manifest = |app_id: &str,
                         channel: &str,
                         version: &str,
                         expires: u64,
                         url: &str,
                         size: u64,
                         sha: &str| {
        serde_json::to_vec(&UpdateManifest {
            app_id: app_id.to_string(),
            channel: channel.to_string(),
            version: version.to_string(),
            minimum_supported_version: "1.0.0".to_string(),
            expires,
            rollout_percent: 100,
            payload_url: url.to_string(),
            payload_size: size,
            payload_sha256: sha.to_string(),
            release_notes: "Bad test release".to_string(),
        })
        .unwrap()
    };

    let check_opts = UpdateCheckOptions {
        app_id: "com.wavical.bench".to_string(),
        installed_version: "1.0.0".to_string(),
        channel: "stable".to_string(),
        manifest_url: format!("{}/manifest.json", srv.base_url),
        public_keys: vec![pk],
        download_options: dl_opts,
        now_unix: Some(now),
    };

    // A. Tampered manifest (1 byte flipped)
    {
        let mut m = make_manifest(
            "com.wavical.bench",
            "stable",
            "1.2.0",
            now + 86400,
            &format!("{}/p.zip", srv.base_url),
            good_payload.len() as u64,
            &good_sha,
        );
        let sig = sign_bytes(&sk, &m);
        m[5] ^= 0x01;
        srv.set_route("/manifest.json", MaliciousReply::Ok(m));
        srv.set_route("/manifest.json.sig", MaliciousReply::Ok(sig));
        assert!(check_and_stage_update(&updater_dir, &check_opts).is_err());
        assert_installed_state_unchanged(&updater_dir, &runtime_dir);
    }

    // B. Unknown signing key
    {
        let m = make_manifest(
            "com.wavical.bench",
            "stable",
            "1.2.0",
            now + 86400,
            &format!("{}/p.zip", srv.base_url),
            good_payload.len() as u64,
            &good_sha,
        );
        srv.set_route("/manifest.json", MaliciousReply::Ok(m.clone()));
        srv.set_route(
            "/manifest.json.sig",
            MaliciousReply::Ok(sign_bytes(&sk_unknown, &m)),
        );
        assert!(check_and_stage_update(&updater_dir, &check_opts).is_err());
        assert_installed_state_unchanged(&updater_dir, &runtime_dir);
    }

    // C. Wrong appId, channel mismatch, expired, >30d expiry
    for (app, chan, exp) in [
        ("com.evil.app", "stable", now + 86400),
        ("com.wavical.bench", "canary", now + 86400),
        ("com.wavical.bench", "stable", now - 5),
        ("com.wavical.bench", "stable", now + 31 * 86400),
    ] {
        let m = make_manifest(
            app,
            chan,
            "1.2.0",
            exp,
            &format!("{}/p.zip", srv.base_url),
            good_payload.len() as u64,
            &good_sha,
        );
        srv.set_route("/manifest.json", MaliciousReply::Ok(m.clone()));
        srv.set_route(
            "/manifest.json.sig",
            MaliciousReply::Ok(sign_bytes(&sk, &m)),
        );
        assert!(check_and_stage_update(&updater_dir, &check_opts).is_err());
        assert_installed_state_unchanged(&updater_dir, &runtime_dir);
    }

    // D. Replay older & same version (must NOT stage)
    for ver in ["0.9.0", "1.0.0"] {
        let m = make_manifest(
            "com.wavical.bench",
            "stable",
            ver,
            now + 86400,
            &format!("{}/p.zip", srv.base_url),
            good_payload.len() as u64,
            &good_sha,
        );
        srv.set_route("/manifest.json", MaliciousReply::Ok(m.clone()));
        srv.set_route(
            "/manifest.json.sig",
            MaliciousReply::Ok(sign_bytes(&sk, &m)),
        );
        let res = check_and_stage_update(&updater_dir, &check_opts).unwrap();
        assert!(!res.available);
        assert_installed_state_unchanged(&updater_dir, &runtime_dir);
    }

    // E. Tampered payload, size mismatch, off-list redirect, interrupted download
    let m_good = make_manifest(
        "com.wavical.bench",
        "stable",
        "1.2.0",
        now + 86400,
        &format!("{}/p.zip", srv.base_url),
        good_payload.len() as u64,
        &good_sha,
    );
    srv.set_route("/manifest.json", MaliciousReply::Ok(m_good.clone()));
    srv.set_route(
        "/manifest.json.sig",
        MaliciousReply::Ok(sign_bytes(&sk, &m_good)),
    );

    let mut bad_p = good_payload.clone();
    bad_p[0] ^= 0xFF;
    srv.set_route("/p.zip", MaliciousReply::Ok(bad_p));
    assert!(check_and_stage_update(&updater_dir, &check_opts).is_err());
    assert_installed_state_unchanged(&updater_dir, &runtime_dir);

    srv.set_route("/p.zip", MaliciousReply::Ok(b"short".to_vec()));
    assert!(check_and_stage_update(&updater_dir, &check_opts).is_err());
    assert_installed_state_unchanged(&updater_dir, &runtime_dir);

    srv.set_route(
        "/p.zip",
        MaliciousReply::Redirect("https://evil.example.invalid/p.zip".to_string()),
    );
    assert!(check_and_stage_update(&updater_dir, &check_opts).is_err());
    assert_installed_state_unchanged(&updater_dir, &runtime_dir);

    srv.set_route(
        "/p.zip",
        MaliciousReply::Truncated(good_payload[..4].to_vec()),
    );
    assert!(check_and_stage_update(&updater_dir, &check_opts).is_err());
    assert_installed_state_unchanged(&updater_dir, &runtime_dir);

    let _ = fs::remove_dir_all(&root);
}
