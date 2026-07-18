//! Explicit, HTTPS-only release manifest checks with no download or installation behavior.

use std::{cmp::Ordering, env, io};

const ENDPOINT_ENV: &str = "FLASH_SHOT_UPDATE_ENDPOINT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateConfig {
    endpoint: String,
}

impl UpdateConfig {
    pub fn from_environment() -> io::Result<Option<Self>> {
        let Some(endpoint) = env::var(ENDPOINT_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let endpoint = endpoint.trim();
        validate_endpoint(endpoint)?;
        Ok(Some(Self {
            endpoint: endpoint.to_owned(),
        }))
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateAvailability {
    Available { version: String },
    Current { version: String },
    NewerLocal { version: String },
}

/// Fetches and validates a release manifest only after the user explicitly asks to check.
pub fn check(config: &UpdateConfig, current_version: &str) -> io::Result<UpdateAvailability> {
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let response = agent
        .get(config.endpoint())
        .set("accept", "application/json")
        .call()
        .map_err(update_error)?;
    let manifest = response.into_json().map_err(update_error)?;
    let version = release_version_from_manifest(manifest)?;
    match compare_versions(&version, current_version)? {
        Ordering::Greater => Ok(UpdateAvailability::Available { version }),
        Ordering::Equal => Ok(UpdateAvailability::Current { version }),
        Ordering::Less => Ok(UpdateAvailability::NewerLocal { version }),
    }
}

fn validate_endpoint(endpoint: &str) -> io::Result<()> {
    if !endpoint.starts_with("https://") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "update endpoint must use HTTPS",
        ));
    }
    Ok(())
}

fn release_version_from_manifest(value: serde_json::Value) -> io::Result<String> {
    let schema_version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64);
    if schema_version != Some(1) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "update manifest has an unsupported schema version",
        ));
    }
    if value.get("platform").and_then(serde_json::Value::as_str) != Some("windows") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "update manifest is not for Windows",
        ));
    }
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "update manifest has no version")
        })?;
    parse_version(version)?;
    let asset_prefix = format!("FlashShot-{version}-windows-");
    let assets = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .filter(|assets| !assets.is_empty())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "update manifest has no assets")
        })?;
    if assets.iter().any(|asset| {
        !asset
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| {
                name.starts_with(&asset_prefix)
                    && (name.ends_with(".zip") || name.ends_with(".exe"))
            })
            || asset
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|hash| {
                    hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            || asset
                .get("size_bytes")
                .and_then(serde_json::Value::as_u64)
                .is_none_or(|size| size == 0)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "update manifest contains an invalid asset",
        ));
    }
    Ok(version.to_owned())
}

fn compare_versions(left: &str, right: &str) -> io::Result<Ordering> {
    let left = parse_version(left)?;
    let right = parse_version(right)?;
    Ok(left.cmp(&right))
}

fn parse_version(version: &str) -> io::Result<[u64; 3]> {
    let mut values = version.split('.').map(|part| part.parse::<u64>());
    let parsed = [
        parse_version_part(values.next())?,
        parse_version_part(values.next())?,
        parse_version_part(values.next())?,
    ];
    if values.next().is_some() || parsed.iter().any(Option::is_none) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "version must use major.minor.patch numeric format",
        ));
    }
    Ok(parsed.map(Option::unwrap))
}

fn parse_version_part(
    value: Option<Result<u64, std::num::ParseIntError>>,
) -> io::Result<Option<u64>> {
    value.transpose().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "version must use numeric components",
        )
    })
}

fn update_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("update manifest request failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        UpdateAvailability, compare_versions, release_version_from_manifest, validate_endpoint,
    };
    use std::cmp::Ordering;

    fn manifest(version: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "product": "Flash Shot",
            "version": version,
            "platform": "windows",
            "assets": [{
                "name": "FlashShot-1.2.3-windows-x86_64.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "size_bytes": 12
            }]
        })
    }

    #[test]
    fn update_configuration_requires_https() {
        assert!(validate_endpoint("https://releases.example/manifest.json").is_ok());
        assert!(validate_endpoint("http://releases.example/manifest.json").is_err());
    }

    #[test]
    fn release_manifest_requires_versioned_windows_assets() {
        assert_eq!(
            release_version_from_manifest(manifest("1.2.3")).unwrap(),
            "1.2.3"
        );
        assert!(release_version_from_manifest(manifest("1.2")).is_err());
        let mut invalid = manifest("1.2.3");
        invalid["assets"] = serde_json::json!([]);
        assert!(release_version_from_manifest(invalid).is_err());
        let mut mismatched_asset = manifest("1.2.3");
        mismatched_asset["assets"][0]["name"] =
            serde_json::json!("FlashShot-1.2.2-windows-x86_64.zip");
        assert!(release_version_from_manifest(mismatched_asset).is_err());
    }

    #[test]
    fn release_versions_are_compared_numerically() {
        assert_eq!(
            compare_versions("1.10.0", "1.2.9").unwrap(),
            Ordering::Greater
        );
        assert_eq!(compare_versions("1.2.3", "1.2.3").unwrap(), Ordering::Equal);
        assert_eq!(compare_versions("1.2.3", "2.0.0").unwrap(), Ordering::Less);
        assert!(compare_versions("1.2", "1.2.0").is_err());
    }

    #[test]
    fn availability_states_are_explicit() {
        assert_eq!(
            UpdateAvailability::Available {
                version: "1.1.0".to_owned()
            },
            UpdateAvailability::Available {
                version: "1.1.0".to_owned()
            }
        );
    }
}
