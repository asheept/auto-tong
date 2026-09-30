use serde::Serialize;
use tauri_plugin_updater::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFailureKind {
    Network,
    Signature,
    Permission,
    Unknown,
}

impl UpdateFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Signature => "signature",
            Self::Permission => "permission",
            Self::Unknown => "unknown",
        }
    }

    pub fn guidance(self) -> &'static str {
        match self {
            Self::Network => "네트워크 연결을 확인한 뒤 다시 시도하세요.",
            Self::Signature => "업데이트 파일 검증에 실패했습니다. 공식 릴리스 상태를 확인하세요.",
            Self::Permission => "앱을 종료한 뒤 설치 권한과 파일 잠금을 확인하세요.",
            Self::Unknown => "업데이트 로그와 공식 릴리스를 확인한 뒤 다시 시도하세요.",
        }
    }
}

pub fn classify(error: &Error) -> UpdateFailureKind {
    match error {
        Error::Network(_) | Error::Reqwest(_) => UpdateFailureKind::Network,
        Error::Minisign(_) | Error::Base64(_) | Error::SignatureUtf8(_) => {
            UpdateFailureKind::Signature
        }
        Error::AuthenticationFailed => UpdateFailureKind::Permission,
        Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
            UpdateFailureKind::Permission
        }
        _ => UpdateFailureKind::Unknown,
    }
}

pub fn is_newer_version(current: &str, remote: &str) -> Result<bool, String> {
    let current = semver::Version::parse(current.trim_start_matches('v'))
        .map_err(|e| format!("현재 버전 형식 오류: {e}"))?;
    let remote = semver::Version::parse(remote.trim_start_matches('v'))
        .map_err(|e| format!("원격 버전 형식 오류: {e}"))?;
    Ok(remote.cmp_precedence(&current).is_gt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_and_build_metadata_follow_semver_order() {
        assert!(is_newer_version("1.0.0-beta.2", "1.0.0-beta.10").unwrap());
        assert!(is_newer_version("1.0.0-rc.1", "1.0.0").unwrap());
        assert!(!is_newer_version("1.0.0", "1.0.0-rc.1").unwrap());
        assert!(!is_newer_version("1.0.0", "1.0.0+build.2").unwrap());
        assert!(is_newer_version("v0.2.9", "v0.2.10").unwrap());
        assert!(is_newer_version("1.0.0", "garbage").is_err());
    }

    #[test]
    fn update_errors_have_specific_actions() {
        let cases = [
            (Error::Network("offline".into()), UpdateFailureKind::Network),
            (
                Error::SignatureUtf8("bad".into()),
                UpdateFailureKind::Signature,
            ),
            (
                Error::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                UpdateFailureKind::Permission,
            ),
            (Error::AuthenticationFailed, UpdateFailureKind::Permission),
            (Error::PackageInstallFailed, UpdateFailureKind::Unknown),
        ];
        for (error, expected) in cases {
            let actual = classify(&error);
            assert_eq!(actual, expected);
            assert!(!actual.guidance().is_empty());
        }
    }
}
