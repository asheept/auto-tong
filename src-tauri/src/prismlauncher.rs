use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use crate::import_path::safe_join;
use crate::zip_util::{check_archive_limits, decode_zip_name, ZipLimits, DEFAULT_ZIP_LIMITS};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Command에 플랫폼별 플래그 적용
fn silent_command(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// zip 형식 판별 결과
pub enum ZipType {
    /// PrismLauncher 인스턴스 (instance.cfg / mmc-pack.json 포함)
    PrismInstance,
    /// 바닐라 런처 .minecraft 폴더를 압축한 zip
    VanillaDotMinecraft,
    /// 알 수 없는 형식 (mods만 묶은 zip 등) — PrismInstance로 취급
    Unknown,
}

/// zip 파일의 형식을 판별
pub fn detect_zip_type(zip_path: &Path) -> Result<ZipType, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;
    check_archive_limits(&mut archive, DEFAULT_ZIP_LIMITS)?;

    let mut has_instance_cfg = false;
    let mut has_mmc_pack = false;
    let mut has_vanilla_marker = false;
    let mut has_versions_dir = false;

    const VANILLA_MARKERS: &[&str] = &[
        "launcher_profiles.json",
        "launcher_settings.json",
        "launcher_accounts.json",
    ];

    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = decode_zip_name(entry.name_raw(), entry.name());

        if name == "instance.cfg" || name.ends_with("/instance.cfg") {
            has_instance_cfg = true;
        }
        if name == "mmc-pack.json" || name.ends_with("/mmc-pack.json") {
            has_mmc_pack = true;
        }
        for &marker in VANILLA_MARKERS {
            // 최상위 또는 1단계 래핑 (.minecraft/launcher_profiles.json 등)
            if name == marker || name.ends_with(&format!("/{}", marker)) {
                has_vanilla_marker = true;
            }
        }
        if name.starts_with("versions/") || name.contains("/versions/") {
            has_versions_dir = true;
        }
    }

    if has_instance_cfg && has_mmc_pack {
        let mut valid_minecraft = false;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("ZIP 엔트리 읽기 실패: {e}"))?;
            let name = decode_zip_name(entry.name_raw(), entry.name());
            if name == "mmc-pack.json" || name.ends_with("/mmc-pack.json") {
                let mut content = String::new();
                io::Read::read_to_string(&mut entry, &mut content)
                    .map_err(|e| format!("mmc-pack.json 읽기 실패: {e}"))?;
                let metadata: serde_json::Value = serde_json::from_str(&content)
                    .map_err(|e| format!("mmc-pack.json 형식 오류: {e}"))?;
                valid_minecraft = metadata["components"].as_array().is_some_and(|components| {
                    components.iter().any(|component| {
                        component["uid"] == "net.minecraft"
                            && component["version"].as_str().is_some_and(|v| !v.is_empty())
                    })
                });
                break;
            }
        }
        if !valid_minecraft {
            return Err("ZIP에 유효한 Minecraft 실행 메타데이터가 없습니다".to_string());
        }
        Ok(ZipType::PrismInstance)
    } else if has_vanilla_marker && has_versions_dir {
        Ok(ZipType::VanillaDotMinecraft)
    } else {
        Ok(ZipType::Unknown)
    }
}

#[cfg(test)]
pub fn import_modpack<F>(
    instances_dir: &Path,
    zip_path: &Path,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    extract_zip(zip_path, instances_dir, on_progress)
}

pub fn import_modpack_as<F>(
    instances_dir: &Path,
    zip_path: &Path,
    instance_name: &str,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    extract_zip_with_limits_named(
        zip_path,
        instances_dir,
        instance_name,
        DEFAULT_ZIP_LIMITS,
        on_progress,
    )
}

/// 바닐라 .minecraft zip을 PrismLauncher 인스턴스로 변환하여 가져오기
#[cfg(test)]
pub fn import_vanilla_zip<F>(
    zip_path: &Path,
    instances_dir: &Path,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instance_name = instance_name_from_zip(zip_path);
    import_vanilla_zip_as(zip_path, instances_dir, &instance_name, on_progress)
}

pub fn import_vanilla_zip_as<F>(
    zip_path: &Path,
    instances_dir: &Path,
    instance_name: &str,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    if instance_name.is_empty() {
        return Err("파일이름에서 인스턴스 이름을 추출할 수 없습니다".to_string());
    }

    let file = fs::File::open(zip_path).map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;
    check_archive_limits(&mut archive, DEFAULT_ZIP_LIMITS)?;

    // 1. 래핑 구조 감지 (예: .minecraft/mods/... 형태)
    let root_prefix = detect_vanilla_root_prefix(&mut archive);
    log::info!("바닐라 zip 루트 프리픽스: {:?}", root_prefix);

    // 2. versions/ 디렉토리에서 MC 버전 + 모드로더 파싱
    let loader_info = detect_loader_from_zip(&mut archive, root_prefix.as_deref())?;
    if loader_info.mc_version.is_empty() {
        return Err("바닐라 ZIP에서 Minecraft 버전을 찾을 수 없습니다".to_string());
    }
    log::info!(
        "바닐라 zip 감지: MC {}, 로더: {:?}",
        loader_info.mc_version,
        loader_info.loader
    );

    let mut output_names = crate::import_path::OutputNames::default();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {e}"))?;
        let name = decode_zip_name(entry.name_raw(), entry.name());
        let stripped = match &root_prefix {
            Some(prefix) => name.strip_prefix(prefix).unwrap_or(""),
            None => &name,
        };
        if !stripped.is_empty() && !should_skip_vanilla_entry(stripped) {
            output_names.insert(stripped)?;
        }
    }

    let instance_dir = safe_join(instances_dir, instance_name)?;

    // 이미 존재하는 인스턴스의 instance.cfg가 있으면 덮어쓰기 방지
    if instance_dir.join("instance.cfg").exists() {
        return Err(format!(
            "인스턴스 '{}'이(가) 이미 존재합니다. 재설치하려면 이력에서 재다운로드를 사용하세요.",
            instance_name
        ));
    }

    let instance_output =
        crate::confined_output::ConfinedOutput::open(instances_dir)?.subdir(instance_name)?;
    let minecraft_output = instance_output.subdir(".minecraft")?;

    // 3. 게임 데이터만 선별 추출 → .minecraft/ 에 배치
    let total = archive.len();
    for i in 0..total {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {}", e))?;

        let raw_name = decode_zip_name(entry.name_raw(), entry.name());
        if raw_name.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        // 루트 프리픽스 제거 (예: ".minecraft/mods/x.jar" → "mods/x.jar")
        let stripped = if let Some(ref prefix) = root_prefix {
            match raw_name.strip_prefix(prefix.as_str()) {
                Some(rest) => rest.to_string(),
                None => {
                    on_progress(i + 1, total);
                    continue;
                }
            }
        } else {
            raw_name.clone()
        };

        if stripped.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        // 스킵 대상: 런처 파일, 런타임 디렉토리
        if should_skip_vanilla_entry(&stripped) {
            on_progress(i + 1, total);
            continue;
        }

        if entry.is_dir() || raw_name.ends_with('/') {
            minecraft_output.create_dir(&stripped)?;
        } else {
            let mut outfile = minecraft_output.create_file(&stripped, false)?;
            io::copy(&mut entry, &mut outfile).map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }

        on_progress(i + 1, total);
    }

    // 3. mmc-pack.json 생성
    generate_vanilla_mmc_pack(&loader_info, &instance_output)?;

    // 4. instance.cfg 생성
    generate_vanilla_instance_cfg(&loader_info, &instance_output, instance_name)?;

    log::info!(
        "바닐라 zip → 인스턴스 '{}' 변환 완료 (MC {}, {:?})",
        instance_name,
        loader_info.mc_version,
        loader_info.loader
    );
    Ok(())
}

/// 바닐라 zip에서 스킵해야 할 엔트리인지 판별
fn should_skip_vanilla_entry(name: &str) -> bool {
    // 런처 자체 파일
    if name.starts_with("launcher_")
        || name == "clientId_v2.txt"
        || name == "treatment_tags.json"
        || name == "updateSourceCache.json"
    {
        return true;
    }
    // 런타임/캐시 디렉토리 (PrismLauncher가 자체 관리)
    let skip_prefixes = [
        "assets/",
        "libraries/",
        "versions/",
        "bin/",
        "logs/",
        "crash-reports/",
        "webcache2/",
        "tv-cache/",
        "staging/",
        "quickPlay/",
        "avatars/",
        ".mixin.out/",
    ];
    for prefix in &skip_prefixes {
        if name.starts_with(prefix) {
            return true;
        }
    }
    // 바이너리 런처 파일
    if name.ends_with("_msa_credentials.bin")
        || name.ends_with("_msa_credentials_microsoft_store.bin")
    {
        return true;
    }
    false
}

/// 바닐라 zip의 루트 프리픽스 감지 (예: ".minecraft/" 또는 "폴더명/.minecraft/")
fn detect_vanilla_root_prefix(archive: &mut zip::ZipArchive<fs::File>) -> Option<String> {
    const VANILLA_MARKERS: &[&str] = &["launcher_profiles.json", "launcher_settings.json"];

    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = decode_zip_name(entry.name_raw(), entry.name());

        for &marker in VANILLA_MARKERS {
            // 최상위에 있으면 프리픽스 없음
            if name == marker {
                return None;
            }
            // "something/launcher_profiles.json" → 프리픽스 = "something/"
            if name.ends_with(marker) {
                let prefix = &name[..name.len() - marker.len()];
                if !prefix.is_empty() {
                    return Some(prefix.to_string());
                }
            }
        }
    }
    None
}

/// 모드로더 종류
#[derive(Debug, Clone)]
pub enum ModLoader {
    Vanilla,
    Forge(String),    // forge 버전
    Fabric(String),   // fabric-loader 버전
    NeoForge(String), // neoforge 버전
    Quilt(String),    // quilt-loader 버전
}

/// zip에서 추출한 로더 정보
#[derive(Debug, Clone)]
pub struct LoaderInfo {
    pub mc_version: String,
    pub loader: ModLoader,
}

/// versions/ 디렉토리명에서 MC 버전 + 모드로더 파싱
fn detect_loader_from_zip(
    archive: &mut zip::ZipArchive<fs::File>,
    root_prefix: Option<&str>,
) -> Result<LoaderInfo, String> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    let mut version_dirs: Vec<String> = Vec::new();

    let versions_prefix = match root_prefix {
        Some(prefix) => format!("{}versions/", prefix),
        None => "versions/".to_string(),
    };

    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = decode_zip_name(entry.name_raw(), entry.name());
        if let Some(rest) = name.strip_prefix(&versions_prefix) {
            if let Some(dir_name) = rest.split('/').next() {
                if !dir_name.is_empty()
                    && dir_name != "jre_manifest.json"
                    && dir_name != "version_manifest_v2.json"
                    && seen.insert(dir_name.to_string())
                {
                    version_dirs.push(dir_name.to_string());
                }
            }
        }
    }

    log::info!("versions/ 디렉토리 목록: {:?}", version_dirs);

    if version_dirs.is_empty() {
        return Err("versions/ 디렉토리에서 버전 정보를 찾을 수 없습니다".to_string());
    }

    // 모드로더 버전 디렉토리 우선 탐색
    for dir in &version_dirs {
        if let Some(mut info) = parse_forge_version(dir)
            .or_else(|| parse_fabric_version(dir))
            .or_else(|| parse_neoforge_version(dir))
            .or_else(|| parse_quilt_version(dir))
        {
            // mc_version이 비어있으면 다단 fallback으로 결정
            if info.mc_version.is_empty() {
                // 1순위: 로더 폴더의 <dir>.json inheritsFrom (가장 정확)
                let from_json = read_inherits_from(archive, root_prefix, dir);
                // 2순위: 로더 종류별 버전 번호 규칙 추론
                let inferred = from_json.or_else(|| match &info.loader {
                    ModLoader::NeoForge(v) => infer_mc_from_neoforge(v),
                    _ => None,
                });
                // 3순위: versions/ 디렉토리의 다른 릴리스 폴더 (로더 폴더 본인 제외)
                info.mc_version = inferred.unwrap_or_else(|| {
                    version_dirs
                        .iter()
                        .find(|v| v.as_str() != dir && is_release_version(v))
                        .cloned()
                        .unwrap_or_default()
                });
            }
            if info.mc_version.is_empty() {
                return Err("모드로더는 감지했으나 Minecraft 버전을 추론할 수 없습니다".to_string());
            }
            return Ok(info);
        }
    }

    // 모드로더가 없으면 순수 MC 버전 (스냅샷 등 제외하고 릴리스 우선)
    let mc_version = version_dirs
        .iter()
        .find(|v| is_release_version(v))
        .or_else(|| version_dirs.first())
        .cloned()
        .unwrap_or_default();

    Ok(LoaderInfo {
        mc_version,
        loader: ModLoader::Vanilla,
    })
}

/// "1.16.5-forge-36.2.33" 또는 "1.16.5-rc1-forge-36.2.33" → LoaderInfo
fn parse_forge_version(dir: &str) -> Option<LoaderInfo> {
    let pos = dir.find("-forge-")?;
    let mc = &dir[..pos];
    let forge_ver = &dir[pos + "-forge-".len()..];
    if !mc.is_empty() && !forge_ver.is_empty() {
        Some(LoaderInfo {
            mc_version: mc.to_string(),
            loader: ModLoader::Forge(forge_ver.to_string()),
        })
    } else {
        None
    }
}

/// "fabric-loader-0.16.0-1.21.1" → LoaderInfo
fn parse_fabric_version(dir: &str) -> Option<LoaderInfo> {
    let rest = dir.strip_prefix("fabric-loader-")?;
    // rest = "0.16.0-1.21.1"  →  loader_ver-mc_ver
    let dash = rest.rfind('-')?;
    let loader_ver = &rest[..dash];
    let mc_ver = &rest[dash + 1..];
    if !mc_ver.is_empty() && mc_ver.contains('.') {
        Some(LoaderInfo {
            mc_version: mc_ver.to_string(),
            loader: ModLoader::Fabric(loader_ver.to_string()),
        })
    } else {
        None
    }
}

/// "neoforge-21.1.1" 또는 "1.20.4-neoforge-20.4.xxx" → LoaderInfo
fn parse_neoforge_version(dir: &str) -> Option<LoaderInfo> {
    // "1.20.4-neoforge-20.4.xxx" 형태 우선
    if let Some(pos) = dir.find("-neoforge-") {
        let mc = &dir[..pos];
        let neo_ver = &dir[pos + "-neoforge-".len()..];
        if !mc.is_empty() && !neo_ver.is_empty() {
            return Some(LoaderInfo {
                mc_version: mc.to_string(),
                loader: ModLoader::NeoForge(neo_ver.to_string()),
            });
        }
    }
    // "neoforge-21.1.1" 형태 (MC 버전은 detect_loader_from_zip에서 fallback)
    if let Some(rest) = dir.strip_prefix("neoforge-") {
        if !rest.is_empty() {
            return Some(LoaderInfo {
                mc_version: String::new(),
                loader: ModLoader::NeoForge(rest.to_string()),
            });
        }
    }
    None
}

/// "quilt-loader-0.26.0-1.21.1" → LoaderInfo
fn parse_quilt_version(dir: &str) -> Option<LoaderInfo> {
    let rest = dir.strip_prefix("quilt-loader-")?;
    let dash = rest.rfind('-')?;
    let loader_ver = &rest[..dash];
    let mc_ver = &rest[dash + 1..];
    if !mc_ver.is_empty() && mc_ver.contains('.') {
        Some(LoaderInfo {
            mc_version: mc_ver.to_string(),
            loader: ModLoader::Quilt(loader_ver.to_string()),
        })
    } else {
        None
    }
}

/// 릴리스 버전인지 (1.X.Y 형태)
fn is_release_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() >= 2 && parts[0].parse::<u32>().is_ok() && parts[1].parse::<u32>().is_ok()
}

/// 로더 폴더 내 `<dir>/<dir>.json`에서 `inheritsFrom` 값을 읽어 MC 버전 반환
fn read_inherits_from(
    archive: &mut zip::ZipArchive<fs::File>,
    root_prefix: Option<&str>,
    dir_name: &str,
) -> Option<String> {
    use std::io::Read;
    let path = match root_prefix {
        Some(p) => format!("{}versions/{}/{}.json", p, dir_name, dir_name),
        None => format!("versions/{}/{}.json", dir_name, dir_name),
    };
    let mut entry = archive.by_name(&path).ok()?;
    let mut content = String::new();
    entry.read_to_string(&mut content).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed
        .get("inheritsFrom")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// NeoForge 버전 번호에서 MC 버전 추론 (예: 21.1.218 → 1.21.1, 20.4.230 → 1.20.4)
fn infer_mc_from_neoforge(ver: &str) -> Option<String> {
    let parts: Vec<&str> = ver.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let major: u32 = parts[0].parse().ok()?;
    let minor: u32 = parts[1].parse().ok()?;
    Some(format!("1.{}.{}", major, minor))
}

/// 바닐라 zip용 mmc-pack.json 생성
fn generate_vanilla_mmc_pack(
    info: &LoaderInfo,
    output: &crate::confined_output::ConfinedOutput,
) -> Result<(), String> {
    let mut components = Vec::new();

    // Minecraft
    if !info.mc_version.is_empty() {
        components.push(serde_json::json!({
            "cachedName": "Minecraft",
            "cachedVersion": info.mc_version,
            "important": true,
            "uid": "net.minecraft",
            "version": info.mc_version
        }));
    }

    match &info.loader {
        ModLoader::Forge(ver) => {
            components.push(crate::launcher_meta::loader_component("forge", ver).unwrap());
        }
        ModLoader::Fabric(ver) => {
            if !info.mc_version.is_empty() {
                components.push(serde_json::json!({
                    "cachedName": "Intermediary Mappings",
                    "cachedVersion": info.mc_version,
                    "cachedVolatile": true,
                    "dependencyOnly": true,
                    "uid": "net.fabricmc.intermediary",
                    "version": info.mc_version
                }));
            }
            components.push(crate::launcher_meta::loader_component("fabric-loader", ver).unwrap());
        }
        ModLoader::NeoForge(ver) => {
            components.push(crate::launcher_meta::loader_component("neoforge", ver).unwrap());
        }
        ModLoader::Quilt(ver) => {
            components.push(crate::launcher_meta::loader_component("quilt-loader", ver).unwrap());
        }
        ModLoader::Vanilla => {}
    }

    let mmc_pack = serde_json::json!({
        "components": components,
        "formatVersion": 1
    });

    let content = serde_json::to_string_pretty(&mmc_pack)
        .map_err(|e| format!("mmc-pack.json 생성 실패: {}", e))?;
    output.write_file("mmc-pack.json", content, false)?;

    log::info!("mmc-pack.json 생성 완료");
    Ok(())
}

/// 바닐라 zip용 instance.cfg 생성
fn generate_vanilla_instance_cfg(
    info: &LoaderInfo,
    output: &crate::confined_output::ConfinedOutput,
    instance_name: &str,
) -> Result<(), String> {
    let loader_name = match &info.loader {
        ModLoader::Vanilla => "Vanilla",
        ModLoader::Forge(_) => "Forge",
        ModLoader::Fabric(_) => "Fabric",
        ModLoader::NeoForge(_) => "NeoForge",
        ModLoader::Quilt(_) => "Quilt",
    };

    let cfg = format!(
        "[General]\n\
         AutomaticJava=false\n\
         ConfigVersion=1.3\n\
         InstanceType=OneSix\n\
         OverrideJavaLocation=true\n\
         iconKey=default\n\
         name={}\n\
         notes=Converted from vanilla .minecraft zip ({} {})\n",
        instance_name, loader_name, info.mc_version
    );

    output.write_file("instance.cfg", cfg, false)?;

    log::info!("instance.cfg 생성 완료");
    Ok(())
}

/// PrismLauncher 데이터 루트 폴더 찾기 (표준/portable 지원)
fn select_prism_data_dir(
    exe_path: &Path,
    standard: &Path,
    explicit: &str,
) -> Result<std::path::PathBuf, String> {
    let executable_dir = exe_path
        .parent()
        .ok_or("PrismLauncher 실행 파일 경로 오류")?;
    if !explicit.trim().is_empty() {
        let selected = Path::new(explicit);
        if selected.join("instances").is_dir() {
            return Ok(selected.to_path_buf());
        }
        return Err(format!(
            "선택한 PrismLauncher 데이터 폴더에 instances가 없습니다: {}. PrismLauncher의 폴더 메뉴에서 Launcher Root를 확인하고 설정을 다시 지정하세요",
            selected.display()
        ));
    }

    let portable = executable_dir.join("instances").is_dir();
    let normal = standard.join("instances").is_dir();
    if executable_dir.join("portable.txt").exists() {
        if portable {
            return Ok(executable_dir.to_path_buf());
        }
        return Err(format!(
            "선택한 portable PrismLauncher의 instances 폴더가 없습니다: {}. Launcher Root와 portable 설정을 확인하세요",
            executable_dir.display()
        ));
    }
    match (portable, normal) {
        (true, false) => Ok(executable_dir.to_path_buf()),
        (false, true) => Ok(standard.to_path_buf()),
        (true, true) => Err(format!("PrismLauncher 데이터 폴더가 둘 다 존재합니다. 설정에서 Launcher Root를 선택하세요: {}, {}", executable_dir.display(), standard.display())),
        (false, false) => Err(format!("PrismLauncher 데이터 폴더를 찾을 수 없습니다: {}, {}. PrismLauncher의 폴더 메뉴에서 Launcher Root를 열어 설정의 데이터 폴더에 지정하세요", executable_dir.display(), standard.display())),
    }
}

pub fn prism_data_dir(exe_path: &str, explicit: &str) -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "linux")]
    let standard_base = dirs::data_dir();
    #[cfg(not(target_os = "linux"))]
    let standard_base = dirs::config_dir();
    let standard = standard_base
        .ok_or("설정 경로를 찾을 수 없습니다")?
        .join("PrismLauncher");
    select_prism_data_dir(Path::new(exe_path), &standard, explicit)
}

/// PrismLauncher instances 폴더 찾기 (플랫폼별)
pub fn prism_instances_dir(exe_path: &str, explicit: &str) -> Result<std::path::PathBuf, String> {
    prism_data_dir(exe_path, explicit).map(|d| d.join("instances"))
}

/// 파일이름에서 확장자만 제거하여 인스턴스 이름으로 사용
/// 개행/제어 문자를 제거하여 instance.cfg 인젝션 방지
#[cfg(test)]
fn instance_name_from_zip(zip_path: &Path) -> String {
    zip_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .trim()
        .trim_end_matches(&['.', ' '][..])
        .replace(&['\n', '\r', '='][..], "_")
}

#[cfg(test)]
fn extract_zip<F>(zip_path: &Path, instances_dir: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    extract_zip_with_limits(zip_path, instances_dir, DEFAULT_ZIP_LIMITS, on_progress)
}

#[cfg(test)]
fn extract_zip_with_limits<F>(
    zip_path: &Path,
    instances_dir: &Path,
    limits: ZipLimits,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instance_name = instance_name_from_zip(zip_path);
    extract_zip_with_limits_named(zip_path, instances_dir, &instance_name, limits, on_progress)
}

fn extract_zip_with_limits_named<F>(
    zip_path: &Path,
    instances_dir: &Path,
    instance_name: &str,
    limits: ZipLimits,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    if !matches!(detect_zip_type(zip_path)?, ZipType::PrismInstance) {
        return Err("지원하지 않는 Prism 인스턴스 ZIP 형식입니다".to_string());
    }
    if instance_name.is_empty() {
        return Err("파일이름에서 인스턴스 이름을 추출할 수 없습니다".to_string());
    }

    let file = fs::File::open(zip_path).map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;
    check_archive_limits(&mut archive, limits)?;

    // zip 구조 감지
    let mut root_prefix: Option<String> = None;
    {
        for i in 0..archive.len() {
            if let Ok(entry) = archive.by_index(i) {
                let name = decode_zip_name(entry.name_raw(), entry.name());
                if name == "instance.cfg" || name == "mmc-pack.json" {
                    root_prefix = None;
                    break;
                }
                if name.ends_with("/instance.cfg") || name.ends_with("/mmc-pack.json") {
                    if let Some(pos) = name.find('/') {
                        root_prefix = Some(format!("{}/", &name[..pos]));
                    }
                    break;
                }
            }
        }
    }

    log::info!(
        "zip 형식: {}, 루트: {:?}",
        if root_prefix.is_some() {
            "wrapped"
        } else {
            "flat"
        },
        root_prefix
    );

    let mut output_names = crate::import_path::OutputNames::default();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {e}"))?;
        let name = decode_zip_name(entry.name_raw(), entry.name());
        let relative = if let Some(prefix) = &root_prefix {
            if let Some(rest) = name.strip_prefix(prefix) {
                rest
            } else if name.trim_end_matches('/') == prefix.trim_end_matches('/') {
                continue;
            } else {
                &name
            }
        } else {
            &name
        };
        if !relative.is_empty() {
            output_names.insert(relative)?;
        }
    }

    let total = archive.len();
    let instance_output =
        crate::confined_output::ConfinedOutput::open(instances_dir)?.subdir(instance_name)?;

    for i in 0..total {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {}", e))?;

        let raw_name = decode_zip_name(entry.name_raw(), entry.name());
        if raw_name.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        let relative_name = if let Some(ref prefix) = root_prefix {
            if let Some(rest) = raw_name.strip_prefix(prefix.as_str()) {
                rest.to_string()
            } else if raw_name.trim_end_matches('/') == prefix.trim_end_matches('/') {
                on_progress(i + 1, total);
                continue;
            } else {
                raw_name.clone()
            }
        } else {
            raw_name.clone()
        };

        if relative_name.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        if entry.is_dir() || relative_name.ends_with('/') {
            instance_output.create_dir(&relative_name)?;
        } else {
            let mut outfile = instance_output.create_file(&relative_name, false)?;
            io::copy(&mut entry, &mut outfile).map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }

        on_progress(i + 1, total);
    }

    // instance.cfg 의 name= 값 업데이트
    if let Some(content) = instance_output.read_to_string("instance.cfg")? {
        let eol = if content.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let updated = content
            .lines()
            .map(|line| {
                if line.starts_with("name=") {
                    format!("name={}", instance_name)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(eol);
        instance_output.write_file("instance.cfg", updated, true)?;
    }

    log::info!("인스턴스 '{}' 으로 압축 해제 완료", instance_name);
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum PrismState {
    Stopped,
    Running(u32),
    GameRunning(u32),
    QueryFailed(String),
}

#[derive(Debug, PartialEq, Eq)]
enum CommitDecision {
    LeaveStopped,
    Close(u32),
    Defer,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImportPreflight {
    Ready,
    Deferred(String),
}

fn import_preflight_decision(exe_exists: bool, state: PrismState) -> ImportPreflight {
    if !exe_exists {
        return ImportPreflight::Deferred(
            "PrismLauncher 실행 파일을 찾을 수 없습니다. 설정을 확인하세요.".into(),
        );
    }
    match state {
        PrismState::Stopped | PrismState::Running(_) => ImportPreflight::Ready,
        PrismState::GameRunning(_) => ImportPreflight::Deferred(
            "게임 실행 중이므로 가져오기를 보류합니다. 게임을 종료하면 자동으로 다시 시도합니다."
                .into(),
        ),
        PrismState::QueryFailed(error) => ImportPreflight::Deferred(format!(
            "런처 상태 조회 실패로 가져오기를 보류합니다: {error}"
        )),
    }
}

pub fn import_preflight(exe_path: &str) -> ImportPreflight {
    let exists = Path::new(exe_path).is_file();
    import_preflight_with(exists, || query_prism_state(exe_path))
}

pub fn deferred_import_resume_preflight(exe_path: &str) -> ImportPreflight {
    let exists = Path::new(exe_path).is_file();
    deferred_import_resume_preflight_with(exists, || query_prism_state(exe_path))
}

fn deferred_import_resume_preflight_with(
    exists: bool,
    query: impl FnOnce() -> PrismState,
) -> ImportPreflight {
    if !exists {
        return import_preflight_decision(false, PrismState::Stopped);
    }
    match query() {
        PrismState::Stopped => ImportPreflight::Ready,
        PrismState::Running(_) => ImportPreflight::Deferred(
            "이전 설치 확정이 보류되었습니다. PrismLauncher를 종료하면 자동으로 다시 시도합니다."
                .into(),
        ),
        state => import_preflight_decision(true, state),
    }
}

fn import_preflight_with(exe_exists: bool, query: impl FnOnce() -> PrismState) -> ImportPreflight {
    if !exe_exists {
        return import_preflight_decision(false, PrismState::Stopped);
    }
    import_preflight_decision(true, query())
}

fn commit_decision(state: &PrismState) -> CommitDecision {
    match state {
        PrismState::Stopped => CommitDecision::LeaveStopped,
        PrismState::Running(pid) => CommitDecision::Close(*pid),
        PrismState::GameRunning(_) | PrismState::QueryFailed(_) => CommitDecision::Defer,
    }
}

#[cfg(target_os = "windows")]
const WINDOWS_STATE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$target = [IO.Path]::GetFullPath($env:AUTO_TONG_PRISM_EXE)
$prism = Get-Process | Where-Object {
    try {
        $_.Path -and [string]::Equals([IO.Path]::GetFullPath($_.Path), $target, [StringComparison]::OrdinalIgnoreCase)
    } catch { $false }
} | Select-Object -First 1
if ($null -eq $prism) { 'stopped'; exit 0 }
$processes = @(Get-CimInstance Win32_Process)
$descendants = @([int]$prism.Id)
do {
    $next = @($processes | Where-Object {
        ($descendants -contains [int]$_.ParentProcessId) -and
        ($descendants -notcontains [int]$_.ProcessId)
    })
    $descendants += @($next | ForEach-Object { [int]$_.ProcessId })
} while ($next.Count -gt 0)
$java = $processes | Where-Object {
    ($descendants -contains [int]$_.ProcessId) -and $_.Name -match '^java(w)?\.exe$'
} | Select-Object -First 1
if ($null -ne $java) { "game:$($prism.Id)" } else { "running:$($prism.Id)" }
"#;

#[cfg(target_os = "windows")]
fn windows_state_command(exe_path: &str) -> Command {
    let mut command = Command::new("powershell");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        WINDOWS_STATE_SCRIPT,
    ]);
    command.env("AUTO_TONG_PRISM_EXE", exe_path);
    silent_command(&mut command);
    command
}

fn parse_prism_state(success: bool, stdout: &[u8], stderr: &[u8]) -> PrismState {
    if !success {
        return PrismState::QueryFailed(String::from_utf8_lossy(stderr).trim().to_string());
    }
    let output = String::from_utf8_lossy(stdout);
    let output = output.trim();
    if output == "stopped" {
        PrismState::Stopped
    } else if let Some(pid) = output
        .strip_prefix("running:")
        .and_then(|value| value.parse().ok())
    {
        PrismState::Running(pid)
    } else if let Some(pid) = output
        .strip_prefix("game:")
        .and_then(|value| value.parse().ok())
    {
        PrismState::GameRunning(pid)
    } else {
        PrismState::QueryFailed(format!("알 수 없는 프로세스 조회 결과: {output}"))
    }
}

fn query_prism_state(exe_path: &str) -> PrismState {
    #[cfg(target_os = "windows")]
    {
        match windows_state_command(exe_path).output() {
            Ok(output) => {
                parse_prism_state(output.status.success(), &output.stdout, &output.stderr)
            }
            Err(error) => PrismState::QueryFailed(error.to_string()),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = match Command::new("pgrep").args(["-f", exe_path]).output() {
            Ok(output) => output,
            Err(error) => return PrismState::QueryFailed(error.to_string()),
        };
        if output.status.code() == Some(1) {
            return PrismState::Stopped;
        }
        if !output.status.success() {
            return PrismState::QueryFailed(String::from_utf8_lossy(&output.stderr).to_string());
        }
        let pid = match String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .and_then(|line| line.trim().parse::<u32>().ok())
        {
            Some(pid) => pid,
            None => {
                return PrismState::QueryFailed(
                    "PrismLauncher PID를 해석할 수 없습니다".to_string(),
                )
            }
        };
        match Command::new("pgrep")
            .args(["-P", &pid.to_string(), "java"])
            .output()
        {
            Ok(child) if child.status.success() => PrismState::GameRunning(pid),
            Ok(child) if child.status.code() == Some(1) => PrismState::Running(pid),
            Ok(child) => {
                PrismState::QueryFailed(String::from_utf8_lossy(&child.stderr).to_string())
            }
            Err(error) => PrismState::QueryFailed(error.to_string()),
        }
    }
}

fn request_graceful_close(pid: u32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let output = graceful_close_command(pid)
            .output()
            .map_err(|e| format!("PrismLauncher 종료 요청 실패: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "PrismLauncher 종료 요청 거부: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output()
            .map_err(|e| format!("PrismLauncher 종료 요청 실패: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "PrismLauncher 종료 요청 거부: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn graceful_close_command(pid: u32) -> Command {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string()]);
    silent_command(&mut command);
    command
}

/// Returns true only when this function closed a previously running launcher.
pub async fn prepare_for_commit(exe_path: &str) -> Result<bool, String> {
    if !Path::new(exe_path).is_file() {
        return Err(format!(
            "PrismLauncher 실행 파일을 찾을 수 없습니다: {exe_path}. 설정을 확인하세요"
        ));
    }
    prepare_for_commit_with(
        || query_prism_state(exe_path),
        request_graceful_close,
        std::time::Duration::from_millis(250),
    )
    .await
}

async fn prepare_for_commit_with(
    mut query: impl FnMut() -> PrismState,
    mut close: impl FnMut(u32) -> Result<(), String>,
    poll_delay: std::time::Duration,
) -> Result<bool, String> {
    let state = query();
    match commit_decision(&state) {
        CommitDecision::LeaveStopped => Ok(false),
        CommitDecision::Defer => Err(match state {
            PrismState::GameRunning(_) => "게임 실행 중이므로 설치 확정을 보류합니다. 게임을 종료한 뒤 다시 시도하세요".into(),
            PrismState::QueryFailed(error) => format!("런처 상태 조회 실패로 설치 확정을 보류합니다: {error}. PrismLauncher를 종료한 뒤 다시 시도하세요"),
            _ => unreachable!(),
        }),
        CommitDecision::Close(pid) => {
            close(pid)?;
            for _ in 0..40 {
                tokio::time::sleep(poll_delay).await;
                match query() {
                    PrismState::Stopped => return Ok(true),
                    PrismState::Running(current) if current == pid => {},
                    PrismState::Running(_) | PrismState::GameRunning(_) => return Err("런처 또는 게임 상태가 종료 대기 중 바뀌어 설치를 보류합니다".into()),
                    PrismState::QueryFailed(error) => return Err(format!("런처 종료 확인 실패: {error}")),
                }
            }
            Err("PrismLauncher가 정상 종료되지 않아 설치를 보류합니다. 런처를 직접 종료한 뒤 다시 시도하세요".into())
        }
    }
}

pub fn verify_stopped(exe_path: &str) -> Result<(), String> {
    require_stopped(query_prism_state(exe_path))
}

fn require_stopped(state: PrismState) -> Result<(), String> {
    match state {
        PrismState::Stopped => Ok(()),
        PrismState::Running(_) | PrismState::GameRunning(_) => Err(
            "설치 확정 직전에 PrismLauncher 또는 게임이 실행되었습니다. 종료한 뒤 다시 시도하세요"
                .into(),
        ),
        PrismState::QueryFailed(error) => {
            Err(format!("설치 확정 직전 런처 상태 조회 실패: {error}"))
        }
    }
}

fn launcher_restart_command(exe_path: &str, data_dir: &str) -> Result<Command, String> {
    let instances = prism_instances_dir(exe_path, data_dir)?;
    let root = instances.parent().ok_or("PrismLauncher 데이터 폴더 없음")?;
    let mut command = Command::new(exe_path);
    command.arg("--dir").arg(root);
    silent_command(&mut command);
    Ok(command)
}

pub async fn restore_after_commit(
    exe_path: &str,
    data_dir: &str,
    was_running: bool,
) -> Result<(), String> {
    if !was_running {
        return Ok(());
    }
    let mut restart = launcher_restart_command(exe_path, data_dir)?;
    restore_after_commit_with(
        was_running,
        || query_prism_state(exe_path),
        || {
            restart
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("PrismLauncher 재실행 실패: {e}"))
        },
        std::time::Duration::from_millis(250),
    )
    .await
}

async fn restore_after_commit_with(
    was_running: bool,
    mut query: impl FnMut() -> PrismState,
    mut launch: impl FnMut() -> Result<(), String>,
    poll_delay: std::time::Duration,
) -> Result<(), String> {
    if !was_running {
        return Ok(());
    }
    match query() {
        PrismState::Running(_) | PrismState::GameRunning(_) => return Ok(()),
        PrismState::QueryFailed(error) => {
            return Err(format!("런처 재실행 전 상태 조회 실패: {error}"))
        }
        PrismState::Stopped => {}
    }
    launch()?;
    for _ in 0..40 {
        tokio::time::sleep(poll_delay).await;
        match query() {
            PrismState::Running(_) | PrismState::GameRunning(_) => return Ok(()),
            PrismState::Stopped => {}
            PrismState::QueryFailed(error) => {
                return Err(format!("PrismLauncher 재실행 확인 실패: {error}"))
            }
        }
    }
    Err("PrismLauncher 재실행을 확인할 수 없습니다. 런처를 직접 열어 설치 상태를 확인하세요".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn import_preflight_defers_only_real_blockers() {
        let mut queries = 0;
        assert_eq!(
            import_preflight_with(false, || {
                queries += 1;
                PrismState::Stopped
            }),
            ImportPreflight::Deferred(
                "PrismLauncher 실행 파일을 찾을 수 없습니다. 설정을 확인하세요.".into()
            )
        );
        assert_eq!(queries, 0);
        assert_eq!(
            import_preflight_with(true, || {
                queries += 1;
                PrismState::Stopped
            }),
            ImportPreflight::Ready
        );
        assert_eq!(queries, 1);
        assert_eq!(
            import_preflight_decision(true, PrismState::Running(42)),
            ImportPreflight::Ready
        );
        assert!(matches!(
            import_preflight_decision(true, PrismState::GameRunning(42)),
            ImportPreflight::Deferred(_)
        ));
        assert!(matches!(
            import_preflight_decision(true, PrismState::QueryFailed("denied".into())),
            ImportPreflight::Deferred(message) if message.contains("denied")
        ));
        assert_eq!(
            deferred_import_resume_preflight_with(true, || PrismState::Stopped),
            ImportPreflight::Ready
        );
        assert!(matches!(
            deferred_import_resume_preflight_with(true, || PrismState::Running(42)),
            ImportPreflight::Deferred(_)
        ));

        let mut heavy_work = 0;
        for preflight in [
            import_preflight_with(true, || PrismState::GameRunning(42)),
            import_preflight_with(true, || PrismState::QueryFailed("denied".into())),
            import_preflight_with(true, || PrismState::Stopped),
        ] {
            if preflight == ImportPreflight::Ready {
                heavy_work += 1;
            }
        }
        assert_eq!(heavy_work, 1);
    }

    #[test]
    fn selected_launcher_data_wins_and_ambiguous_install_requires_choice() {
        let root = tempdir().unwrap();
        let standard = root.path().join("standard Prism");
        let portable = root.path().join("portable Prism");
        fs::create_dir_all(standard.join("instances")).unwrap();
        fs::create_dir_all(portable.join("instances")).unwrap();
        let exe = portable.join("prismlauncher.exe");
        fs::write(&exe, "").unwrap();

        assert!(select_prism_data_dir(&exe, &standard, "").is_err());
        assert_eq!(
            select_prism_data_dir(&exe, &standard, standard.to_str().unwrap()).unwrap(),
            standard
        );
        fs::write(portable.join("portable.txt"), "").unwrap();
        assert_eq!(
            select_prism_data_dir(&exe, &standard, "").unwrap(),
            portable
        );
        let missing = select_prism_data_dir(
            &exe,
            &standard,
            root.path().join("missing").to_str().unwrap(),
        )
        .unwrap_err();
        assert!(missing.contains("데이터 폴더"));
        assert!(missing.contains("Launcher Root"));
    }

    #[test]
    fn process_lookup_failure_is_not_a_running_or_stopped_state() {
        assert_eq!(
            parse_prism_state(true, b"stopped", b""),
            PrismState::Stopped
        );
        assert_eq!(
            parse_prism_state(true, b"running:42", b""),
            PrismState::Running(42)
        );
        assert_eq!(
            parse_prism_state(true, b"game:42", b""),
            PrismState::GameRunning(42)
        );
        assert!(matches!(
            parse_prism_state(false, b"running:42", b"access denied"),
            PrismState::QueryFailed(_)
        ));
        assert!(matches!(
            parse_prism_state(true, b"garbage", b""),
            PrismState::QueryFailed(_)
        ));
        for state in [
            parse_prism_state(false, b"running:42", b"access denied"),
            parse_prism_state(true, b"garbage", b""),
            PrismState::GameRunning(42),
        ] {
            assert_eq!(commit_decision(&state), CommitDecision::Defer);
        }
        assert_eq!(
            commit_decision(&PrismState::Running(42)),
            CommitDecision::Close(42)
        );
        assert!(require_stopped(PrismState::Stopped).is_ok());
        assert!(require_stopped(PrismState::GameRunning(42)).is_err());
        assert!(require_stopped(PrismState::Running(42)).is_err());
        assert!(require_stopped(PrismState::QueryFailed("denied".into())).is_err());
    }

    #[tokio::test]
    async fn fake_process_states_control_close_and_restart_confirmation() {
        use std::cell::Cell;
        use std::collections::VecDeque;
        let mut states = VecDeque::from([
            PrismState::Running(42),
            PrismState::Running(42),
            PrismState::Stopped,
        ]);
        let closed = Cell::new(None);
        assert!(prepare_for_commit_with(
            || states.pop_front().unwrap(),
            |pid| {
                closed.set(Some(pid));
                Ok(())
            },
            std::time::Duration::ZERO,
        )
        .await
        .unwrap());
        assert_eq!(closed.get(), Some(42));

        let mut starts = 0;
        assert!(restore_after_commit_with(
            false,
            || panic!("should not query"),
            || {
                starts += 1;
                Ok(())
            },
            std::time::Duration::ZERO
        )
        .await
        .is_ok());
        assert_eq!(starts, 0);
        let mut after_launch = VecDeque::from([
            PrismState::Stopped,
            PrismState::Stopped,
            PrismState::Running(99),
        ]);
        assert!(restore_after_commit_with(
            true,
            || after_launch.pop_front().unwrap(),
            || {
                starts += 1;
                Ok(())
            },
            std::time::Duration::ZERO
        )
        .await
        .is_ok());
        assert_eq!(starts, 1);

        let mut game = VecDeque::from([PrismState::GameRunning(42)]);
        assert!(prepare_for_commit_with(
            || game.pop_front().unwrap(),
            |_| panic!("should not close"),
            std::time::Duration::ZERO
        )
        .await
        .is_err());
        let mut lookup_error = VecDeque::from([PrismState::QueryFailed("denied".into())]);
        assert!(prepare_for_commit_with(
            || lookup_error.pop_front().unwrap(),
            |_| panic!("should not close"),
            std::time::Duration::ZERO
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn restart_uses_selected_data_root_and_leaves_stopped_launcher_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let portable = temp.path().join("portable Prism");
        let selected = temp.path().join("O'Neil 한글 데이터");
        fs::create_dir_all(portable.join("instances")).unwrap();
        fs::create_dir_all(selected.join("instances")).unwrap();
        fs::write(portable.join("portable.txt"), "").unwrap();
        let executable = portable.join("prismlauncher.exe");
        let command =
            launcher_restart_command(executable.to_str().unwrap(), selected.to_str().unwrap())
                .unwrap();
        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new("--dir"), selected.as_os_str()]
        );
        assert!(
            restore_after_commit("missing launcher", "missing data", false)
                .await
                .is_ok()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn nested_java_process_is_detected_as_running_game() {
        let mock = r#"
function Get-Process { [pscustomobject]@{ Path = $env:AUTO_TONG_PRISM_EXE; Id = 100 } }
function Get-CimInstance { @(
    [pscustomobject]@{ ProcessId = 100; ParentProcessId = 0; Name = 'prismlauncher.exe' },
    [pscustomobject]@{ ProcessId = 200; ParentProcessId = 100; Name = 'helper.exe' },
    [pscustomobject]@{ ProcessId = 300; ParentProcessId = 200; Name = 'javaw.exe' }
) }
"#;
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("{mock}\n{WINDOWS_STATE_SCRIPT}"),
            ])
            .env("AUTO_TONG_PRISM_EXE", r"C:\fixture\PrismLauncher.exe")
            .output()
            .unwrap();
        assert_eq!(
            parse_prism_state(output.status.success(), &output.stdout, &output.stderr),
            PrismState::GameRunning(100)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn launcher_close_request_never_uses_forced_termination() {
        let command = graceful_close_command(42);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, vec!["/PID", "42"]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn apostrophe_path_is_only_passed_as_environment_data() {
        let path = r"C:\Users\O'Neil\Prism Launcher\prismlauncher.exe";
        let command = windows_state_command(path);
        assert!(command
            .get_args()
            .all(|arg| !arg.to_string_lossy().contains("O'Neil")));
        assert!(command.get_envs().any(|(key, value)| {
            key == "AUTO_TONG_PRISM_EXE" && value.is_some_and(|value| value == path)
        }));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn static_windows_query_handles_missing_apostrophe_path() {
        assert_eq!(
            query_prism_state(r"C:\Users\O'Neil\never-existing-prism-fixture.exe"),
            PrismState::Stopped
        );
    }

    #[test]
    fn unknown_zip_does_not_create_instance() {
        let root = tempdir().unwrap();
        let archive = root.path().join("unknown.zip");
        let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("mods/a.jar", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"fixture").unwrap();
        zip.finish().unwrap();
        let instances = root.path().join("instances");
        assert!(extract_zip(&archive, &instances, |_, _| {}).is_err());
        assert!(!instances.join("unknown").exists());
    }

    #[test]
    fn archive_limit_rejection_preserves_existing_instance() {
        let root = tempdir().unwrap();
        let archive = root.path().join("pack.zip");
        let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
        for (name, content) in [
            ("instance.cfg", "[General]\nname=pack"),
            (
                "mmc-pack.json",
                r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#,
            ),
            ("mods/a.jar", "payload"),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        let instances = root.path().join("instances");
        let installed = instances.join("pack");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("instance.cfg"), "previous install").unwrap();
        let limits = [
            ZipLimits {
                entries: 2,
                file_bytes: 100,
                total_bytes: 200,
            },
            ZipLimits {
                entries: 3,
                file_bytes: 4,
                total_bytes: 200,
            },
            ZipLimits {
                entries: 3,
                file_bytes: 100,
                total_bytes: 10,
            },
        ];
        for limit in limits {
            assert!(extract_zip_with_limits(&archive, &instances, limit, |_, _| {}).is_err());
            assert_eq!(
                fs::read_to_string(installed.join("instance.cfg")).unwrap(),
                "previous install"
            );
            assert!(!installed.join("mods/a.jar").exists());
        }
    }

    #[test]
    fn prism_and_vanilla_parent_paths_cannot_escape_instance() {
        for vanilla in [false, true] {
            let root = tempdir().unwrap();
            let archive = root.path().join("pack.zip");
            let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
            let entries = if vanilla {
                vec![
                    ("launcher_profiles.json", "{}"),
                    ("versions/1.20.4/1.20.4.json", "{}"),
                ]
            } else {
                vec![
                    ("instance.cfg", "[General]"),
                    (
                        "mmc-pack.json",
                        r#"{"components":[{"uid":"net.minecraft","version":"1.20.4"}]}"#,
                    ),
                ]
            };
            for (name, content) in entries {
                zip.start_file(name, SimpleFileOptions::default()).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.start_file("../../escape.txt", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"escape").unwrap();
            zip.finish().unwrap();
            let instances = root.path().join("instances");
            let result = if vanilla {
                import_vanilla_zip(&archive, &instances, |_, _| {})
            } else {
                extract_zip(&archive, &instances, |_, _| {})
            };
            assert!(result.is_err());
            assert!(!root.path().join("escape.txt").exists());
        }
    }
}
