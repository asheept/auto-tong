use std::path::Path;
use std::time::Duration;

fn parse_java_info(output: &str, required_major: u32) -> Result<(), String> {
    let version = output
        .lines()
        .find_map(|line| {
            let (_, rest) = line.split_once("version \"")?;
            rest.split('"').next()
        })
        .ok_or("Java 버전 출력이 없습니다")?;
    let mut numbers = version.split('.');
    let first: u32 = numbers
        .next()
        .unwrap_or("")
        .parse()
        .map_err(|_| "Java 버전을 해석할 수 없습니다")?;
    let major = if first == 1 {
        numbers
            .next()
            .unwrap_or("")
            .parse()
            .map_err(|_| "Java 8 버전을 해석할 수 없습니다")?
    } else {
        first
    };
    if major != required_major {
        return Err(format!(
            "Java 버전 불일치: 필요 {}, 실제 {}",
            required_major, major
        ));
    }
    let arch = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("os.arch ="))
        .map(str::trim)
        .ok_or("Java 아키텍처 출력이 없습니다")?;
    let expected = match std::env::consts::ARCH {
        "x86_64" => &["amd64", "x86_64"][..],
        "aarch64" => &["aarch64", "arm64"][..],
        _ => return Err("지원하지 않는 Java 아키텍처입니다".to_string()),
    };
    if !expected
        .iter()
        .any(|candidate| arch.eq_ignore_ascii_case(candidate))
    {
        return Err(format!("Java 아키텍처 불일치: {arch}"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn check_pe_architecture(path: &Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Java 실행 파일 열기 실패: {e}"))?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header)
        .map_err(|e| format!("Java PE 헤더 읽기 실패: {e}"))?;
    if &header[..2] != b"MZ" {
        return Err("Java 실행 파일이 Windows PE 형식이 아닙니다".to_string());
    }
    let offset = u32::from_le_bytes(header[60..64].try_into().unwrap()) as u64;
    if offset > 1024 * 1024 {
        return Err("Java PE 헤더 위치가 유효하지 않습니다".to_string());
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut pe = [0u8; 6];
    file.read_exact(&mut pe)
        .map_err(|e| format!("Java PE 서명 읽기 실패: {e}"))?;
    if &pe[..4] != b"PE\0\0" {
        return Err("Java PE 서명이 유효하지 않습니다".to_string());
    }
    let machine = u16::from_le_bytes([pe[4], pe[5]]);
    let expected = match std::env::consts::ARCH {
        "x86_64" => 0x8664,
        "aarch64" => 0xaa64,
        _ => return Err("지원하지 않는 Java 아키텍처입니다".to_string()),
    };
    if machine != expected {
        return Err("Java 실행 파일 아키텍처가 앱과 다릅니다".to_string());
    }
    Ok(())
}

pub async fn validate_runtime(configured_binary: &Path, required_major: u32) -> Result<(), String> {
    let bin_dir = configured_binary
        .parent()
        .ok_or("Java bin 폴더가 없습니다")?;
    #[cfg(target_os = "windows")]
    let console_binary = bin_dir.join("java.exe");
    #[cfg(not(target_os = "windows"))]
    let console_binary = bin_dir.join("java");

    if !configured_binary.is_file() || !console_binary.is_file() {
        return Err("Java 실행 파일이 누락되었습니다".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        check_pe_architecture(configured_binary)?;
        check_pe_architecture(&console_binary)?;
    }
    let output = tokio::time::timeout(
        Duration::from_secs(8),
        tokio::process::Command::new(&console_binary)
            .args(["-XshowSettings:properties", "-version"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Java 버전 확인 시간 초과".to_string())?
    .map_err(|e| format!("Java 실행 실패: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Java 버전 확인 실패: 종료 코드 {:?}",
            output.status.code()
        ));
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_java_info(&text, required_major)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_version_and_architecture_output() {
        let arch = if std::env::consts::ARCH == "aarch64" {
            "aarch64"
        } else {
            "amd64"
        };
        assert!(
            parse_java_info(&format!("openjdk version \"25.0.1\"\nos.arch = {arch}"), 25).is_ok()
        );
        assert!(
            parse_java_info(&format!("openjdk version \"17.0.1\"\nos.arch = {arch}"), 21).is_err()
        );
        assert!(parse_java_info("openjdk version \"21.0.1\"\nos.arch = x86", 21).is_err());
        assert!(
            parse_java_info(&format!("java version \"1.8.0_401\"\nos.arch = {arch}"), 8).is_ok()
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn fake_javaw_and_missing_runtime_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let javaw = bin.join("javaw.exe");
        assert!(validate_runtime(&javaw, 21).await.is_err());
        std::fs::write(&javaw, b"fake javaw").unwrap();
        std::fs::write(bin.join("java.exe"), b"fake java").unwrap();
        assert!(validate_runtime(&javaw, 21).await.is_err());
    }
}
