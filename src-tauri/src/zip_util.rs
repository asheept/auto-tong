//! ZIP 엔트리 이름 디코딩 유틸리티
//!
//! ZIP 스펙(범용 비트 11, EFS)은 파일명이 UTF-8인지 여부를 표시하지만,
//! 한국어 Windows 탐색기로 압축한 zip은 이 비트를 세팅하지 않고
//! 파일명을 CP949(EUC-KR) 바이트로 저장한다.
//!
//! `zip` crate의 기본 동작은 비트가 없을 때 CP437로 해석하여
//! 한글 파일명을 mojibake로 만든다. 이 모듈은 다음 순서로 디코딩을 시도한다:
//!
//! 1. raw 바이트가 유효한 UTF-8이면 그대로 사용 (EFS 비트 세팅 케이스 포함)
//! 2. CP949(EUC-KR)로 디코딩 시도
//! 3. 실패 시 `zip` crate 기본 해석(fallback) 사용

#[derive(Clone, Copy)]
pub struct ZipLimits {
    pub entries: usize,
    pub file_bytes: u64,
    pub total_bytes: u64,
}

pub const DEFAULT_ZIP_LIMITS: ZipLimits = ZipLimits {
    entries: 20_000,
    file_bytes: 512 * 1024 * 1024,
    total_bytes: 4 * 1024 * 1024 * 1024,
};

/// 대상 폴더를 만들기 전에 ZIP의 선언된 해제 크기를 검사한다.
pub fn check_archive_limits(
    archive: &mut zip::ZipArchive<std::fs::File>,
    limits: ZipLimits,
) -> Result<(), String> {
    if archive.len() > limits.entries {
        return Err(format!(
            "ZIP 엔트리 수 제한 초과: {} > {}",
            archive.len(),
            limits.entries
        ));
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("ZIP 엔트리 읽기 실패: {e}"))?;
        if entry.size() > limits.file_bytes {
            return Err(format!(
                "ZIP 단일 파일 해제 크기 제한 초과: {}",
                entry.name()
            ));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| "ZIP 총 해제 크기 계산 범위 초과".to_string())?;
        if total > limits.total_bytes {
            return Err("ZIP 총 해제 크기 제한 초과".to_string());
        }
    }
    Ok(())
}

/// ZIP 엔트리의 raw 파일명 바이트를 안전하게 UTF-8 문자열로 디코딩한다.
///
/// - `raw`: `entry.name_raw()` 결과
/// - `fallback`: `entry.name()` 결과 (최종 폴백)
///
/// 반환값의 경로 구분자는 `/`로 정규화되어 있다.
pub fn decode_zip_name(raw: &[u8], fallback: &str) -> String {
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.replace('\\', "/");
    }

    let (decoded, _, had_errors) = encoding_rs::EUC_KR.decode(raw);
    if !had_errors {
        log::debug!("zip 엔트리 이름을 CP949로 재해석: {:?}", decoded);
        return decoded.replace('\\', "/");
    }

    log::warn!(
        "zip 엔트리 이름 디코딩 실패 (UTF-8/CP949 모두) — 기본 폴백 사용: {}",
        fallback
    );
    fallback.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_name_passes_through() {
        assert_eq!(
            decode_zip_name(b"mods/fabric.jar", "mods/fabric.jar"),
            "mods/fabric.jar"
        );
    }

    #[test]
    fn utf8_korean_passes_through() {
        let utf8 = "saves/한국어월드/level.dat";
        assert_eq!(decode_zip_name(utf8.as_bytes(), utf8), utf8);
    }

    #[test]
    fn cp949_korean_recovers() {
        let cp949_bytes: Vec<u8> = encoding_rs::EUC_KR
            .encode("resourcepacks/한글팩.zip")
            .0
            .into_owned();
        let fallback = "garbage";
        assert_eq!(
            decode_zip_name(&cp949_bytes, fallback),
            "resourcepacks/한글팩.zip"
        );
    }

    #[test]
    fn backslash_normalized_to_forward_slash() {
        assert_eq!(decode_zip_name(b"a\\b\\c.txt", "a\\b\\c.txt"), "a/b/c.txt");
    }
}
