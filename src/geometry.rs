use serde::{Deserialize, Serialize};

use crate::spec::GeometryMode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinate {
    pub lon: f64,
    pub lat: f64,
}

impl From<[f64; 2]> for Coordinate {
    fn from(value: [f64; 2]) -> Self {
        Self::new(value[0], value[1])
    }
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Geometry {
    Point {
        coordinates: [f64; 2],
    },
    LineString {
        coordinates: Vec<[f64; 2]>,
    },
    Polygon {
        coordinates: Vec<Vec<[f64; 2]>>,
    },
    MultiPolygon {
        coordinates: Vec<Vec<Vec<[f64; 2]>>>,
    },
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

pub fn polygon_or_multipolygon(polygons: Vec<Vec<Vec<Coordinate>>>) -> Option<Geometry> {
    let mut output: Vec<Vec<Vec<[f64; 2]>>> = polygons
        .into_iter()
        .map(|polygon| {
            polygon
                .into_iter()
                .map(|ring| ring.into_iter().map(Coordinate::as_array).collect())
                .collect()
        })
        .collect();

    match output.len() {
        0 => None,
        1 => Some(Geometry::Polygon {
            coordinates: output.remove(0),
        }),
        _ => Some(Geometry::MultiPolygon {
            coordinates: output,
        }),
    }
}

pub fn ring_area(ring: &[Coordinate]) -> f64 {
    if ring.len() < 4 {
        return 0.0;
    }
    ring.windows(2)
        .map(|window| {
            let a = window[0];
            let b = window[1];
            (a.lon * b.lat) - (b.lon * a.lat)
        })
        .sum::<f64>()
        / 2.0
}

pub fn normalize_ring_orientation(ring: &mut [Coordinate], counter_clockwise: bool) {
    let is_counter_clockwise = ring_area(ring) > 0.0;
    if is_counter_clockwise != counter_clockwise {
        ring.reverse();
    }
}

pub fn point_in_ring(point: Coordinate, ring: &[Coordinate]) -> bool {
    if ring.len() < 4 {
        return false;
    }

    let mut inside = false;
    let mut previous = ring[ring.len() - 1];
    for current in ring.iter().copied() {
        let intersects = ((current.lat > point.lat) != (previous.lat > point.lat))
            && (point.lon
                < (previous.lon - current.lon) * (point.lat - current.lat)
                    / (previous.lat - current.lat)
                    + current.lon);
        if intersects {
            inside = !inside;
        }
        previous = current;
    }
    inside
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

    #[test]
    fn normalizes_ring_orientation() {
        let mut ring = vec![
            Coordinate::new(0.0, 0.0),
            Coordinate::new(0.0, 1.0),
            Coordinate::new(1.0, 1.0),
            Coordinate::new(1.0, 0.0),
            Coordinate::new(0.0, 0.0),
        ];
        normalize_ring_orientation(&mut ring, true);
        assert!(ring_area(&ring) > 0.0);
        normalize_ring_orientation(&mut ring, false);
        assert!(ring_area(&ring) < 0.0);
    }

    #[test]
    fn point_in_ring_detects_inside_and_outside() {
        let ring = vec![
            Coordinate::new(0.0, 0.0),
            Coordinate::new(1.0, 0.0),
            Coordinate::new(1.0, 1.0),
            Coordinate::new(0.0, 1.0),
            Coordinate::new(0.0, 0.0),
        ];
        assert!(point_in_ring(Coordinate::new(0.5, 0.5), &ring));
        assert!(!point_in_ring(Coordinate::new(2.0, 0.5), &ring));
    }
}
