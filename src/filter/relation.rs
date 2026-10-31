use std::collections::HashMap;

use osmpbfreader::{RelationId, Tags, WayId};

use crate::geometry::{
    Coordinate, Geometry, normalize_ring_orientation, point_in_ring, polygon_or_multipolygon,
    ring_area,
};
use crate::model::Tags as NormalizedTags;

#[derive(Debug, Clone)]
pub(super) struct CandidateRelation {
    pub(super) id: RelationId,
    pub(super) tags: NormalizedTags,
    pub(super) members: Vec<RelationWayMember>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RelationWayMember {
    pub(super) way_id: WayId,
    pub(super) role: RelationMemberRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelationMemberRole {
    Outer,
    Inner,
}

#[derive(Debug)]
pub(super) enum RelationAssemblyResult {
    Geometry(Geometry),
    MissingMembers,
    InvalidRings,
}

pub(super) fn is_area_relation_osm(tags: &Tags) -> bool {
    matches!(
        tags.iter()
            .find_map(|(key, value)| (key.as_str() == "type").then_some(value.as_str())),
        Some("multipolygon" | "boundary")
    )
}

pub(super) fn assemble_relation(
    candidate: &CandidateRelation,
    relation_way_geometries: &HashMap<WayId, Vec<Coordinate>>,
) -> RelationAssemblyResult {
    let mut outer_segments = Vec::new();
    let mut inner_segments = Vec::new();
    for member in &candidate.members {
        let Some(coordinates) = relation_way_geometries.get(&member.way_id) else {
            return RelationAssemblyResult::MissingMembers;
        };
        match member.role {
            RelationMemberRole::Outer => outer_segments.push(coordinates.clone()),
            RelationMemberRole::Inner => inner_segments.push(coordinates.clone()),
        }
    }

    let Some(mut outer_rings) = stitch_rings(outer_segments) else {
        return RelationAssemblyResult::InvalidRings;
    };
    let Some(mut inner_rings) = stitch_rings(inner_segments) else {
        return RelationAssemblyResult::InvalidRings;
    };

    if outer_rings.is_empty() {
        return RelationAssemblyResult::InvalidRings;
    }

    for ring in &mut outer_rings {
        normalize_ring_orientation(ring, true);
    }
    for ring in &mut inner_rings {
        normalize_ring_orientation(ring, false);
    }

    let mut polygons: Vec<Vec<Vec<Coordinate>>> =
        outer_rings.into_iter().map(|outer| vec![outer]).collect();

    for inner in inner_rings {
        let Some(point) = inner.first().copied() else {
            return RelationAssemblyResult::InvalidRings;
        };
        let Some((target_index, _)) = polygons
            .iter()
            .enumerate()
            .filter_map(|(index, polygon)| {
                let outer = &polygon[0];
                if point_in_ring(point, outer) {
                    Some((index, ring_area(outer).abs()))
                } else {
                    None
                }
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
        else {
            return RelationAssemblyResult::InvalidRings;
        };
        polygons[target_index].push(inner);
    }

    match polygon_or_multipolygon(polygons) {
        Some(geometry) => RelationAssemblyResult::Geometry(geometry),
        None => RelationAssemblyResult::InvalidRings,
    }
}

fn stitch_rings(mut segments: Vec<Vec<Coordinate>>) -> Option<Vec<Vec<Coordinate>>> {
    let mut rings = Vec::new();

    while !segments.is_empty() {
        let mut ring = segments.remove(0);
        if ring.len() < 2 {
            return None;
        }

        loop {
            if is_valid_closed_ring(&ring) {
                rings.push(ring);
                break;
            }

            let (index, action) = find_connecting_segment(&ring, &segments)?;
            let segment = segments.remove(index);
            apply_segment(&mut ring, segment, action);
        }
    }

    Some(rings)
}

fn is_valid_closed_ring(ring: &[Coordinate]) -> bool {
    ring.len() >= 4 && ring.first() == ring.last()
}

#[derive(Debug, Clone, Copy)]
enum StitchAction {
    AppendForward,
    AppendReverse,
    PrependForward,
    PrependReverse,
}

fn find_connecting_segment(
    ring: &[Coordinate],
    segments: &[Vec<Coordinate>],
) -> Option<(usize, StitchAction)> {
    let first = *ring.first()?;
    let last = *ring.last()?;
    segments.iter().enumerate().find_map(|(index, segment)| {
        let segment_first = *segment.first()?;
        let segment_last = *segment.last()?;
        if last == segment_first {
            Some((index, StitchAction::AppendForward))
        } else if last == segment_last {
            Some((index, StitchAction::AppendReverse))
        } else if first == segment_last {
            Some((index, StitchAction::PrependForward))
        } else if first == segment_first {
            Some((index, StitchAction::PrependReverse))
        } else {
            None
        }
    })
}

fn apply_segment(ring: &mut Vec<Coordinate>, mut segment: Vec<Coordinate>, action: StitchAction) {
    match action {
        StitchAction::AppendForward => ring.extend(segment.into_iter().skip(1)),
        StitchAction::AppendReverse => {
            segment.reverse();
            ring.extend(segment.into_iter().skip(1));
        }
        StitchAction::PrependForward => {
            segment.pop();
            segment.extend(ring.iter().copied());
            *ring = segment;
        }
        StitchAction::PrependReverse => {
            segment.reverse();
            segment.pop();
            segment.extend(ring.iter().copied());
            *ring = segment;
        }
    }
}
