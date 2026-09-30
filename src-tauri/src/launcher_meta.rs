/// PrismLauncher metadata component IDs.
/// Source: https://github.com/PrismLauncher/meta-launcher/tree/master
pub fn loader_uid(loader: &str) -> Option<&'static str> {
    match loader {
        "forge" => Some("net.minecraftforge"),
        "fabric-loader" => Some("net.fabricmc.fabric-loader"),
        "neoforge" => Some("net.neoforged"),
        "quilt-loader" => Some("org.quiltmc.quilt-loader"),
        _ => None,
    }
}

pub fn loader_component(loader: &str, version: &str) -> Option<serde_json::Value> {
    let (name, uid) = match loader {
        "forge" => ("Forge", loader_uid(loader)?),
        "fabric-loader" => ("Fabric Loader", loader_uid(loader)?),
        "neoforge" => ("NeoForge", loader_uid(loader)?),
        "quilt-loader" => ("Quilt Loader", loader_uid(loader)?),
        _ => return None,
    };
    Some(serde_json::json!({
        "cachedName": name,
        "cachedVersion": version,
        "uid": uid,
        "version": version
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_prism_loader_ids() {
        for (key, uid) in [
            ("forge", "net.minecraftforge"),
            ("fabric-loader", "net.fabricmc.fabric-loader"),
            ("neoforge", "net.neoforged"),
            ("quilt-loader", "org.quiltmc.quilt-loader"),
        ] {
            assert_eq!(loader_component(key, "1.0").unwrap()["uid"], uid);
        }
        assert!(loader_component("unknown", "1.0").is_none());
    }
}
