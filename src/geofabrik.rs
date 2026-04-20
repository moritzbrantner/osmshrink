use regex::Regex;

use crate::error::{OsmshrinkError, Result};

const GEOFABRIK_PREFIX: &str = "geofabrik:";
const GEOFABRIK_BASE_URL: &str = "https://download.geofabrik.de";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeofabrikSource {
    pub region: String,
    pub url: String,
}

pub fn resolve_source(input: &str) -> Result<String> {
    if input.starts_with("http://") || input.starts_with("https://") {
        return Ok(input.to_owned());
    }

    if input.starts_with(GEOFABRIK_PREFIX) {
        return Ok(parse_geofabrik(input)?.url);
    }

    Err(OsmshrinkError::UnsupportedSource(input.to_owned()))
}

pub fn parse_geofabrik(input: &str) -> Result<GeofabrikSource> {
    let Some(region) = input.strip_prefix(GEOFABRIK_PREFIX) else {
        return Err(OsmshrinkError::InvalidGeofabrikSource(input.to_owned()));
    };

    validate_region(region)?;
    Ok(GeofabrikSource {
        region: region.to_owned(),
        url: format!("{GEOFABRIK_BASE_URL}/{region}-latest.osm.pbf"),
    })
}

pub fn geofabrik_url_for_region(region: &str) -> Result<String> {
    validate_region(region)?;
    Ok(format!("{GEOFABRIK_BASE_URL}/{region}-latest.osm.pbf"))
}

fn validate_region(region: &str) -> Result<()> {
    if region.is_empty()
        || region.starts_with('/')
        || region.ends_with('/')
        || region.contains("//")
        || region.split('/').any(|part| part == "." || part == "..")
    {
        return Err(OsmshrinkError::InvalidGeofabrikRegion(region.to_owned()));
    }

    let valid = Regex::new(r"^[a-z0-9][a-z0-9/_-]*[a-z0-9]$").expect("valid regex");
    if !valid.is_match(region) {
        return Err(OsmshrinkError::InvalidGeofabrikRegion(region.to_owned()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_geofabrik_shorthand() {
        let source = parse_geofabrik("geofabrik:europe/germany/baden-wuerttemberg").unwrap();
        assert_eq!(source.region, "europe/germany/baden-wuerttemberg");
        assert_eq!(
            source.url,
            "https://download.geofabrik.de/europe/germany/baden-wuerttemberg-latest.osm.pbf"
        );
    }

    #[test]
    fn rejects_invalid_geofabrik_shorthand() {
        assert!(parse_geofabrik("geofabrik:../planet").is_err());
        assert!(parse_geofabrik("geofabrik:europe//germany").is_err());
        assert!(parse_geofabrik("geofabrik:Europe/Germany").is_err());
    }
}
