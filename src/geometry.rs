use serde::Serialize;

use crate::spec::GeometryMode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinate {
    pub lon: f64,
    pub lat: f64,
}

impl Coordinate {
    pub fn new(lon: f64, lat: f64) -> Self {
        Self { lon, lat }
    }

    pub fn as_array(self) -> [f64; 2] {
        [self.lon, self.lat]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BBox {
    pub fn new(values: [f64; 4]) -> Self {
        Self {
            min_lon: values[0],
            min_lat: values[1],
            max_lon: values[2],
            max_lat: values[3],
        }
    }

    pub fn contains(self, coordinate: Coordinate) -> bool {
        coordinate.lon >= self.min_lon
            && coordinate.lon <= self.max_lon
            && coordinate.lat >= self.min_lat
            && coordinate.lat <= self.max_lat
    }

    pub fn intersects_any(self, coordinates: &[Coordinate]) -> bool {
        coordinates
            .iter()
            .copied()
            .any(|coordinate| self.contains(coordinate))
    }

    pub fn validate(self) -> std::result::Result<(), String> {
        if self.min_lon < -180.0 || self.max_lon > 180.0 {
            return Err("bbox longitude values must be between -180 and 180".to_owned());
        }
        if self.min_lat < -90.0 || self.max_lat > 90.0 {
            return Err("bbox latitude values must be between -90 and 90".to_owned());
        }
        if self.min_lon > self.max_lon {
            return Err("bbox min_lon must be <= max_lon".to_owned());
        }
        if self.min_lat > self.max_lat {
            return Err("bbox min_lat must be <= max_lat".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum Geometry {
    Point { coordinates: [f64; 2] },
    LineString { coordinates: Vec<[f64; 2]> },
    Polygon { coordinates: Vec<Vec<[f64; 2]>> },
}

pub fn point(coordinate: Coordinate) -> Geometry {
    Geometry::Point {
        coordinates: coordinate.as_array(),
    }
}

pub fn way_geometry(
    coordinates: &[Coordinate],
    is_closed: bool,
    geometry_mode: GeometryMode,
) -> Geometry {
    let line: Vec<[f64; 2]> = coordinates
        .iter()
        .copied()
        .map(Coordinate::as_array)
        .collect();

    if geometry_mode == GeometryMode::Polygon && is_closed && line.len() >= 4 {
        Geometry::Polygon {
            coordinates: vec![line],
        }
    } else {
        Geometry::LineString { coordinates: line }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_contains_coordinate() {
        let bbox = BBox::new([8.5, 48.8, 9.3, 49.2]);
        assert!(bbox.contains(Coordinate::new(8.7, 48.9)));
        assert!(!bbox.contains(Coordinate::new(10.0, 48.9)));
    }
}
