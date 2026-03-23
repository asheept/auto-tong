use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MrpackIndex {
    pub format_version: u32,
    pub game: String,
    pub version_id: String,
    pub name: String,
    pub summary: Option<String>,
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
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MrpackEnv {
    pub client: Option<String>,
    pub server: Option<String>,
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
    let file = fs::File::open(mrpack_path)
        .map_err(|e| format!("mrpack 파일 열기 실패: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("mrpack 읽기 실패: {}", e))?;

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

    log::info!(
        "mrpack: {} v{} (MC {})",
        index.name,
        index.version_id,
        index.dependencies.get("minecraft").unwrap_or(&"?".to_string())
    );

    let instance_dir = instances_dir.join(instance_name);
    let minecraft_dir = instance_dir.join(".minecraft");
    fs::create_dir_all(&minecraft_dir).ok();

    // 2. overrides 폴더 복사
    let override_prefixes = ["overrides/", "client-overrides/"];
    let total_entries = archive.len();
    for i in 0..total_entries {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 오류: {}", e))?;

        let raw_name = entry.name().replace('\\', "/");

        let relative = override_prefixes
            .iter()
            .find_map(|prefix| raw_name.strip_prefix(prefix).map(|r| r.to_string()));

        let relative = match relative {
            Some(r) if !r.is_empty() => r,
            _ => continue,
        };

        let target = minecraft_dir.join(&relative);

        if entry.is_dir() || relative.ends_with('/') {
            fs::create_dir_all(&target).ok();
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).ok();
            }
            let mut outfile = fs::File::create(&target)
                .map_err(|e| format!("파일 생성 실패 {}: {}", target.display(), e))?;
            io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }
    }
    log::info!("overrides 복사 완료");

    // 3. 모드 파일 다운로드 (client 전용)
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
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("HTTP 클라이언트 생성 실패: {}", e))?;

    for (i, mod_file) in client_files.iter().enumerate() {
        let file_name = Path::new(&mod_file.path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        on_progress(i, total_files, &file_name);

        let url = match mod_file.downloads.first() {
            Some(u) => u,
            None => {
                log::warn!("다운로드 URL 없음: {}", mod_file.path);
                continue;
            }
        };

        let target_path = minecraft_dir.join(&mod_file.path);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).ok();
        }

        // 이미 존재하고 크기가 같으면 건너뜀
        if target_path.exists() {
            if let Some(expected_size) = mod_file.file_size {
                if let Ok(meta) = fs::metadata(&target_path) {
                    if meta.len() == expected_size {
                        log::info!("이미 존재 (건너뜀): {}", file_name);
                        continue;
                    }
                }
            }
        }

        log::info!("다운로드 중 [{}/{}]: {}", i + 1, total_files, file_name);

        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("다운로드 실패 {}: {}", file_name, e))?;

        if !response.status().is_success() {
            log::error!("다운로드 실패 {}: HTTP {}", file_name, response.status());
            continue;
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("다운로드 읽기 실패 {}: {}", file_name, e))?;

        fs::write(&target_path, &bytes)
            .map_err(|e| format!("파일 저장 실패 {}: {}", file_name, e))?;
    }

    on_progress(total_files, total_files, "완료");
    log::info!("모드 다운로드 완료");

    // 4. mmc-pack.json 생성
    generate_mmc_pack(&index, &instance_dir)?;

    // 5. instance.cfg 생성
    generate_instance_cfg(&index, &instance_dir, instance_name)?;

    log::info!("mrpack 설치 완료: {}", instance_name);
    Ok(())
}

/// mmc-pack.json 생성 (PrismLauncher 인스턴스 메타데이터)
fn generate_mmc_pack(index: &MrpackIndex, instance_dir: &Path) -> Result<(), String> {
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
        components.push(serde_json::json!({
            "cachedName": "Fabric Loader",
            "cachedVersion": ver,
            "uid": "net.fabricmc.fabric-loader",
            "version": ver
        }));
    }

    // Forge
    if let Some(ver) = index.dependencies.get("forge") {
        components.push(serde_json::json!({
            "cachedName": "Forge",
            "cachedVersion": ver,
            "uid": "net.minecraftforge",
            "version": ver
        }));
    }

    // NeoForge
    if let Some(ver) = index.dependencies.get("neoforge") {
        components.push(serde_json::json!({
            "cachedName": "NeoForge",
            "cachedVersion": ver,
            "uid": "net.neoforged.neoforge",
            "version": ver
        }));
    }

    // Quilt Loader
    if let Some(ver) = index.dependencies.get("quilt-loader") {
        components.push(serde_json::json!({
            "cachedName": "Quilt Loader",
            "cachedVersion": ver,
            "uid": "org.quiltmc.quilt-loader",
            "version": ver
        }));
    }

    let mmc_pack = serde_json::json!({
        "components": components,
        "formatVersion": 1
    });

    let content = serde_json::to_string_pretty(&mmc_pack)
        .map_err(|e| format!("mmc-pack.json 생성 실패: {}", e))?;
    fs::write(instance_dir.join("mmc-pack.json"), content)
        .map_err(|e| format!("mmc-pack.json 쓰기 실패: {}", e))?;

    Ok(())
}

/// instance.cfg 생성
fn generate_instance_cfg(
    index: &MrpackIndex,
    instance_dir: &Path,
    instance_name: &str,
) -> Result<(), String> {
    let mc_ver = index
        .dependencies
        .get("minecraft")
        .cloned()
        .unwrap_or_default();

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
        "InstanceType=OneSix\n\
         iconKey=default\n\
         name={}\n\
         ManagedPack=true\n\
         ManagedPackType=modrinth\n\
         ManagedPackVersionName={}\n\
         notes=Modrinth modpack: {} ({})\n",
        instance_name, index.version_id, index.name, loader
    );

    fs::write(instance_dir.join("instance.cfg"), cfg)
        .map_err(|e| format!("instance.cfg 쓰기 실패: {}", e))?;

    // .packignore 생성
    fs::write(instance_dir.join(".packignore"), "").ok();

    Ok(())
}

/// mrpack에서 Minecraft 버전 추출 (Java 버전 판단용)
pub fn get_mc_version_from_mrpack(mrpack_path: &Path) -> Option<String> {
    let file = fs::File::open(mrpack_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name("modrinth.index.json").ok()?;
    let mut content = String::new();
    entry.read_to_string(&mut content).ok()?;
    let index: MrpackIndex = serde_json::from_str(&content).ok()?;
    index.dependencies.get("minecraft").cloned()
}
