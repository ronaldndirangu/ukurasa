//! Pictures on a page. Lists the raster images and figures a page's content draws, and moves,
//! resizes, or removes one by rewriting the operators around it, leaving the rest of the page as
//! it was.

use std::collections::{HashMap, HashSet};

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use ukurasa_geom::{PageGeometry, Rect, transform};

use crate::EditError;
use crate::lines::page_id;
use crate::lines::read::{
    IDENTITY, MAX_CONTENT, Matrix, check_images, decode_content, deref, matrix, mul, num,
};

/// A picture covering this much of the page or more is a background or a scan, and is left
/// out.
const BACKGROUND: f64 = 0.9;

/// How far an edit's original bounds may be from the picture's, in display points.
const TOLERANCE: f64 = 0.05;

/// The largest width or height a picture may be given, in display points.
const MAX_SIZE: f64 = 100_000.0;

/// How deep to look through nested forms for an image.
const FORM_DEPTH: usize = 4;

/// How deep page trees and nested forms may go before they are treated as unreadable.
const TREE_DEPTH: usize = 64;

/// The corners of the area an image XObject or inline image fills, in the space it is drawn in.
const UNIT: [(f64, f64); 4] = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];

/// A rectangle in display points: its top-left corner and size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    fn from_rect(rect: Rect) -> Self {
        Self {
            x: rect.x0,
            y: rect.y0,
            width: rect.width(),
            height: rect.height(),
        }
    }

    fn near(&self, other: &Bounds) -> bool {
        [
            (self.x, other.x),
            (self.y, other.y),
            (self.width, other.width),
            (self.height, other.height),
        ]
        .iter()
        .all(|(a, b)| (a - b).abs() <= TOLERANCE)
    }
}

/// A picture that can be moved, resized, or removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageImage {
    /// Index of the content operator that draws the picture.
    pub op: usize,
    /// Where the picture shows.
    pub bounds: Bounds,
    /// The area the picture stays visible in when it moves.
    pub limits: Bounds,
}

/// Moves, resizes, or removes a picture, identified by the content operator that draws it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEdit {
    pub page: usize,
    pub op: usize,
    /// The picture's bounds when it was read, to detect a page that changed since.
    pub original: Bounds,
    /// Where to show the picture, or `None` to remove it.
    pub bounds: Option<Bounds>,
}

/// A picture found in a page's content.
struct Found {
    op: usize,
    /// The operator an edit rewrites: the `q` of the outermost group that draws only this
    /// picture, so clipping set inside the group moves with it, or else `op` itself.
    anchor: usize,
    /// The transformation matrix in effect at the anchor.
    ctm: Matrix,
    bounds: Rect,
    limits: Rect,
    /// The XObject name the picture is drawn with, or `None` for an inline image.
    name: Option<Vec<u8>>,
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    /// Clipping regions in display coordinates, each the bounding box of a clipping path.
    clips: Vec<Rect>,
}

/// The pictures on page `page` (zero-based) that can be edited.
pub fn page_images(doc: &Document, page: usize) -> Result<Vec<PageImage>, EditError> {
    let page_id = page_id(doc, page)?;
    let ops = decode_content(doc, page_id)?;
    check_images(&ops)?;
    Ok(scan(doc, page_id, &ops)
        .into_iter()
        .map(|found| PageImage {
            op: found.op,
            bounds: Bounds::from_rect(found.bounds),
            limits: Bounds::from_rect(found.limits),
        })
        .collect())
}

/// An operator index and the operators that replace it.
pub(crate) type Replacement = (usize, Vec<Operation>);

/// The replacement for the anchor operator of each picture `edits` change, and the XObject
/// names of the pictures they remove.
pub(crate) fn edit_images(
    doc: &Document,
    page_id: ObjectId,
    ops: &[Operation],
    edits: &[&ImageEdit],
    index: usize,
) -> Result<(Vec<Replacement>, Vec<Vec<u8>>), EditError> {
    let stale = || EditError::StaleImage(index + 1);
    let found = scan(doc, page_id, ops);
    let geometry = crate::page_geometry(doc, page_id);
    let mut replaced = Vec::new();
    let mut removed = Vec::new();
    for edit in edits {
        let image = found.iter().find(|f| f.op == edit.op).ok_or_else(stale)?;
        let from = Bounds::from_rect(image.bounds);
        if !from.near(&edit.original) {
            return Err(stale());
        }
        let Some(to) = edit.bounds else {
            replaced.push((image.op, Vec::new()));
            removed.extend(image.name.clone());
            continue;
        };
        let size = |n: f64| n.is_finite() && n > 0.0 && n <= MAX_SIZE;
        if !(to.x.is_finite() && to.y.is_finite() && size(to.width) && size(to.height)) {
            return Err(EditError::Invalid(
                "picture bounds must be positive and finite",
            ));
        }
        let (kx, ky) = (to.width / from.width, to.height / from.height);
        let display = [kx, 0.0, 0.0, ky, to.x - from.x * kx, to.y - from.y * ky];
        let (to_display, to_user) = display_matrices(&geometry).ok_or_else(stale)?;
        let user = mul(&mul(&to_display, &display), &to_user);
        let inverse = invert(&image.ctm).ok_or_else(stale)?;
        let m = mul(&mul(&image.ctm, &user), &inverse);
        let cm = Operation::new("cm", m.iter().map(|&v| Object::Real(v as f32)).collect());
        let ops = if image.anchor == image.op {
            vec![
                Operation::new("q", vec![]),
                cm,
                ops[image.op].clone(),
                Operation::new("Q", vec![]),
            ]
        } else {
            vec![ops[image.anchor].clone(), cm]
        };
        replaced.push((image.anchor, ops));
    }
    Ok((replaced, removed))
}

/// The XObject names the `Do` operators in `ops` draw.
pub(crate) fn drawn_names(ops: &[Operation]) -> HashSet<Vec<u8>> {
    ops.iter()
        .filter(|op| op.operator == "Do")
        .filter_map(|op| op.operands.first()?.as_name().ok().map(<[u8]>::to_vec))
        .collect()
}

/// The XObjects named `names` in the resources of page `page_id` and its ancestors.
pub(crate) fn xobject_ids(doc: &Document, page_id: ObjectId, names: &[Vec<u8>]) -> Vec<ObjectId> {
    tree_resources(doc, page_id)
        .into_iter()
        .filter_map(|resources| xobjects(doc, resources))
        .flat_map(|xobjects| {
            names
                .iter()
                .filter_map(|name| xobjects.get(name).ok()?.as_reference().ok())
        })
        .collect()
}

/// Takes the pictures in `removed` that nothing draws anymore out of the page tree's
/// resources, which other pages may share or inherit, so a rewrite without unused objects drops
/// them. When some content can't be read, every picture stays.
pub(crate) fn forget(doc: &mut Document, removed: &HashSet<ObjectId>) {
    if removed.is_empty() {
        return;
    }
    let Some(drawn) = drawn_xobjects(doc) else {
        return;
    };
    let unused: HashSet<ObjectId> = removed.difference(&drawn).copied().collect();
    if unused.is_empty() {
        return;
    }
    let nodes: HashSet<ObjectId> = doc
        .get_pages()
        .into_values()
        .flat_map(|page_id| ancestry(doc, page_id))
        .collect();
    let places: Vec<(ObjectId, &[&[u8]])> = nodes
        .into_iter()
        .filter_map(|node| xobjects_place(doc, node))
        .collect();
    for (id, path) in places {
        let Some(xobjects) = dictionary_mut(doc, id, path) else {
            continue;
        };
        let names: Vec<Vec<u8>> = xobjects
            .iter()
            .filter(|(_, o)| matches!(o, Object::Reference(id) if unused.contains(id)))
            .map(|(name, _)| name.clone())
            .collect();
        for name in names {
            xobjects.remove(&name);
        }
    }
}

/// Page `page_id` and its ancestors in the page tree.
fn ancestry(doc: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let mut nodes = vec![page_id];
    while nodes.len() < TREE_DEPTH {
        let parent = doc
            .get_dictionary(nodes[nodes.len() - 1])
            .ok()
            .and_then(|node| node.get(b"Parent").ok()?.as_reference().ok());
        match parent {
            Some(parent) if !nodes.contains(&parent) => nodes.push(parent),
            _ => break,
        }
    }
    nodes
}

/// The resources of page `page_id` and of its ancestors, nearest first.
fn tree_resources(doc: &Document, page_id: ObjectId) -> Vec<&Dictionary> {
    ancestry(doc, page_id)
        .into_iter()
        .filter_map(|id| doc.get_dictionary(id).ok()?.get(b"Resources").ok())
        .filter_map(|o| deref(doc, o)?.as_dict().ok())
        .collect()
}

/// Where the XObjects of page tree node `node`'s own resources are: an object, and the keys
/// that lead from it to the dictionary.
fn xobjects_place(doc: &Document, node: ObjectId) -> Option<(ObjectId, &'static [&'static [u8]])> {
    const OWN: &[&[u8]] = &[];
    const IN_RESOURCES: &[&[u8]] = &[b"XObject"];
    const IN_NODE: &[&[u8]] = &[b"Resources", b"XObject"];
    let (owner, resources, path) = match doc.get_dictionary(node).ok()?.get(b"Resources").ok()? {
        &Object::Reference(id) => (id, doc.get_dictionary(id).ok()?, IN_RESOURCES),
        Object::Dictionary(resources) => (node, resources, IN_NODE),
        _ => return None,
    };
    Some(match resources.get(b"XObject") {
        Ok(&Object::Reference(id)) => (id, OWN),
        _ => (owner, path),
    })
}

fn dictionary_mut<'a>(
    doc: &'a mut Document,
    id: ObjectId,
    path: &[&[u8]],
) -> Option<&'a mut Dictionary> {
    let mut object = doc.get_object_mut(id).ok()?;
    for key in path {
        object = object.as_dict_mut().ok()?.get_mut(key).ok()?;
    }
    object.as_dict_mut().ok()
}

/// The XObjects that the pages, the forms they draw, and their annotations' appearances draw,
/// or `None` when some of that content can't be read.
fn drawn_xobjects(doc: &Document) -> Option<HashSet<ObjectId>> {
    let mut drawn = HashSet::new();
    for page_id in doc.get_pages().into_values() {
        let ops = decode_content(doc, page_id).ok()?;
        draw(doc, &ops, &tree_resources(doc, page_id), 0, &mut drawn)?;
        for appearance in appearances(doc, page_id) {
            draw_form(doc, appearance, &[], 0, &mut drawn)?;
        }
    }
    Some(drawn)
}

/// Adds the XObjects `ops` draw to `drawn`, looking their names up in `resources`.
fn draw(
    doc: &Document,
    ops: &[Operation],
    resources: &[&Dictionary],
    depth: usize,
    drawn: &mut HashSet<ObjectId>,
) -> Option<()> {
    for name in drawn_names(ops) {
        for xobjects in resources.iter().filter_map(|r| xobjects(doc, r)) {
            let Ok(&Object::Reference(id)) = xobjects.get(&name) else {
                continue;
            };
            let first = drawn.insert(id);
            let Ok(form) = doc.get_object(id).and_then(Object::as_stream) else {
                continue;
            };
            // A form without resources of its own uses those of whatever draws it.
            if subtype(doc, form) == Some(b"Form") && (first || !form.dict.has(b"Resources")) {
                draw_form(doc, form, resources, depth + 1, drawn)?;
            }
        }
    }
    Some(())
}

fn draw_form(
    doc: &Document,
    form: &Stream,
    inherited: &[&Dictionary],
    depth: usize,
    drawn: &mut HashSet<ObjectId>,
) -> Option<()> {
    if depth > TREE_DEPTH {
        return None;
    }
    let content = form.get_plain_content_with_limit(MAX_CONTENT).ok()?;
    let ops = Content::decode_strict(&content).ok()?.operations;
    let own = form
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|o| deref(doc, o)?.as_dict().ok());
    match own {
        Some(resources) => draw(doc, &ops, &[resources], depth, drawn),
        None => draw(doc, &ops, inherited, depth, drawn),
    }
}

/// The appearance streams of the page's annotations.
fn appearances<'a>(doc: &'a Document, page_id: ObjectId) -> Vec<&'a Stream> {
    let annots = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| deref(doc, page.get(b"Annots").ok()?)?.as_array().ok());
    let stream = |o: &'a Object| deref(doc, o)?.as_stream().ok();
    annots
        .into_iter()
        .flatten()
        .filter_map(|annot| deref(doc, annot)?.as_dict().ok()?.get(b"AP").ok())
        .filter_map(|ap| deref(doc, ap)?.as_dict().ok())
        .flat_map(|ap| ap.iter().filter_map(|(_, o)| deref(doc, o)))
        .flat_map(|o| match o {
            Object::Dictionary(states) => states.iter().filter_map(|(_, s)| stream(s)).collect(),
            o => stream(o).into_iter().collect::<Vec<_>>(),
        })
        .collect()
}

/// The page's user space to display matrix, and its inverse.
fn display_matrices(geometry: &PageGeometry) -> Option<(Matrix, Matrix)> {
    let (ox, oy) = geometry.to_display(0.0, 0.0);
    let (ax, ay) = geometry.to_display(1.0, 0.0);
    let (bx, by) = geometry.to_display(0.0, 1.0);
    let forward = [ax - ox, ay - oy, bx - ox, by - oy, ox, oy];
    Some((forward, invert(&forward)?))
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inverse = [
        m[3] / det,
        -m[1] / det,
        -m[2] / det,
        m[0] / det,
        (m[2] * m[5] - m[3] * m[4]) / det,
        (m[1] * m[4] - m[0] * m[5]) / det,
    ];
    inverse.iter().all(|v| v.is_finite()).then_some(inverse)
}

/// Whether the operator paints anything on the page.
fn paints(op: &Operation) -> bool {
    matches!(
        op.operator.as_str(),
        "S" | "s"
            | "f"
            | "F"
            | "f*"
            | "B"
            | "B*"
            | "b"
            | "b*"
            | "sh"
            | "Do"
            | "BI"
            | "Tj"
            | "TJ"
            | "'"
            | "\""
    )
}

/// The XObjects in `resources`.
fn xobjects<'a>(doc: &'a Document, resources: &'a Dictionary) -> Option<&'a Dictionary> {
    resources
        .get(b"XObject")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
}

fn subtype<'a>(doc: &'a Document, stream: &'a Stream) -> Option<&'a [u8]> {
    stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_name().ok())
}

/// Whether the form, or a form it draws, draws an image.
fn form_has_image(
    doc: &Document,
    form: &Stream,
    depth: usize,
    seen: &mut HashSet<ObjectId>,
) -> bool {
    let Some(xobjects) = form
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .and_then(|resources| xobjects(doc, resources))
    else {
        return false;
    };
    xobjects.iter().any(|(_, xobject)| {
        if let Object::Reference(id) = xobject
            && !seen.insert(*id)
        {
            return false;
        }
        let Some(stream) = deref(doc, xobject).and_then(|o| o.as_stream().ok()) else {
            return false;
        };
        match subtype(doc, stream) {
            Some(b"Image") => true,
            Some(b"Form") => depth > 0 && form_has_image(doc, stream, depth - 1, seen),
            _ => false,
        }
    })
}

/// The corners of the area a form fills, in the space it is drawn in: its bounding box, through
/// its matrix.
fn form_corners(doc: &Document, form: &Stream) -> Option<[(f64, f64); 4]> {
    let array = |key: &[u8]| {
        form.dict
            .get(key)
            .ok()
            .and_then(|o| deref(doc, o))
            .and_then(|o| o.as_array().ok())
    };
    let bbox: Vec<f64> = array(b"BBox")?.iter().filter_map(num).collect();
    let [x0, y0, x1, y1] = bbox[..] else {
        return None;
    };
    let m = match array(b"Matrix") {
        Some(operands) => matrix(operands)?,
        None => IDENTITY,
    };
    Some([(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| transform(&m, x, y)))
}

/// The page's XObjects that draw pictures, by name, with the corners of the area each fills in
/// the space it is drawn in. Images are pictures, and so are forms that draw one, such as an
/// included figure.
fn picture_xobjects(doc: &Document, page_id: ObjectId) -> HashMap<Vec<u8>, [(f64, f64); 4]> {
    let Ok((direct, inherited)) = doc.get_page_resources(page_id) else {
        return HashMap::new();
    };
    let resources = direct.into_iter().chain(
        inherited
            .into_iter()
            .filter_map(|id| doc.get_dictionary(id).ok()),
    );
    let mut seen = HashSet::new();
    let mut pictures = HashMap::new();
    for resources in resources {
        let Some(xobjects) = xobjects(doc, resources) else {
            continue;
        };
        for (name, xobject) in xobjects.iter() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(stream) = deref(doc, xobject).and_then(|o| o.as_stream().ok()) else {
                continue;
            };
            let corners = match subtype(doc, stream) {
                Some(b"Image") => Some(UNIT),
                Some(b"Form") if form_has_image(doc, stream, FORM_DEPTH, &mut HashSet::new()) => {
                    form_corners(doc, stream)
                }
                _ => None,
            };
            if let Some(corners) = corners {
                pictures.insert(name.clone(), corners);
            }
        }
    }
    pictures
}

/// The editable pictures the page's decoded content `ops` draws.
fn scan(doc: &Document, page_id: ObjectId, ops: &[Operation]) -> Vec<Found> {
    let pictures = picture_xobjects(doc, page_id);
    let geometry = crate::page_geometry(doc, page_id);
    let (width, height) = geometry.display_size();
    let page = Rect::new(0.0, 0.0, width, height);

    // Where each closed group ends, and how many painting operators come before each operator.
    let mut ends = HashMap::new();
    let mut open = Vec::new();
    let mut painted = Vec::with_capacity(ops.len() + 1);
    painted.push(0usize);
    for (index, op) in ops.iter().enumerate() {
        match op.operator.as_str() {
            "q" => open.push(index),
            "Q" => {
                if let Some(start) = open.pop() {
                    ends.insert(start, index);
                }
            }
            _ => {}
        }
        painted.push(painted[index] + usize::from(paints(op)));
    }

    let display = |ctm: &Matrix, x: f64, y: f64| {
        let (x, y) = transform(ctm, x, y);
        geometry.to_display(x, y)
    };
    let mut state = State {
        ctm: IDENTITY,
        clips: Vec::new(),
    };
    let mut stack: Vec<(usize, State)> = Vec::new();
    let mut path: Vec<(f64, f64)> = Vec::new();
    let mut clipping = false;
    let mut found = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        let operands = op.operands.as_slice();
        let n = |i: usize| operands.get(i).and_then(num);
        let (name, corners) = match op.operator.as_str() {
            "q" => {
                stack.push((index, state.clone()));
                continue;
            }
            "Q" => {
                if let Some((_, saved)) = stack.pop() {
                    state = saved;
                }
                continue;
            }
            "cm" => {
                if let Some(m) = matrix(operands) {
                    state.ctm = mul(&m, &state.ctm);
                }
                continue;
            }
            "m" | "l" => {
                if let (Some(x), Some(y)) = (n(0), n(1)) {
                    path.push(display(&state.ctm, x, y));
                }
                continue;
            }
            "c" | "v" | "y" => {
                let values: Vec<f64> = operands.iter().filter_map(num).collect();
                for [x, y] in values.as_chunks::<2>().0 {
                    path.push(display(&state.ctm, *x, *y));
                }
                continue;
            }
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (n(0), n(1), n(2), n(3)) {
                    for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                        path.push(display(&state.ctm, px, py));
                    }
                }
                continue;
            }
            "W" | "W*" => {
                clipping = true;
                continue;
            }
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                if clipping && !path.is_empty() {
                    state.clips.push(Rect::bounding(&path));
                }
                path.clear();
                clipping = false;
                continue;
            }
            "Do" => match operands
                .first()
                .and_then(|o| o.as_name().ok())
                .and_then(|name| Some((name, pictures.get(name)?)))
            {
                Some((name, corners)) => (Some(name.to_vec()), *corners),
                None => continue,
            },
            "BI" if matches!(operands, [Object::Stream(_)]) => (None, UNIT),
            _ => continue,
        };

        if invert(&state.ctm).is_none() {
            continue;
        }
        let corners = corners.map(|(u, v)| display(&state.ctm, u, v));
        let mut anchor = (index, &state);
        for (start, saved) in stack.iter().rev() {
            match ends.get(start) {
                Some(&end) if painted[end] - painted[*start] == 1 => anchor = (*start, saved),
                _ => break,
            }
        }
        let (anchor, saved) = anchor;
        let kept = saved.clips.len();
        let intersect =
            |rects: &[Rect], start: Rect| rects.iter().fold(start, |r, c| r.intersect(c));
        let bounds = intersect(&state.clips[kept..], Rect::bounding(&corners));
        let limits = intersect(&state.clips[..kept], page);
        let visible = bounds.intersect(&limits);
        let area = |r: &Rect| r.width() * r.height();
        if visible.width() < 1.0
            || visible.height() < 1.0
            || area(&visible) >= BACKGROUND * area(&page)
        {
            continue;
        }
        found.push(Found {
            op: index,
            anchor,
            ctm: saved.ctm,
            bounds,
            limits,
            name,
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverts_matrices() {
        let m = [2.0, 1.0, -1.0, 3.0, 10.0, -4.0];
        let product = mul(&m, &invert(&m).unwrap());
        for (value, expected) in product.iter().zip(IDENTITY) {
            assert!((value - expected).abs() < 1e-9, "{product:?}");
        }
        assert!(invert(&[1.0, 2.0, 2.0, 4.0, 0.0, 0.0]).is_none());
    }
}
