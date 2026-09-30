use sha2::{Digest, Sha256};
use std::time::Duration;

const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 512 * 1024 * 1024;

pub struct JavaPackage {
    pub url: reqwest::Url,
    pub checksum: String,
}

pub fn parse_package(raw: &[u8], os: &str, arch: &str) -> Result<JavaPackage, String> {
    let assets: serde_json::Value = serde_json::from_slice(raw)
        .map_err(|e| format!("Adoptium metadata parsing failed: {e}"))?;
    let binary = assets
        .as_array()
        .and_then(|items| items.first())
        .and_then(|asset| asset.get("binary"))
        .ok_or("Adoptium metadata has no binary")?;
    if binary["os"] != os || binary["architecture"] != arch || binary["image_type"] != "jre" {
        return Err(
            "Adoptium package does not match the required OS, architecture or JRE type".into(),
        );
    }
    let package = &binary["package"];
    let link = package["link"]
        .as_str()
        .ok_or("Adoptium package link is missing")?;
    let url =
        reqwest::Url::parse(link).map_err(|e| format!("Invalid Adoptium package URL: {e}"))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err("Adoptium package must use HTTPS".into());
    }
    let checksum = package["checksum"]
        .as_str()
        .ok_or("Adoptium checksum is missing")?;
    if checksum.len() != 64 || !checksum.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Adoptium SHA-256 checksum is invalid".into());
    }
    Ok(JavaPackage {
        url,
        checksum: checksum.to_ascii_lowercase(),
    })
}

pub async fn fetch_package(
    client: &reqwest::Client,
    major: u32,
    os: &str,
    arch: &str,
) -> Result<JavaPackage, String> {
    let url = format!("https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type=jre&os={os}&vendor=eclipse");
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Adoptium metadata request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Adoptium metadata response failed: {e}"))?;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Adoptium metadata read failed: {e}"))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_METADATA_BYTES {
            return Err("Adoptium metadata exceeds size limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    parse_package(&body, os, arch)
}

pub async fn download_verified(
    client: &reqwest::Client,
    package: &JavaPackage,
) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(package.url.clone())
        .send()
        .await
        .map_err(|e| format!("Java download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Java download failed: {e}"))?;
    let mut archive = Vec::new();
    let mut digest = Sha256::new();
    let download = async {
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("Java download interrupted: {e}"))?
        {
            if archive.len().saturating_add(chunk.len()) > MAX_ARCHIVE_BYTES {
                return Err("Java archive exceeds size limit".into());
            }
            digest.update(&chunk);
            archive.extend_from_slice(&chunk);
        }
        Ok::<(), String>(())
    };
    tokio::time::timeout(Duration::from_secs(300), download)
        .await
        .map_err(|_| "Java download timed out".to_string())??;
    let actual = format!("{:x}", digest.finalize());
    if actual != package.checksum {
        return Err("Java archive SHA-256 checksum mismatch".into());
    }
    Ok(archive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn fixture_server(response: &'static [u8]) -> (reqwest::Url, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = reqwest::Url::parse(&format!(
            "http://{}/java.zip",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            stream.write_all(response).unwrap();
        });
        (url, server)
    }

    #[test]
    fn package_metadata_requires_matching_platform_and_digest() {
        let raw = br#"[{"binary":{"os":"windows","architecture":"x64","image_type":"jre","package":{"link":"https://example.com/java.zip","checksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}]"#;
        assert!(parse_package(raw, "windows", "x64").is_ok());
        assert!(parse_package(raw, "windows", "aarch64").is_err());
        let invalid_digest = String::from_utf8(raw.to_vec()).unwrap().replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
        );
        assert!(parse_package(invalid_digest.as_bytes(), "windows", "x64").is_err());
        let insecure = String::from_utf8(raw.to_vec())
            .unwrap()
            .replace("https://", "http://");
        assert!(parse_package(insecure.as_bytes(), "windows", "x64").is_err());
    }

    #[tokio::test]
    async fn checksum_mismatch_and_interrupted_download_are_rejected() {
        let client = reqwest::Client::new();
        for response in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nshort"[..],
        ] {
            let (url, server) = fixture_server(response);
            let package = JavaPackage {
                url,
                checksum: format!("{:x}", Sha256::digest(b"good")),
            };
            assert!(download_verified(&client, &package).await.is_err());
            server.join().unwrap();
        }
    }
}
