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
use wavical_ai::downloader::DownloadOptions;
use wavical_config::UpdaterConfig;
use wavical_ipc::{Capabilities, HandlerRegistry, UpdateManifest};
use wavical_shell::updater::{
    UpdateCheckOptions, UpdaterError, apply_pending_update, check_and_stage_update,
    load_or_create_rollout_bucket, mark_app_ready, prepare_launch,
};
use wavical_shell::{WindowConfig, process_ipc_message};
use wry::http::Uri;
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
    let dir = std::env::temp_dir().join(format!("wavical_updater_test_{name}_{nanos}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn make_zip_payload(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, content) in files {
            zw.start_file(*name, opts).expect("start zip file");
            zw.write_all(content).expect("write zip file");
        }
        zw.finish().expect("finish zip");
    }
    buf.into_inner()
}

#[derive(Clone)]
enum MockReply {
    Ok(Vec<u8>),
    Redirect(String),
    Truncated(Vec<u8>),
}

struct MockServer {
    base_url: String,
    routes: Arc<Mutex<HashMap<String, MockReply>>>,
    stop: Arc<AtomicBool>,
}

impl MockServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        listener.set_nonblocking(true).ok();
        let routes: Arc<Mutex<HashMap<String, MockReply>>> = Arc::new(Mutex::new(HashMap::new()));
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
                            Some(MockReply::Ok(body)) => {
                                let hdr = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = stream.write_all(hdr.as_bytes());
                                let _ = stream.write_all(&body);
                            }
                            Some(MockReply::Redirect(loc)) => {
                                let hdr = format!(
                                    "HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                );
                                let _ = stream.write_all(hdr.as_bytes());
                            }
                            Some(MockReply::Truncated(body)) => {
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

    fn set_route(&self, path: &str, reply: MockReply) {
        self.routes.lock().unwrap().insert(path.to_string(), reply);
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn gen_keypair() -> (SigningKey, String, String) {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).expect("rng");
    let sk = SigningKey::from_bytes(&secret);
    let pk_b64 = encode_b64(&sk.verifying_key().to_bytes());
    let sk_b64 = encode_b64(&secret);
    (sk, pk_b64, sk_b64)
}

fn sign_bytes(sk: &SigningKey, data: &[u8]) -> Vec<u8> {
    let sig = sk.sign(data);
    format!("{}\n", encode_b64(&sig.to_bytes())).into_bytes()
}

fn test_download_opts() -> DownloadOptions {
    DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn build_manifest_bytes(
    app_id: &str,
    channel: &str,
    version: &str,
    expires: u64,
    rollout_percent: u8,
    payload_url: &str,
    payload_size: u64,
    payload_sha256: &str,
) -> Vec<u8> {
    let m = UpdateManifest {
        app_id: app_id.to_string(),
        channel: channel.to_string(),
        version: version.to_string(),
        minimum_supported_version: "1.0.0".to_string(),
        expires,
        rollout_percent,
        payload_url: payload_url.to_string(),
        payload_size,
        payload_sha256: payload_sha256.to_string(),
        release_notes: "Security and stability fixes.".to_string(),
    };
    serde_json::to_vec(&m).expect("serialize manifest")
}

fn seed_installed_version(updater_dir: &Path, version: &str) {
    let vdir = updater_dir.join("versions").join(version);
    fs::create_dir_all(&vdir).expect("create vdir");
    fs::write(vdir.join("index.html"), b"<html>v1</html>").expect("write v1");
    fs::write(updater_dir.join("current"), format!("{version}\n")).expect("write current");
}

fn make_check_opts(base_url: &str, pubkeys: Vec<String>) -> UpdateCheckOptions {
    UpdateCheckOptions {
        app_id: "com.wavical.test".to_string(),
        installed_version: "1.0.0".to_string(),
        channel: "stable".to_string(),
        manifest_url: format!("{base_url}/manifest.json"),
        public_keys: pubkeys,
        download_options: test_download_opts(),
        now_unix: Some(now_secs()),
    }
}

#[test]
fn test_updater_happy_path_key_rotation_ipc_and_marker_key_isolation() {
    let dir = unique_temp_dir("happy");
    seed_installed_version(&dir, "1.0.0");

    let (_sk_current, pk_current, _) = gen_keypair();
    let (sk_next, pk_next, marker_sk_b64) = gen_keypair();

    let srv = MockServer::start();
    let payload = make_zip_payload(&[("index.html", b"<html>v1.2.0</html>")]);
    let sha = sha256_hex(&payload);
    let manifest_bytes = build_manifest_bytes(
        "com.wavical.test",
        "stable",
        "1.2.0",
        now_secs() + 7 * 86400,
        100,
        &format!("{}/app-1.2.0.zip", srv.base_url),
        payload.len() as u64,
        &sha,
    );
    // Key rotation: signed by "next" key!
    let sig_bytes = sign_bytes(&sk_next, &manifest_bytes);
    srv.set_route("/manifest.json", MockReply::Ok(manifest_bytes));
    srv.set_route("/manifest.json.sig", MockReply::Ok(sig_bytes));
    srv.set_route("/app-1.2.0.zip", MockReply::Ok(payload));

    let mut caps = Capabilities::new();
    caps.grant("updater.check");
    caps.grant("updater.apply");

    let cfg = WindowConfig {
        window_id: "com.wavical.test".to_string(),
        capabilities: caps,
        updater_config: Some(UpdaterConfig {
            enabled: true,
            manifest_url: Some(format!("{}/manifest.json", srv.base_url)),
            public_keys: vec![pk_current, pk_next],
            channel: "stable".to_string(),
            check_interval_hours: 24,
            auto_download: true,
        }),
        installed_version: Some("1.0.0".to_string()),
        updater_dir: Some(dir.clone()),
        updater_download_options: Some(test_download_opts()),
        ..Default::default()
    };

    let registry = HandlerRegistry::with_core_handlers();
    let uri = "http://wavical.localhost/index.html"
        .parse::<Uri>()
        .unwrap();

    // Page supplying parameters to override source/version/channel must be rejected
    let bad_res = process_ipc_message(
        &uri,
        r#"{"id": 1, "method": "updater.check", "params": {"manifestUrl": "https://evil.com/m.json"}}"#,
        &cfg,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(bad_res.error.is_some());

    // Valid updater.check returns { available: true, version: "1.2.0", notes: ... }
    let check_res = process_ipc_message(
        &uri,
        r#"{"id": 2, "method": "updater.check"}"#,
        &cfg,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(check_res.error.is_none());
    let data = check_res.result.unwrap();
    assert_eq!(data["available"], true);
    assert_eq!(data["version"], "1.2.0");
    assert_eq!(data["notes"], "Security and stability fixes.");
    assert!(!data.to_string().contains(&marker_sk_b64));

    // Valid updater.apply switches current -> 1.2.0 and keeps 1.0.0 on disk
    let apply_res = process_ipc_message(
        &uri,
        r#"{"id": 3, "method": "updater.apply"}"#,
        &cfg,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(apply_res.error.is_none());
    assert_eq!(
        fs::read_to_string(dir.join("current")).unwrap().trim(),
        "1.2.0"
    );
    assert_eq!(
        fs::read_to_string(dir.join("previous")).unwrap().trim(),
        "1.0.0"
    );
    assert!(dir.join("versions").join("1.0.0").is_dir());
    assert!(
        dir.join("versions")
            .join("1.2.0")
            .join("index.html")
            .is_file()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_updater_rejects_security_failures_and_leaves_installed_untouched() {
    let srv = MockServer::start();
    let (sk, pk, sk_b64) = gen_keypair();
    let (sk_unknown, _, _) = gen_keypair();
    let payload = make_zip_payload(&[("index.html", b"<html>new</html>")]);
    let sha = sha256_hex(&payload);
    let now = now_secs();

    // 1. Tampered payload (SHA-256 mismatch)
    {
        let dir = unique_temp_dir("tampered_payload");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        let mut bad_payload = payload.clone();
        bad_payload[0] ^= 0xFF;
        srv.set_route("/p.zip", MockReply::Ok(bad_payload));

        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("tampered payload must fail");
        assert!(!err.to_string().contains(&sk_b64));
        assert_eq!(
            fs::read_to_string(dir.join("current")).unwrap().trim(),
            "1.0.0"
        );
        assert!(!dir.join("versions").join("1.2.0").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    // 2. Tampered manifest (1 byte)
    {
        let dir = unique_temp_dir("tampered_manifest");
        seed_installed_version(&dir, "1.0.0");
        let mut m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        let sig = sign_bytes(&sk, &m);
        m[10] ^= 0x01;
        srv.set_route("/manifest.json", MockReply::Ok(m));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sig));

        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("1-byte tampered manifest must fail");
        assert!(matches!(err, UpdaterError::SignatureInvalid));
        let _ = fs::remove_dir_all(&dir);
    }

    // 3. Unknown signing key
    {
        let dir = unique_temp_dir("unknown_key");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route(
            "/manifest.json.sig",
            MockReply::Ok(sign_bytes(&sk_unknown, &m)),
        );
        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("unknown key must fail");
        assert!(matches!(err, UpdaterError::SignatureInvalid));
        let _ = fs::remove_dir_all(&dir);
    }

    // 4. Valid signature but another appId
    {
        let dir = unique_temp_dir("wrong_appid");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.other.app",
            "stable",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("wrong appId must fail");
        assert!(matches!(err, UpdaterError::Manifest(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    // 5. Replay older version & same version (returns available: false)
    for ver in ["0.9.0", "1.0.0"] {
        let dir = unique_temp_dir("replay");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            ver,
            now + 86400,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        let res = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect("older/same version returns available: false");
        assert!(!res.available);
        let _ = fs::remove_dir_all(&dir);
    }

    // 6. Expired manifest & expires > 30 days max
    for bad_exp in [now - 10, now + 31 * 86400] {
        let dir = unique_temp_dir("expiry");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            bad_exp,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("expired or >30d manifest must fail");
        assert!(matches!(err, UpdaterError::Manifest(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    // 7. Channel mismatch
    {
        let dir = unique_temp_dir("channel");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "beta",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("channel mismatch must fail");
        assert!(matches!(err, UpdaterError::Manifest(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    // 8. Rollout bucket just below (49 < 50 -> eligible) and at percentage (50 >= 50 -> excluded)
    {
        let dir = unique_temp_dir("rollout");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            now + 86400,
            50,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        srv.set_route("/p.zip", MockReply::Ok(payload.clone()));

        fs::write(dir.join("rollout-bucket"), "50\n").unwrap();
        assert_eq!(load_or_create_rollout_bucket(&dir).unwrap(), 50);
        let res_at =
            check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
                .unwrap();
        assert!(!res_at.available);

        fs::write(dir.join("rollout-bucket"), "49\n").unwrap();
        let res_below =
            check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
                .unwrap();
        assert!(res_below.available);
        let _ = fs::remove_dir_all(&dir);
    }

    // 9. Size mismatch
    {
        let dir = unique_temp_dir("size_mismatch");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/p.zip", srv.base_url),
            payload.len() as u64 + 10,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        srv.set_route("/p.zip", MockReply::Ok(payload.clone()));
        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("size mismatch must fail");
        assert!(matches!(err, UpdaterError::Download(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    // 10. Off-list redirect
    {
        let dir = unique_temp_dir("redirect");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/redir.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        srv.set_route(
            "/redir.zip",
            MockReply::Redirect("https://evil.example.invalid/p.zip".to_string()),
        );
        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk.clone()]))
            .expect_err("off-list redirect must fail");
        assert!(matches!(err, UpdaterError::Download(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    // 11. Interrupted download
    {
        let dir = unique_temp_dir("interrupted");
        seed_installed_version(&dir, "1.0.0");
        let m = build_manifest_bytes(
            "com.wavical.test",
            "stable",
            "1.2.0",
            now + 86400,
            100,
            &format!("{}/trunc.zip", srv.base_url),
            payload.len() as u64,
            &sha,
        );
        srv.set_route("/manifest.json", MockReply::Ok(m.clone()));
        srv.set_route("/manifest.json.sig", MockReply::Ok(sign_bytes(&sk, &m)));
        srv.set_route("/trunc.zip", MockReply::Truncated(payload[..5].to_vec()));
        let err = check_and_stage_update(&dir, &make_check_opts(&srv.base_url, vec![pk]))
            .expect_err("interrupted download must fail");
        assert!(matches!(err, UpdaterError::Download(_)));
        assert_eq!(
            fs::read_to_string(dir.join("current")).unwrap().trim(),
            "1.0.0"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn test_crash_recovery_and_two_strike_health_rollback() {
    let dir = unique_temp_dir("rollback");
    seed_installed_version(&dir, "1.0.0");

    // Simulate interrupted staging directory left on disk: prepare_launch still starts 1.0.0
    fs::create_dir_all(dir.join("versions").join("1.2.0.staged")).unwrap();
    let target = prepare_launch(&dir, "1.0.0").unwrap();
    assert_eq!(target.version, "1.0.0");
    assert!(target.rolled_back_from.is_none());

    // Now simulate valid staged 1.2.0 + pending_update.json -> apply_pending_update
    let v12 = dir.join("versions").join("1.2.0");
    fs::create_dir_all(&v12).unwrap();
    fs::write(v12.join("index.html"), b"<html>v1.2.0</html>").unwrap();
    fs::write(
        dir.join("pending_update.json"),
        br#"{"version":"1.2.0","releaseNotes":"ok","payloadSha256":"00"}"#,
    )
    .unwrap();
    apply_pending_update(&dir).unwrap();

    // Launch #1 of 1.2.0 (never calls mark_app_ready -> crash #1)
    let t1 = prepare_launch(&dir, "1.0.0").unwrap();
    assert_eq!(t1.version, "1.2.0");
    assert!(t1.rolled_back_from.is_none());

    // Launch #2 of 1.2.0 (never calls mark_app_ready -> crash #2)
    let t2 = prepare_launch(&dir, "1.0.0").unwrap();
    assert_eq!(t2.version, "1.2.0");
    assert!(t2.rolled_back_from.is_none());

    // Launch #3 (after 2 failed launches -> automatic rollback to 1.0.0!)
    let t3 = prepare_launch(&dir, "1.0.0").unwrap();
    assert_eq!(t3.version, "1.0.0");
    assert_eq!(t3.rolled_back_from.as_deref(), Some("1.2.0"));
    assert_eq!(
        fs::read_to_string(dir.join("current")).unwrap().trim(),
        "1.0.0"
    );
    assert!(dir.join("versions").join("1.0.0").is_dir());

    // Subsequent launch stays healthy on 1.0.0
    mark_app_ready(&dir, "1.0.0").unwrap();
    let t4 = prepare_launch(&dir, "1.0.0").unwrap();
    assert_eq!(t4.version, "1.0.0");
    assert!(t4.rolled_back_from.is_none());

    let _ = fs::remove_dir_all(&dir);
}
