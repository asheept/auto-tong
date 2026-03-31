use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

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
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("zip 읽기 실패: {}", e))?;

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
        let name = entry.name().replace('\\', "/");

        if name == "instance.cfg" || name.ends_with("/instance.cfg") {
            has_instance_cfg = true;
        }
        if name == "mmc-pack.json" || name.ends_with("/mmc-pack.json") {
            has_mmc_pack = true;
        }
        for &marker in VANILLA_MARKERS {
            if name == marker {
                has_vanilla_marker = true;
            }
        }
        if name.starts_with("versions/") {
            has_versions_dir = true;
        }
    }

    if has_instance_cfg || has_mmc_pack {
        Ok(ZipType::PrismInstance)
    } else if has_vanilla_marker && has_versions_dir {
        Ok(ZipType::VanillaDotMinecraft)
    } else {
        Ok(ZipType::Unknown)
    }
}

pub fn import_modpack<F>(exe_path: &str, zip_path: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instances_dir = prism_instances_dir(exe_path)?;
    extract_zip(zip_path, &instances_dir, on_progress)
}

/// 바닐라 .minecraft zip을 PrismLauncher 인스턴스로 변환하여 가져오기
pub fn import_vanilla_zip<F>(
    zip_path: &Path,
    instances_dir: &Path,
    on_progress: F,
) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instance_name = instance_name_from_zip(zip_path);
    if instance_name.is_empty() {
        return Err("파일이름에서 인스턴스 이름을 추출할 수 없습니다".to_string());
    }

    let file = fs::File::open(zip_path)
        .map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("zip 읽기 실패: {}", e))?;

    // 1. versions/ 디렉토리에서 MC 버전 + 모드로더 파싱
    let loader_info = detect_loader_from_zip(&mut archive)?;
    log::info!(
        "바닐라 zip 감지: MC {}, 로더: {:?}",
        loader_info.mc_version,
        loader_info.loader
    );

    let instance_dir = instances_dir.join(&instance_name);
    let minecraft_dir = instance_dir.join(".minecraft");
    fs::create_dir_all(&minecraft_dir)
        .map_err(|e| format!("디렉토리 생성 실패: {}", e))?;

    // 2. 게임 데이터만 선별 추출 → .minecraft/ 에 배치
    let total = archive.len();
    for i in 0..total {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {}", e))?;

        let raw_name = entry.name().replace('\\', "/");
        if raw_name.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        // 스킵 대상: 런처 파일, 런타임 디렉토리
        if should_skip_vanilla_entry(&raw_name) {
            on_progress(i + 1, total);
            continue;
        }

        // 경로 탐색 공격 차단
        if raw_name.contains("..") || Path::new(&raw_name).is_absolute() {
            log::warn!("위험한 zip 엔트리 차단: {}", raw_name);
            on_progress(i + 1, total);
            continue;
        }

        let target = minecraft_dir.join(&raw_name);

        if entry.is_dir() || raw_name.ends_with('/') {
            fs::create_dir_all(&target)
                .map_err(|e| format!("디렉토리 생성 실패: {}", e))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("디렉토리 생성 실패: {}", e))?;
            }
            let mut outfile = fs::File::create(&target)
                .map_err(|e| format!("파일 생성 실패: {}", e))?;
            io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }

        on_progress(i + 1, total);
    }

    // 3. mmc-pack.json 생성
    generate_vanilla_mmc_pack(&loader_info, &instance_dir)?;

    // 4. instance.cfg 생성
    generate_vanilla_instance_cfg(&loader_info, &instance_dir, &instance_name)?;

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
    if name.starts_with("launcher_") || name == "clientId_v2.txt"
        || name == "treatment_tags.json" || name == "updateSourceCache.json"
    {
        return true;
    }
    // 런타임/캐시 디렉토리 (PrismLauncher가 자체 관리)
    let skip_prefixes = [
        "assets/", "libraries/", "versions/", "bin/",
        "logs/", "crash-reports/", "webcache2/", "tv-cache/",
        "staging/", "quickPlay/", "avatars/",
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

/// 모드로더 종류
#[derive(Debug, Clone)]
pub enum ModLoader {
    Vanilla,
    Forge(String),      // forge 버전
    Fabric(String),     // fabric-loader 버전
    NeoForge(String),   // neoforge 버전
    Quilt(String),      // quilt-loader 버전
}

/// zip에서 추출한 로더 정보
#[derive(Debug, Clone)]
pub struct LoaderInfo {
    pub mc_version: String,
    pub loader: ModLoader,
}

/// versions/ 디렉토리명에서 MC 버전 + 모드로더 파싱
fn detect_loader_from_zip(archive: &mut zip::ZipArchive<fs::File>) -> Result<LoaderInfo, String> {
    let mut version_dirs: Vec<String> = Vec::new();

    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().replace('\\', "/");
        // versions/X.Y.Z/ 또는 versions/1.16.5-forge-36.2.33/ 형태
        if let Some(rest) = name.strip_prefix("versions/") {
            if let Some(dir_name) = rest.split('/').next() {
                if !dir_name.is_empty()
                    && dir_name != "jre_manifest.json"
                    && dir_name != "version_manifest_v2.json"
                    && !version_dirs.contains(&dir_name.to_string())
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
        // Forge: "1.16.5-forge-36.2.33"
        if let Some(info) = parse_forge_version(dir) {
            return Ok(info);
        }
        // Fabric: "fabric-loader-0.16.0-1.21.1"
        if let Some(info) = parse_fabric_version(dir) {
            return Ok(info);
        }
        // NeoForge: "neoforge-21.1.1" 또는 "1.20.4-neoforge-20.4.xxx"
        if let Some(info) = parse_neoforge_version(dir) {
            return Ok(info);
        }
        // Quilt: "quilt-loader-0.26.0-1.21.1"
        if let Some(info) = parse_quilt_version(dir) {
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

/// "1.16.5-forge-36.2.33" → LoaderInfo
fn parse_forge_version(dir: &str) -> Option<LoaderInfo> {
    // 패턴: {mc_version}-forge-{forge_version}
    let parts: Vec<&str> = dir.splitn(3, '-').collect();
    if parts.len() == 3 && parts[1] == "forge" {
        return Some(LoaderInfo {
            mc_version: parts[0].to_string(),
            loader: ModLoader::Forge(parts[2].to_string()),
        });
    }
    None
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
    if let Some(rest) = dir.strip_prefix("neoforge-") {
        // "neoforge-21.1.1" — MC 버전은 NeoForge 메이저에서 추론 필요
        // NeoForge 21.x = MC 1.21.x 등. 정확한 매핑은 어려우므로 버전만 저장
        return Some(LoaderInfo {
            mc_version: String::new(), // 아래에서 fallback 처리
            loader: ModLoader::NeoForge(rest.to_string()),
        });
    }
    // "1.20.4-neoforge-20.4.xxx"
    let parts: Vec<&str> = dir.splitn(3, '-').collect();
    if parts.len() == 3 && parts[1] == "neoforge" {
        return Some(LoaderInfo {
            mc_version: parts[0].to_string(),
            loader: ModLoader::NeoForge(parts[2].to_string()),
        });
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

/// 바닐라 zip용 mmc-pack.json 생성
fn generate_vanilla_mmc_pack(info: &LoaderInfo, instance_dir: &Path) -> Result<(), String> {
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
            components.push(serde_json::json!({
                "cachedName": "Forge",
                "cachedVersion": ver,
                "uid": "net.minecraftforge",
                "version": ver
            }));
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
            components.push(serde_json::json!({
                "cachedName": "Fabric Loader",
                "cachedVersion": ver,
                "uid": "net.fabricmc.fabric-loader",
                "version": ver
            }));
        }
        ModLoader::NeoForge(ver) => {
            components.push(serde_json::json!({
                "cachedName": "NeoForge",
                "cachedVersion": ver,
                "uid": "net.neoforged.neoforge",
                "version": ver
            }));
        }
        ModLoader::Quilt(ver) => {
            components.push(serde_json::json!({
                "cachedName": "Quilt Loader",
                "cachedVersion": ver,
                "uid": "org.quiltmc.quilt-loader",
                "version": ver
            }));
        }
        ModLoader::Vanilla => {}
    }

    let mmc_pack = serde_json::json!({
        "components": components,
        "formatVersion": 1
    });

    let content = serde_json::to_string_pretty(&mmc_pack)
        .map_err(|e| format!("mmc-pack.json 생성 실패: {}", e))?;
    fs::write(instance_dir.join("mmc-pack.json"), content)
        .map_err(|e| format!("mmc-pack.json 쓰기 실패: {}", e))?;

    log::info!("mmc-pack.json 생성 완료");
    Ok(())
}

/// 바닐라 zip용 instance.cfg 생성
fn generate_vanilla_instance_cfg(
    info: &LoaderInfo,
    instance_dir: &Path,
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

    fs::write(instance_dir.join("instance.cfg"), cfg)
        .map_err(|e| format!("instance.cfg 쓰기 실패: {}", e))?;

    log::info!("instance.cfg 생성 완료");
    Ok(())
}

/// PrismLauncher 데이터 루트 폴더 찾기 (표준/portable 지원)
pub fn prism_data_dir(exe_path: &str) -> Result<std::path::PathBuf, String> {
    // 1. 표준 경로
    let standard = dirs::config_dir()
        .ok_or("설정 경로를 찾을 수 없습니다")?
        .join("PrismLauncher");
    if standard.join("instances").exists() {
        log::info!("PrismLauncher 데이터 폴더 (표준): {}", standard.display());
        return Ok(standard);
    }

    // 2. macOS 대체 경로
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let mac_path = home.join("Library/Application Support/PrismLauncher");
            if mac_path.join("instances").exists() {
                log::info!("PrismLauncher 데이터 폴더 (macOS): {}", mac_path.display());
                return Ok(mac_path);
            }
        }
    }

    // 3. 포터블 설치 (exe와 같은 폴더)
    let exe_parent = Path::new(exe_path).parent().ok_or("exe 경로 오류")?;
    if exe_parent.join("instances").exists() {
        log::info!("PrismLauncher 데이터 폴더 (portable): {}", exe_parent.display());
        return Ok(exe_parent.to_path_buf());
    }

    Err(format!(
        "PrismLauncher 데이터 폴더를 찾을 수 없습니다 (표준: {}, portable: {})",
        standard.display(),
        exe_parent.display()
    ))
}

/// PrismLauncher instances 폴더 찾기 (플랫폼별)
pub fn prism_instances_dir(exe_path: &str) -> Result<std::path::PathBuf, String> {
    prism_data_dir(exe_path).map(|d| d.join("instances"))
}

/// 파일이름에서 확장자만 제거하여 인스턴스 이름으로 사용
fn instance_name_from_zip(zip_path: &Path) -> String {
    zip_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .trim()
        .trim_end_matches(&['.', ' '][..])
        .to_string()
}

fn extract_zip<F>(zip_path: &Path, instances_dir: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instance_name = instance_name_from_zip(zip_path);
    if instance_name.is_empty() {
        return Err("파일이름에서 인스턴스 이름을 추출할 수 없습니다".to_string());
    }

    let file =
        fs::File::open(zip_path).map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;

    // zip 구조 감지
    let mut root_prefix: Option<String> = None;
    {
        for i in 0..archive.len() {
            if let Ok(entry) = archive.by_index(i) {
                let name = entry.name().replace('\\', "/");
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
        if root_prefix.is_some() { "wrapped" } else { "flat" },
        root_prefix
    );

    let total = archive.len();
    let instance_dir = instances_dir.join(&instance_name);
    fs::create_dir_all(&instance_dir)
        .map_err(|e| format!("인스턴스 디렉토리 생성 실패 {}: {}", instance_dir.display(), e))?;

    for i in 0..total {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {}", e))?;

        let raw_name = entry.name().replace('\\', "/");
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

        // 경로 탐색 공격 차단: ".." 또는 절대경로 포함 시 건너뜀
        if relative_name.contains("..") || Path::new(&relative_name).is_absolute() {
            log::warn!("위험한 zip 엔트리 차단: {}", relative_name);
            on_progress(i + 1, total);
            continue;
        }

        let target = instance_dir.join(&relative_name);

        if entry.is_dir() || relative_name.ends_with('/') {
            fs::create_dir_all(&target)
                .map_err(|e| format!("디렉토리 생성 실패 {}: {}", target.display(), e))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("디렉토리 생성 실패 {}: {}", parent.display(), e))?;
            }
            let mut outfile = fs::File::create(&target)
                .map_err(|e| format!("파일 생성 실패 {}: {}", target.display(), e))?;
            io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }

        on_progress(i + 1, total);
    }

    // instance.cfg 의 name= 값 업데이트
    let cfg_path = instance_dir.join("instance.cfg");
    if cfg_path.exists() {
        if let Ok(content) = fs::read_to_string(&cfg_path) {
            let eol = if content.contains("\r\n") { "\r\n" } else { "\n" };
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
            if let Err(e) = fs::write(&cfg_path, updated) {
                log::warn!("instance.cfg 업데이트 실패: {}", e);
            }
        }
    }

    log::info!("인스턴스 '{}' 으로 압축 해제 완료", instance_name);
    Ok(())
}

/// PrismLauncher 프로세스 상태를 확인하고 PID를 반환
fn get_pid_by_path(exe_path: &str) -> Option<u32> {
    #[cfg(target_os = "windows")]
    {
        let normalized = exe_path.replace('/', "\\");
        let output = silent_command(Command::new("powershell").args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-Process | Where-Object {{ $_.Path -eq '{}' }} | Select-Object -First 1 -ExpandProperty Id",
                normalized
            ),
        ]))
        .output()
        .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim().parse::<u32>().ok()
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("pgrep")
            .args(["-f", exe_path])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().next()?.trim().parse::<u32>().ok()
    }
}

/// 해당 PID의 자식 프로세스 중 java가 있는지 확인
fn has_java_child(parent_pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        let output = silent_command(Command::new("powershell").args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process | Where-Object {{ $_.ParentProcessId -eq {} -and $_.Name -match 'java' }} | Select-Object -First 1 ProcessId",
                parent_pid
            ),
        ]))
        .output();

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout.lines().any(|line| line.trim().parse::<u32>().is_ok())
            }
            Err(_) => false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("pgrep")
            .args(["-P", &parent_pid.to_string(), "java"])
            .output();

        match output {
            Ok(o) => !o.stdout.is_empty(),
            Err(_) => false,
        }
    }
}

/// PrismLauncher 종료. 성공하면 true, 게임 실행 중이면 false.
pub async fn kill_prism(exe_path: &str) -> bool {
    match get_pid_by_path(exe_path) {
        Some(pid) => {
            if has_java_child(pid) {
                log::info!(
                    "PrismLauncher(PID:{}) 게임 실행 중 — 대기 (녹화 보호)",
                    pid
                );
                false
            } else {
                log::info!("PrismLauncher(PID:{}) 게임 미실행 — 종료", pid);
                kill_process(pid);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                true
            }
        }
        None => true, // 이미 꺼져있음
    }
}

/// PrismLauncher 시작
pub fn start_prism(exe_path: &str) {
    Command::new(exe_path).spawn().ok();
    log::info!("PrismLauncher 시작됨: {}", exe_path);
}

/// PrismLauncher 새로고침 시도. 성공하면 true, java 대기 중이면 false.
pub async fn try_refresh(exe_path: &str) -> bool {
    if kill_prism(exe_path).await {
        start_prism(exe_path);
        true
    } else {
        false
    }
}

fn kill_process(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        silent_command(Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]))
            .output()
            .ok();
    }

    #[cfg(not(target_os = "windows"))]
    {
        Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
            .ok();
    }
}
