use crate::{java, mrpack, prismlauncher, tracker};
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::{fs, io::{Read, Write}, net::TcpListener, path::{Path, PathBuf}, thread, time::{Duration, Instant}};
use tempfile::tempdir;
use zip::{write::SimpleFileOptions, ZipWriter};

fn zip_at(path: &Path, entries: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn index(files: Vec<Value>) -> Value {
    json!({"formatVersion":1,"game":"minecraft","versionId":"1","name":"Review fixture",
        "files":files,"dependencies":{"minecraft":"1.21.1"}})
}

fn mod_file(path: &str, urls: Vec<String>, bytes: &[u8]) -> Value {
    json!({"path":path,"downloads":urls,"fileSize":bytes.len(),
        "hashes":{"sha512":format!("{:x}", Sha512::digest(bytes))}})
}

fn mrpack_at(root: &Path, manifest: &Value, overrides: &[(&str, &[u8])]) -> PathBuf {
    let path = root.join("fixture.mrpack");
    let manifest = serde_json::to_vec(manifest).unwrap();
    let mut entries = vec![("modrinth.index.json", manifest.as_slice())];
    entries.extend_from_slice(overrides);
    zip_at(&path, &entries);
    path
}

fn serve_once(status: &str, body: &[u8]) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let body = body.to_vec();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request);
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("local fixture request not received: {e}"),
            }
        }
    });
    (format!("http://{address}/fixture.bin"), handle)
}

#[tokio::test]
async fn r01_manifest_path_escapes_minecraft_directory() {
    let root = tempdir().unwrap();
    let (url, server) = serve_once("200 OK", b"fixture");
    let pack = mrpack_at(root.path(), &index(vec![mod_file("../escaped.txt", vec![url], b"fixture")]), &[]);
    let instances = root.path().join("instances");
    assert!(mrpack::install_mrpack(&pack, &instances, "review", |_, _, _| {}).await.is_ok());
    server.join().unwrap();
    assert_eq!(fs::read(instances.join("review/escaped.txt")).unwrap(), b"fixture");
}

#[test]
fn r01_windows_rooted_path_bypasses_existing_guard() {
    let relative = Path::new(r"\auto-tong-review-escape.txt");
    assert!(!relative.is_absolute());
    assert!(!relative.to_string_lossy().contains(".."));
    assert!(!Path::new(r"C:\review\instance").join(relative).starts_with(r"C:\review\instance"));
}

#[tokio::test]
async fn r02_http_404_is_reported_as_success() {
    let root = tempdir().unwrap();
    let (url, server) = serve_once("404 Not Found", b"");
    let pack = mrpack_at(root.path(), &index(vec![mod_file("mods/a.jar", vec![url], b"new")]), &[]);
    let instances = root.path().join("instances");
    assert!(mrpack::install_mrpack(&pack, &instances, "review", |_, _, _| {}).await.is_ok());
    server.join().unwrap();
    assert!(!instances.join("review/.minecraft/mods/a.jar").exists());
    assert!(instances.join("review/instance.cfg").exists());
}

#[tokio::test]
async fn r02_missing_download_url_is_reported_as_success() {
    let root = tempdir().unwrap();
    let pack = mrpack_at(root.path(), &index(vec![mod_file("mods/a.jar", vec![], b"new")]), &[]);
    assert!(mrpack::install_mrpack(&pack, &root.path().join("instances"), "review", |_, _, _| {}).await.is_ok());
}

#[tokio::test]
async fn r03_same_size_wrong_content_bypasses_hash_check() {
    let root = tempdir().unwrap();
    let instances = root.path().join("instances");
    let target = instances.join("review/.minecraft/mods/a.jar");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"bad").unwrap();
    let pack = mrpack_at(root.path(), &index(vec![mod_file("mods/a.jar", vec!["http://127.0.0.1:9/not-used".into()], b"new")]), &[]);
    assert!(mrpack::install_mrpack(&pack, &instances, "review", |_, _, _| {}).await.is_ok());
    assert_eq!(fs::read(target).unwrap(), b"bad");
}

#[test]
fn r04_failed_zip_import_changes_existing_instance() {
    let root = tempdir().unwrap();
    let zip = root.path().join("pack.zip");
    zip_at(&zip, &[("settings.txt", b"new"), ("collision/file.txt", b"later")]);
    let instances = root.path().join("instances");
    fs::create_dir_all(instances.join("pack")).unwrap();
    fs::write(instances.join("pack/settings.txt"), b"old").unwrap();
    fs::write(instances.join("pack/collision"), b"file blocking directory").unwrap();
    assert!(prismlauncher::review_extract(&zip, &instances).is_err());
    assert_eq!(fs::read(instances.join("pack/settings.txt")).unwrap(), b"new");
}

#[test]
fn r05_vanilla_reimport_refuses_existing_instance() {
    let root = tempdir().unwrap();
    let zip = root.path().join("pack.zip");
    zip_at(&zip, &[("launcher_profiles.json", b"{}"), ("versions/1.20.1/1.20.1.json", b"{}"), ("mods/a.jar", b"fixture")]);
    let instances = root.path().join("instances");
    assert!(prismlauncher::import_vanilla_zip(&zip, &instances, |_, _| {}).is_ok());
    assert!(prismlauncher::import_vanilla_zip(&zip, &instances, |_, _| {}).is_err());
}

#[test]
fn r08_same_name_from_another_source_overwrites_instance() {
    let root = tempdir().unwrap();
    let first = root.path().join("a/pack.zip");
    let second = root.path().join("b/pack.zip");
    zip_at(&first, &[("instance.cfg", b"[General]\nname=first\n"), ("data.txt", b"first")]);
    zip_at(&second, &[("instance.cfg", b"[General]\nname=second\n"), ("data.txt", b"second")]);
    let instances = root.path().join("instances");
    prismlauncher::review_extract(&first, &instances).unwrap();
    prismlauncher::review_extract(&second, &instances).unwrap();
    assert_eq!(fs::read(instances.join("pack/data.txt")).unwrap(), b"second");
}

#[test]
fn r09_failed_history_write_still_marks_file_processed_in_memory() {
    let root = tempdir().unwrap();
    let tracker = tracker::review_tracker(root.path().join("missing/processed.json"));
    assert!(tracker.mark_processed("pack.zip", 1).is_err());
    assert!(!tracker.needs_import("pack.zip", 1));
}

#[test]
fn r09_failure_retains_duplicate_success_history() {
    let root = tempdir().unwrap();
    let tracker = tracker::review_tracker(root.path().join("processed.json"));
    tracker.mark_processed("pack.zip", 1).unwrap();
    tracker.mark_failed("pack.zip", 2).unwrap();
    assert_eq!(tracker.get_history_with_status().len(), 2);
}

#[tokio::test]
async fn r10_neoforge_metadata_uses_different_uid_from_prism() {
    let root = tempdir().unwrap();
    let mut manifest = index(vec![]);
    manifest["dependencies"]["neoforge"] = json!("21.1.1");
    let pack = mrpack_at(root.path(), &manifest, &[]);
    let instances = root.path().join("instances");
    mrpack::install_mrpack(&pack, &instances, "review", |_, _, _| {}).await.unwrap();
    let metadata: Value = serde_json::from_slice(&fs::read(instances.join("review/mmc-pack.json")).unwrap()).unwrap();
    assert!(metadata["components"].as_array().unwrap().iter().any(|c| c["uid"] == "net.neoforged.neoforge"));
}

#[tokio::test]
async fn r11_partial_java_file_is_accepted_as_installed() {
    let root = tempdir().unwrap();
    let binary = root.path().join("java/java-21/bin/javaw.exe");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, b"not an executable").unwrap();
    assert_eq!(java::ensure_java(21, Some(root.path())).await.unwrap(), binary);
}

#[tokio::test]
async fn r14_override_result_depends_on_archive_entry_order() {
    let root = tempdir().unwrap();
    let pack = mrpack_at(root.path(), &index(vec![]), &[("client-overrides/config/a.txt", b"client"), ("overrides/config/a.txt", b"base")]);
    let instances = root.path().join("instances");
    mrpack::install_mrpack(&pack, &instances, "review", |_, _, _| {}).await.unwrap();
    assert_eq!(fs::read(instances.join("review/.minecraft/config/a.txt")).unwrap(), b"base");
}

#[tokio::test]
async fn r20_unsupported_manifest_is_accepted() {
    let root = tempdir().unwrap();
    let mut manifest = index(vec![]);
    manifest["formatVersion"] = json!(999);
    manifest["game"] = json!("unsupported-game");
    manifest["dependencies"] = json!({});
    let pack = mrpack_at(root.path(), &manifest, &[]);
    let instances = root.path().join("instances");
    assert!(mrpack::install_mrpack(&pack, &instances, "review", |_, _, _| {}).await.is_ok());
    let metadata: Value = serde_json::from_slice(&fs::read(instances.join("review/mmc-pack.json")).unwrap()).unwrap();
    assert!(metadata["components"].as_array().unwrap().is_empty());
}

#[test]
fn r21_minecraft_26_1_is_assigned_java_21() {
    assert_eq!(java::required_java_version("26.1"), 21);
}
