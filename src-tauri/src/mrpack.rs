use std::fs;
use std::io::{self, Read};
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha512};
use tokio::io::AsyncWriteExt;

use crate::confined_output::ConfinedOutput;
use crate::import_path::{safe_relative, OutputNames};
use crate::zip_util::{check_archive_limits, decode_zip_name, DEFAULT_ZIP_LIMITS};

const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACK_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
static DOWNLOAD_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy)]
struct HttpTimeouts {
    connect: std::time::Duration,
    read: std::time::Duration,
    total: std::time::Duration,
}

const DEFAULT_HTTP_TIMEOUTS: HttpTimeouts = HttpTimeouts {
    connect: std::time::Duration::from_secs(10),
    read: std::time::Duration::from_secs(30),
    total: std::time::Duration::from_secs(15 * 60),
};

fn permitted_download_url(url: &reqwest::Url) -> bool {
    let local_test = cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1");
    (url.scheme() == "https" || local_test)
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    pub files: Vec<MrpackFile>,
    pub dependencies: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackFile {
    pub path: String,
    pub hashes: MrpackHashes,
    pub downloads: Vec<String>,
    pub file_size: Option<u64>,
    pub env: Option<MrpackEnv>,
}

#[derive(Debug, Deserialize)]
pub struct MrpackHashes {
    pub sha512: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MrpackEnv {
    pub client: Option<String>,
}

/// mrpack 파일인지 확인
pub fn is_mrpack(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("mrpack"))
        .unwrap_or(false)
}

/// mrpack을 PrismLauncher 인스턴스로 설치
pub async fn install_mrpack<F>(
    mrpack_path: &Path,
    instances_dir: &Path,
    instance_name: &str,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize, &str),
{
    install_mrpack_with_timeouts(
        mrpack_path,
        instances_dir,
        instance_name,
        on_progress,
        DEFAULT_HTTP_TIMEOUTS,
    )
    .await
}

async fn install_mrpack_with_timeouts<F>(
    mrpack_path: &Path,
    instances_dir: &Path,
    instance_name: &str,
    on_progress: F,
    timeouts: HttpTimeouts,
) -> Result<(), String>
where
    F: Fn(usize, usize, &str),
{
    let file = fs::File::open(mrpack_path).map_err(|e| format!("mrpack 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("mrpack 읽기 실패: {}", e))?;
    check_archive_limits(&mut archive, DEFAULT_ZIP_LIMITS)?;

    // 1. modrinth.index.json 파싱
    let index = {
        let mut entry = archive
            .by_name("modrinth.index.json")
            .map_err(|_| "modrinth.index.json을 찾을 수 없습니다".to_string())?;
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .map_err(|e| format!("modrinth.index.json 읽기 실패: {}", e))?;
        serde_json::from_str::<MrpackIndex>(&content)
            .map_err(|e| format!("modrinth.index.json 파싱 실패: {}", e))?
    };

    if index.format_version != 1 || index.game != "minecraft" {
        return Err(format!(
            "지원하지 않는 MRPACK 형식: version={}, game={}",
            index.format_version, index.game
        ));
    }
    if index
        .dependencies
        .get("minecraft")
        .is_none_or(|version| version.trim().is_empty())
    {
        return Err("MRPACK에 Minecraft 버전이 없습니다".to_string());
    }
    for key in index.dependencies.keys() {
        if key != "minecraft" && crate::launcher_meta::loader_uid(key).is_none() {
            return Err(format!("지원하지 않는 MRPACK 의존성: {key}"));
        }
    }
    if index
        .dependencies
        .keys()
        .filter(|key| key.as_str() != "minecraft")
        .count()
        > 1
    {
        return Err("MRPACK에 모드 로더를 두 개 이상 지정할 수 없습니다".to_string());
    }
    let mut total_download_bytes = 0u64;
    let mut manifest_names = OutputNames::default();
    for file in &index.files {
        manifest_names.insert(&file.path)?;
        if file
            .hashes
            .sha512
            .as_ref()
            .is_none_or(|hash| hash.len() != 128 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(format!("유효한 SHA-512 해시가 없습니다: {}", file.path));
        }
        if file.env.as_ref().and_then(|env| env.client.as_deref()) != Some("unsupported")
            && file.downloads.is_empty()
        {
            return Err(format!("필수 파일 다운로드 URL 없음: {}", file.path));
        }
        let file_size = file
            .file_size
            .ok_or_else(|| format!("파일 크기가 없습니다: {}", file.path))?;
        if file_size > MAX_DOWNLOAD_BYTES {
            return Err(format!("파일 크기 제한 초과: {}", file.path));
        }
        if file.env.as_ref().and_then(|env| env.client.as_deref()) != Some("unsupported") {
            total_download_bytes = total_download_bytes
                .checked_add(file_size)
                .ok_or_else(|| "팩 다운로드 크기 계산 범위 초과".to_string())?;
            if total_download_bytes > MAX_PACK_DOWNLOAD_BYTES {
                return Err("팩 다운로드 크기 제한 초과".to_string());
            }
        }
        if let Some(client) = file.env.as_ref().and_then(|env| env.client.as_deref()) {
            if !matches!(client, "required" | "optional" | "unsupported") {
                return Err(format!("유효하지 않은 클라이언트 환경: {}", file.path));
            }
        }
        for address in &file.downloads {
            let url = reqwest::Url::parse(address)
                .map_err(|error| format!("유효하지 않은 다운로드 URL {}: {}", file.path, error))?;
            if !permitted_download_url(&url) {
                return Err(format!("HTTPS 다운로드 URL이 아닙니다: {}", file.path));
            }
        }
    }
    let mut override_names = [OutputNames::default(), OutputNames::default()];
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|error| format!("zip 엔트리 오류: {error}"))?;
        let name = decode_zip_name(entry.name_raw(), entry.name());
        for (scope, prefix) in ["overrides/", "client-overrides/"].iter().enumerate() {
            if let Some(relative) = name.strip_prefix(prefix) {
                if !relative.is_empty() {
                    override_names[scope].insert(relative)?;
                }
            }
        }
    }

    log::info!(
        "mrpack: {} v{} (MC {})",
        index.name,
        index.version_id,
        index
            .dependencies
            .get("minecraft")
            .unwrap_or(&"?".to_string())
    );

    let instance_output = ConfinedOutput::open(instances_dir)?.subdir(instance_name)?;
    let minecraft_output = instance_output.subdir(".minecraft")?;

    // 2. 모드 파일을 먼저 다운로드한다.
    let client_files: Vec<&MrpackFile> = index
        .files
        .iter()
        .filter(|f| {
            match &f.env {
                Some(env) => {
                    // unsupported가 아닌 것만
                    env.client.as_deref() != Some("unsupported")
                }
                None => true,
            }
        })
        .collect();

    let total_files = client_files.len();
    log::info!("다운로드할 모드: {}개", total_files);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("리다이렉트 횟수 제한 초과")
            } else if !permitted_download_url(attempt.url()) {
                attempt.error("허용되지 않는 리다이렉트 URL")
            } else {
                attempt.follow()
            }
        }))
        .connect_timeout(timeouts.connect)
        .read_timeout(timeouts.read)
        .timeout(timeouts.total)
        .build()
        .map_err(|e| format!("HTTP 클라이언트 생성 실패: {}", e))?;

    for (i, mod_file) in client_files.iter().enumerate() {
        let file_name = Path::new(&mod_file.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        on_progress(i, total_files, &file_name);

        if cached_file_matches(&minecraft_output, &mod_file.path, mod_file)? {
            log::info!("해시가 일치하는 기존 파일 재사용: {}", file_name);
            continue;
        }

        log::info!("다운로드 중 [{}/{}]: {}", i + 1, total_files, file_name);

        let mut errors = Vec::new();
        let mut verified_file = None;
        for url in &mod_file.downloads {
            let mut response = match client.get(url).send().await {
                Ok(response) => response,
                Err(error) => {
                    errors.push(format!("{}: {}", url, error));
                    continue;
                }
            };
            if !response.status().is_success() {
                errors.push(format!("{}: HTTP {}", url, response.status()));
                continue;
            }
            if response
                .content_length()
                .is_some_and(|size| size > mod_file.file_size.unwrap_or(0))
            {
                errors.push(format!("{}: 응답 크기 제한 초과", url));
                continue;
            }
            let temp_leaf = format!(
                ".auto-tong-download-{}",
                DOWNLOAD_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let safe_path = safe_relative(&mod_file.path)?;
            let temp_name = match safe_path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => format!(
                    "{}/{}",
                    parent.display().to_string().replace('\\', "/"),
                    temp_leaf
                ),
                _ => temp_leaf,
            };
            let temp = minecraft_output.create_file(&temp_name, false)?;
            let mut output = tokio::fs::File::from_std(temp.into_std());
            let mut hasher = Sha512::new();
            let mut received = 0u64;
            let mut transfer_error = None;
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        received += chunk.len() as u64;
                        if received > mod_file.file_size.unwrap_or(0) {
                            transfer_error = Some("매니페스트 파일 크기 초과".to_string());
                            break;
                        }
                        hasher.update(&chunk);
                        if let Err(error) = output.write_all(&chunk).await {
                            transfer_error = Some(format!("파일 쓰기 실패: {error}"));
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        transfer_error = Some(format!("전송 중단: {error}"));
                        break;
                    }
                }
            }
            if let Err(error) = output.flush().await {
                transfer_error = Some(format!("파일 flush 실패: {error}"));
            }
            drop(output);
            if let Some(error) = transfer_error {
                minecraft_output.remove_file(&temp_name)?;
                errors.push(format!("{}: {}", url, error));
                continue;
            }
            if received != mod_file.file_size.unwrap_or(0) {
                minecraft_output.remove_file(&temp_name)?;
                errors.push(format!("{}: 파일 크기 불일치", url));
                continue;
            }
            let actual = format!("{:x}", hasher.finalize());
            if !actual.eq_ignore_ascii_case(mod_file.hashes.sha512.as_deref().unwrap_or("")) {
                minecraft_output.remove_file(&temp_name)?;
                errors.push(format!("{}: 해시 불일치", url));
                continue;
            }
            verified_file = Some(temp_name);
            break;
        }
        let temp = verified_file.ok_or_else(|| {
            if errors.is_empty() {
                format!("필수 파일 다운로드 URL 없음: {}", mod_file.path)
            } else {
                format!(
                    "필수 파일 다운로드 실패 {}: {}",
                    mod_file.path,
                    errors.join("; ")
                )
            }
        })?;

        if minecraft_output.open_file(&mod_file.path)?.is_some() {
            minecraft_output.remove_file(&mod_file.path)?;
        }
        minecraft_output
            .rename_file(&temp, &mod_file.path)
            .map_err(|error| format!("검증된 파일 저장 실패 {}: {}", file_name, error))?;
    }

    // 3. 공통 설정 위에 클라이언트 설정을 적용한다.
    apply_overrides(&mut archive, &minecraft_output)?;
    on_progress(total_files, total_files, "완료");
    log::info!("모드 다운로드 완료");

    // 4. mmc-pack.json 생성
    generate_mmc_pack(&index, &instance_output)?;

    // 5. instance.cfg 생성
    generate_instance_cfg(&index, &instance_output, instance_name)?;

    log::info!("mrpack 설치 완료: {}", instance_name);
    Ok(())
}

/// mmc-pack.json 생성 (PrismLauncher 인스턴스 메타데이터)
fn generate_mmc_pack(index: &MrpackIndex, output: &ConfinedOutput) -> Result<(), String> {
    let mut components = Vec::new();

    // Minecraft
    if let Some(mc_ver) = index.dependencies.get("minecraft") {
        components.push(serde_json::json!({
            "cachedName": "Minecraft",
            "cachedVersion": mc_ver,
            "important": true,
            "uid": "net.minecraft",
            "version": mc_ver
        }));
    }

    // Fabric Loader
    if let Some(ver) = index.dependencies.get("fabric-loader") {
        // Intermediary mappings (dependency)
        if let Some(mc_ver) = index.dependencies.get("minecraft") {
            components.push(serde_json::json!({
                "cachedName": "Intermediary Mappings",
                "cachedVersion": mc_ver,
                "cachedVolatile": true,
                "dependencyOnly": true,
                "uid": "net.fabricmc.intermediary",
                "version": mc_ver
            }));
        }
        components.push(crate::launcher_meta::loader_component("fabric-loader", ver).unwrap());
    }

    // Forge
    if let Some(ver) = index.dependencies.get("forge") {
        components.push(crate::launcher_meta::loader_component("forge", ver).unwrap());
    }

    // NeoForge
    if let Some(ver) = index.dependencies.get("neoforge") {
        components.push(crate::launcher_meta::loader_component("neoforge", ver).unwrap());
    }

    // Quilt Loader
    if let Some(ver) = index.dependencies.get("quilt-loader") {
        components.push(crate::launcher_meta::loader_component("quilt-loader", ver).unwrap());
    }

    let mmc_pack = serde_json::json!({
        "components": components,
        "formatVersion": 1
    });

    let content = serde_json::to_string_pretty(&mmc_pack)
        .map_err(|e| format!("mmc-pack.json 생성 실패: {}", e))?;
    output.write_file("mmc-pack.json", content, false)?;

    Ok(())
}

/// instance.cfg 생성
fn generate_instance_cfg(
    index: &MrpackIndex,
    output: &ConfinedOutput,
    instance_name: &str,
) -> Result<(), String> {
    let loader = if index.dependencies.contains_key("fabric-loader") {
        "Fabric"
    } else if index.dependencies.contains_key("forge") {
        "Forge"
    } else if index.dependencies.contains_key("neoforge") {
        "NeoForge"
    } else if index.dependencies.contains_key("quilt-loader") {
        "Quilt"
    } else {
        "Vanilla"
    };

    let cfg = format!(
        "[General]\n\
         AutomaticJava=false\n\
         InstanceType=OneSix\n\
         OverrideJavaLocation=true\n\
         iconKey=default\n\
         name={}\n\
         ManagedPack=true\n\
         ManagedPackType=modrinth\n\
         ManagedPackVersionName={}\n\
         notes=Modrinth modpack: {} ({})\n",
        instance_name, index.version_id, index.name, loader
    );

    output.write_file("instance.cfg", cfg, false)?;

    // .packignore 생성
    output.write_file(".packignore", "", false).ok();

    Ok(())
}

fn apply_overrides(
    archive: &mut zip::ZipArchive<fs::File>,
    output: &ConfinedOutput,
) -> Result<(), String> {
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("zip 엔트리 오류: {e}"))?;
            let raw_name = decode_zip_name(entry.name_raw(), entry.name());
            let Some(relative) = raw_name.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            if entry.is_dir() || relative.ends_with('/') {
                output.create_dir(relative)?;
            } else {
                let mut file = output.create_file(relative, true)?;
                io::copy(&mut entry, &mut file)
                    .map_err(|e| format!("파일 쓰기 실패 {}: {e}", relative))?;
            }
        }
    }
    Ok(())
}

fn cached_file_matches(
    output: &ConfinedOutput,
    path: &str,
    file: &MrpackFile,
) -> Result<bool, String> {
    let expected_size = match file.file_size {
        Some(size) => size,
        None => return Ok(false),
    };
    let expected_hash = match file.hashes.sha512.as_deref() {
        Some(hash) => hash,
        None => return Ok(false),
    };
    let mut reader = match output.open_file(path)? {
        Some(reader) => reader,
        None => return Ok(false),
    };
    if reader.metadata().map_err(|error| error.to_string())?.len() != expected_size {
        return Ok(false);
    }
    let mut hasher = Sha512::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => hasher.update(&buffer[..count]),
            Err(_) => return Ok(false),
        }
    }
    Ok(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn pack_manifest(path: &Path, manifest: &serde_json::Value) {
        let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file("modrinth.index.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    fn pack(path: &Path, urls: Vec<String>) {
        let manifest = serde_json::json!({
            "formatVersion": 1,
            "game": "minecraft",
            "versionId": "test",
            "name": "test",
            "dependencies": {"minecraft": "1.21.1"},
            "files": [{
                "path": "mods/required.jar",
                "hashes": {"sha512": format!("{:x}", Sha512::digest(b"fixture"))},
                "downloads": urls,
                "fileSize": 7
            }]
        });
        pack_manifest(path, &manifest);
    }

    #[tokio::test]
    async fn manifest_and_override_parent_paths_cannot_escape_instance() {
        for override_entry in [false, true] {
            let root = tempdir().unwrap();
            let archive = root.path().join("pack.mrpack");
            let files = if override_entry {
                vec![]
            } else {
                vec![serde_json::json!({
                    "path": "../escape.txt",
                    "hashes": {"sha512": format!("{:x}", Sha512::digest(b"escape"))},
                    "downloads": ["https://example.invalid/file"], "fileSize": 6
                })]
            };
            let manifest = serde_json::json!({
                "formatVersion": 1, "game": "minecraft", "versionId": "test", "name": "test",
                "dependencies": {"minecraft": "1.21.1"}, "files": files
            });
            let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
            zip.start_file("modrinth.index.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(manifest.to_string().as_bytes()).unwrap();
            if override_entry {
                zip.start_file("overrides/../../escape.txt", SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(b"escape").unwrap();
            }
            zip.finish().unwrap();
            assert!(
                install_mrpack(&archive, root.path(), "target", |_, _, _| {})
                    .await
                    .is_err()
            );
            assert!(!root.path().join("escape.txt").exists());
            assert!(!root.path().join("target").exists());
        }
    }

    fn server(
        statuses: &[(&'static str, &'static [u8])],
    ) -> (Vec<String>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let urls = (0..statuses.len())
            .map(|i| format!("http://{address}/{i}"))
            .collect();
        let statuses = statuses.to_vec();
        let worker = std::thread::spawn(move || {
            for (status, bytes) in statuses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut connection = loop {
                    match listener.accept() {
                        Ok((connection, _)) => break connection,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("fixture 요청 누락: {error}"),
                    }
                };
                connection.set_nonblocking(false).unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                // TCP may split headers across reads. Respond only after the
                // complete GET headers arrive so closing the fixture cannot
                // reset a connection while the client is still sending them.
                let mut request = Vec::new();
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let count = connection.read(&mut chunk).unwrap();
                    assert!(count > 0, "fixture 요청 헤더가 중단됐습니다");
                    request.extend_from_slice(&chunk[..count]);
                    assert!(request.len() <= 16 * 1024, "fixture 요청 헤더 크기 초과");
                }
                write!(
                    connection,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .unwrap();
                connection.write_all(bytes).unwrap();
            }
        });
        (urls, worker)
    }

    #[tokio::test]
    async fn missing_required_url_fails() {
        let root = tempdir().unwrap();
        let archive = root.path().join("pack.mrpack");
        pack(&archive, vec![]);
        assert!(
            install_mrpack(&archive, root.path(), "target", |_, _, _| {})
                .await
                .is_err()
        );
        assert!(!root
            .path()
            .join("target/.minecraft/mods/required.jar")
            .exists());
    }

    #[tokio::test]
    async fn http_error_fails_required_file() {
        let root = tempdir().unwrap();
        let (urls, worker) = server(&[("404 Not Found", b"")]);
        let archive = root.path().join("pack.mrpack");
        pack(&archive, urls);
        let result = install_mrpack(&archive, root.path(), "target", |_, _, _| {}).await;
        worker.join().unwrap();
        assert!(result.is_err());
        assert!(!root.path().join("target/instance.cfg").exists());
    }

    #[tokio::test]
    async fn next_url_succeeds_after_http_error() {
        let root = tempdir().unwrap();
        let (urls, worker) = server(&[("404 Not Found", b""), ("200 OK", b"fixture")]);
        let archive = root.path().join("pack.mrpack");
        pack(&archive, urls);
        let result = install_mrpack(&archive, root.path(), "target", |_, _, _| {}).await;
        worker.join().unwrap();
        result.unwrap();
        assert_eq!(
            fs::read(root.path().join("target/.minecraft/mods/required.jar")).unwrap(),
            b"fixture"
        );
    }

    #[tokio::test]
    async fn unchanged_archive_succeeds_after_server_recovers() {
        let root = tempdir().unwrap();
        let (urls, worker) = server(&[("503 Service Unavailable", b""), ("200 OK", b"fixture")]);
        let archive = root.path().join("pack.mrpack");
        pack(&archive, vec![urls[0].clone()]);
        let before = crate::source::probe_fingerprint(&archive).unwrap().unwrap();
        assert!(
            install_mrpack(&archive, root.path(), "target", |_, _, _| {})
                .await
                .is_err()
        );
        install_mrpack(&archive, root.path(), "target", |_, _, _| {})
            .await
            .unwrap();
        worker.join().unwrap();
        let after = crate::source::probe_fingerprint(&archive).unwrap().unwrap();
        assert_eq!(before, after);
        assert_eq!(
            fs::read(root.path().join("target/.minecraft/mods/required.jar")).unwrap(),
            b"fixture"
        );
    }

    #[tokio::test]
    async fn stalled_body_stops_at_configured_idle_timeout() {
        let root = tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/stalled", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n")
                .unwrap();
            std::thread::sleep(Duration::from_secs(1));
        });
        let archive = root.path().join("pack.mrpack");
        pack(&archive, vec![url]);
        let started = Instant::now();
        let result = install_mrpack_with_timeouts(
            &archive,
            root.path(),
            "target",
            |_, _, _| {},
            HttpTimeouts {
                connect: Duration::from_millis(200),
                read: Duration::from_millis(100),
                total: Duration::from_millis(500),
            },
        )
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!root
            .path()
            .join("target/.minecraft/mods/required.jar")
            .exists());
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn generated_components_match_prism_ids() {
        for (loader, uid) in [
            ("forge", "net.minecraftforge"),
            ("fabric-loader", "net.fabricmc.fabric-loader"),
            ("neoforge", "net.neoforged"),
            ("quilt-loader", "org.quiltmc.quilt-loader"),
        ] {
            let root = tempdir().unwrap();
            let archive = root.path().join("pack.mrpack");
            let manifest = serde_json::json!({
                "formatVersion": 1, "game": "minecraft", "versionId": "test", "name": "test",
                "dependencies": {"minecraft": "1.21.1", loader: "1.0"}, "files": []
            });
            let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
            zip.start_file("modrinth.index.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(manifest.to_string().as_bytes()).unwrap();
            zip.finish().unwrap();
            install_mrpack(&archive, root.path(), "target", |_, _, _| {})
                .await
                .unwrap();
            let metadata: serde_json::Value = serde_json::from_slice(
                &fs::read(root.path().join("target/mmc-pack.json")).unwrap(),
            )
            .unwrap();
            assert!(metadata["components"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["uid"] == uid));
        }
    }

    #[tokio::test]
    async fn unsupported_manifest_fails_before_creating_instance() {
        for (format_version, game, dependencies) in [
            (999, "minecraft", serde_json::json!({"minecraft":"1.21.1"})),
            (1, "other", serde_json::json!({"minecraft":"1.21.1"})),
            (1, "minecraft", serde_json::json!({})),
            (
                1,
                "minecraft",
                serde_json::json!({"minecraft":"1.21.1", "unknown-loader":"1"}),
            ),
        ] {
            let root = tempdir().unwrap();
            let archive = root.path().join("pack.mrpack");
            pack_manifest(
                &archive,
                &serde_json::json!({
                    "formatVersion": format_version, "game": game, "versionId": "test",
                    "name": "test", "files": [], "dependencies": dependencies
                }),
            );
            assert!(
                install_mrpack(&archive, root.path(), "target", |_, _, _| {})
                    .await
                    .is_err()
            );
            assert!(!root.path().join("target").exists());
        }
    }

    #[tokio::test]
    async fn incomplete_file_definition_fails_before_creating_instance() {
        let root = tempdir().unwrap();
        let archive = root.path().join("pack.mrpack");
        pack_manifest(
            &archive,
            &serde_json::json!({
                "formatVersion": 1, "game": "minecraft", "versionId": "test", "name": "test",
                "dependencies": {"minecraft":"1.21.1"},
                "files": [{"path":"mods/a.jar", "hashes":{}, "downloads":[], "fileSize":1}]
            }),
        );
        assert!(
            install_mrpack(&archive, root.path(), "target", |_, _, _| {})
                .await
                .is_err()
        );
        assert!(!root.path().join("target").exists());
    }

    #[tokio::test]
    async fn same_size_bad_cache_is_replaced_after_hash_check() {
        let root = tempdir().unwrap();
        let (urls, worker) = server(&[("200 OK", b"fixture")]);
        let archive = root.path().join("pack.mrpack");
        pack(&archive, urls);
        let target = root.path().join("target/.minecraft/mods/required.jar");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"garbage").unwrap();
        let result = install_mrpack(&archive, root.path(), "target", |_, _, _| {}).await;
        worker.join().unwrap();
        result.unwrap();
        assert_eq!(fs::read(target).unwrap(), b"fixture");
    }

    #[tokio::test]
    async fn corrupt_or_short_response_never_becomes_cached_file() {
        for body in [b"garbage".as_slice(), b"cut".as_slice()] {
            let root = tempdir().unwrap();
            let (urls, worker) = server(&[("200 OK", body)]);
            let archive = root.path().join("pack.mrpack");
            pack(&archive, urls);
            let result = install_mrpack(&archive, root.path(), "target", |_, _, _| {}).await;
            worker.join().unwrap();
            assert!(result.is_err());
            assert!(!root
                .path()
                .join("target/.minecraft/mods/required.jar")
                .exists());
            assert!(!root.path().join("target/instance.cfg").exists());
        }
    }

    #[tokio::test]
    async fn override_layers_ignore_archive_entry_order() {
        for client_first in [true, false] {
            let root = tempdir().unwrap();
            let (urls, worker) = server(&[("200 OK", b"fixture")]);
            let archive = root.path().join("pack.mrpack");
            let manifest = serde_json::json!({
                "formatVersion": 1, "game": "minecraft", "versionId": "test", "name": "test",
                "dependencies": {"minecraft":"1.21.1"},
                "files": [{
                    "path":"mods/required.jar", "hashes":{"sha512":format!("{:x}", Sha512::digest(b"fixture"))},
                    "downloads":urls, "fileSize":7
                }]
            });
            let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
            zip.start_file("modrinth.index.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(manifest.to_string().as_bytes()).unwrap();
            let layers = if client_first {
                [
                    ("client-overrides/mods/required.jar", b"client".as_slice()),
                    ("overrides/mods/required.jar", b"base".as_slice()),
                ]
            } else {
                [
                    ("overrides/mods/required.jar", b"base".as_slice()),
                    ("client-overrides/mods/required.jar", b"client".as_slice()),
                ]
            };
            for (name, bytes) in layers {
                zip.start_file(name, SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            let result = install_mrpack(&archive, root.path(), "target", |_, _, _| {}).await;
            worker.join().unwrap();
            result.unwrap();
            assert_eq!(
                fs::read(root.path().join("target/.minecraft/mods/required.jar")).unwrap(),
                b"client"
            );
        }
    }

    #[tokio::test]
    async fn unsupported_client_file_is_not_downloaded() {
        let root = tempdir().unwrap();
        let archive = root.path().join("pack.mrpack");
        pack_manifest(
            &archive,
            &serde_json::json!({
                "formatVersion":1, "game":"minecraft", "versionId":"test", "name":"test",
                "dependencies":{"minecraft":"1.21.1"},
                "files":[{
                    "path":"mods/server.jar", "hashes":{"sha512":format!("{:x}", Sha512::digest(b"x"))},
                    "downloads":[], "fileSize":1, "env":{"client":"unsupported","server":"required"}
                }]
            }),
        );
        install_mrpack(&archive, root.path(), "target", |_, _, _| {})
            .await
            .unwrap();
        assert!(!root
            .path()
            .join("target/.minecraft/mods/server.jar")
            .exists());
    }

    #[tokio::test]
    async fn oversized_or_invalid_download_is_rejected_before_instance_creation() {
        for (size, address) in [
            (MAX_DOWNLOAD_BYTES + 1, "https://cdn.modrinth.com/file.jar"),
            (1, "http://example.com/file.jar"),
            (1, "not-a-url"),
        ] {
            let root = tempdir().unwrap();
            let archive = root.path().join("pack.mrpack");
            pack_manifest(
                &archive,
                &serde_json::json!({
                    "formatVersion":1, "game":"minecraft", "versionId":"test", "name":"test",
                    "dependencies":{"minecraft":"1.21.1"},
                    "files":[{
                        "path":"mods/required.jar", "hashes":{"sha512":format!("{:x}", Sha512::digest(b"x"))},
                        "downloads":[address], "fileSize":size
                    }]
                }),
            );
            assert!(
                install_mrpack(&archive, root.path(), "target", |_, _, _| {})
                    .await
                    .is_err()
            );
            assert!(!root.path().join("target").exists());
        }
    }

    #[test]
    fn redirect_url_policy_rejects_insecure_or_credential_urls() {
        for address in [
            "http://example.com/file",
            "ftp://example.com/file",
            "https://user:pass@example.com/file",
        ] {
            assert!(!permitted_download_url(
                &reqwest::Url::parse(address).unwrap()
            ));
        }
        assert!(permitted_download_url(
            &reqwest::Url::parse("https://cdn.modrinth.com/file").unwrap()
        ));
    }
}
