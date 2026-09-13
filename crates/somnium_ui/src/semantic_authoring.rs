//! Semantic operations on the retained graph and timeline models.
//! Native gestures and automation use the same validation and bounded undo history.
use crate::graph::{GraphSurface, NodeId, PinRef};
use crate::timeline::{GroupId, MarkerId, MediaId, TimelineSurface, TrackId};
use glam::Vec2;
use serde_json::{Value, json};

pub(crate) fn text<'a>(p: &'a Value, k: &str) -> Result<&'a str, String> {
    p[k].as_str().ok_or_else(|| format!("{k} must be text"))
}
fn number(p: &Value, k: &str) -> Result<f32, String> {
    let v = p[k]
        .as_f64()
        .ok_or_else(|| format!("{k} must be a finite number"))? as f32;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{k} must be finite"))
    }
}
fn id(p: &Value, k: &str) -> Result<u32, String> {
    p[k].as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| format!("{k} must be an unsigned 32-bit id"))
}
fn point(p: &Value, k: &str) -> Result<Vec2, String> {
    let a: [f32; 2] =
        serde_json::from_value(p[k].clone()).map_err(|_| format!("{k} needs [x,y]"))?;
    let v = Vec2::from(a);
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{k} must be finite"))
    }
}
/// Stable content stamp for detecting concurrent edits, not an authentication token.
pub fn view_token(document: &str) -> String {
    let hash = document
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
    format!("view:{hash:016x}")
}
fn check(p: &Value, document: &str) -> Result<(), String> {
    if text(p, "expected_view")? != view_token(document) {
        Err("document_conflict: refresh the editor query; a designer changed this document".into())
    } else {
        Ok(())
    }
}
fn parsed(s: String) -> Result<Value, String> {
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

/// Query or apply a named graph operation; no second graph is retained by the adapter.
pub fn graph(surface: &mut GraphSurface, p: &Value) -> Result<Value, String> {
    let before = crate::graph::serial::to_json(&surface.graph, &surface.catalogue)
        .map_err(|e| e.to_string())?;
    let op = p["operation"].as_str().unwrap_or("query");
    let mut created = Value::Null;
    if op != "query" {
        check(p, &format!("{before}\n{:?}", surface.selection.ids()))?;
        match op {
            "add_node" => {
                created = json!(
                    surface
                        .add(text(p, "archetype")?, point(p, "position")?)
                        .ok_or("unknown archetype")?
                        .0
                );
            }
            "literal" => {
                if !surface.set_literal(
                    NodeId(id(p, "node")?),
                    u16::try_from(id(p, "pin")?).map_err(|_| "pin out of range")?,
                    text(p, "value")?,
                ) {
                    return Err(
                        "literal rejected (connected, out of range, unknown, or unchanged)".into(),
                    );
                }
            }
            "select" => {
                let ids: Vec<u32> =
                    serde_json::from_value(p["nodes"].clone()).map_err(|_| "nodes must be ids")?;
                if ids
                    .iter()
                    .any(|id| surface.graph.node(NodeId(*id)).is_none())
                {
                    return Err("unknown node".into());
                }
                surface.selection.clear();
                for id in ids {
                    surface.selection.toggle(NodeId(id));
                }
            }
            "move" => {
                surface.move_selection(point(p, "delta")?);
            }
            "connect" | "disconnect" => {
                let from = PinRef::output(
                    NodeId(id(p, "from_node")?),
                    u16::try_from(id(p, "from_pin")?).map_err(|_| "pin out of range")?,
                );
                let to = PinRef::input(
                    NodeId(id(p, "to_node")?),
                    u16::try_from(id(p, "to_pin")?).map_err(|_| "pin out of range")?,
                );
                if op == "connect" {
                    surface.connect(from, to).map_err(|e| format!("{e:?}"))?;
                } else {
                    surface.disconnect(crate::graph::Connection { from, to });
                }
            }
            "comment" => {
                created = json!(
                    surface
                        .add_comment(point(p, "position")?, point(p, "size")?, text(p, "text")?)
                        .ok_or("comment rejected")?
                        .0
                );
            }
            "group" => {
                created = json!(
                    surface
                        .group_selection(point(p, "position")?, point(p, "size")?)
                        .ok_or("group rejected")?
                        .0
                );
            }
            "delete" => {
                surface.delete_selection();
            }
            "copy" => surface.copy(),
            "paste" => {
                created = json!(
                    surface
                        .paste(point(p, "offset")?)
                        .map_err(|e| format!("{e:?}"))?
                        .iter()
                        .map(|id| id.0)
                        .collect::<Vec<_>>()
                );
            }
            "align" => {
                use crate::graph::geometry::Alignment;
                let a = match text(p, "alignment")? {
                    "left" => Alignment::Left,
                    "right" => Alignment::Right,
                    "top" => Alignment::Top,
                    "bottom" => Alignment::Bottom,
                    "center_x" => Alignment::CentreX,
                    "center_y" => Alignment::CentreY,
                    _ => return Err("unknown alignment".into()),
                };
                surface.align_selection(a);
            }
            "replace" => {
                let replacement =
                    crate::graph::serial::from_json(&p["document"].to_string(), &surface.catalogue)
                        .map_err(|e| e.to_string())?;
                let old = surface.graph.clone();
                surface.graph = replacement;
                surface.commit_gesture(old, "Edit Graph Document");
            }
            "undo" => {
                surface.undo();
            }
            "redo" => {
                surface.redo();
            }
            _ => return Err("unknown graph operation".into()),
        }
    }
    let data = crate::graph::serial::to_json(&surface.graph, &surface.catalogue)
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"ok":true,"editor":"graph","view_token":view_token(&format!("{data}\n{:?}",surface.selection.ids())),"document":parsed(data)?,"created":created,"selection":surface.selection.ids().iter().map(|id|id.0).collect::<Vec<_>>(),"history":surface.history().labels(),"history_cursor":surface.history().position(),"catalogue":surface.palette("").iter().map(|a|json!({"id":a.id,"title":a.title,"inputs":a.inputs.iter().enumerate().map(|(i,p)|json!({"index":i,"name":p.name,"type":format!("{:?}",p.ty),"range":p.range})).collect::<Vec<_>>(),"outputs":a.outputs.iter().enumerate().map(|(i,p)|json!({"index":i,"name":p.name,"type":format!("{:?}",p.ty)})).collect::<Vec<_>>() })).collect::<Vec<_>>() }),
    )
}

/// Apply timeline edits to the same retained surface used by its native track and curve controls.
pub fn timeline(s: &mut TimelineSurface, p: &Value) -> Result<Value, String> {
    let before = crate::timeline::serial::to_json(s.document()).map_err(|e| e.to_string())?;
    let op = p["operation"].as_str().unwrap_or("query");
    let mut created = Value::Null;
    if op != "query" {
        check(p, &before)?;
        match op {
            "add_group" => {
                let parent = if p["parent"].is_null() {
                    None
                } else {
                    Some(GroupId(id(p, "parent")?))
                };
                created = json!(
                    s.add_group(text(p, "title")?, parent)
                        .map_err(|e| format!("{e:?}"))?
                        .0
                );
            }
            "add_track" => {
                let group = if p["group"].is_null() {
                    None
                } else {
                    Some(GroupId(id(p, "group")?))
                };
                created = json!(
                    s.add_track(text(p, "archetype")?, text(p, "title")?, group)
                        .map_err(|e| format!("{e:?}"))?
                        .0
                );
            }
            "add_media" => {
                created = json!(
                    s.add_media(
                        TrackId(id(p, "track")?),
                        text(p, "kind")?,
                        text(p, "source")?,
                        number(p, "start")?,
                        number(p, "duration")?
                    )
                    .map_err(|e| format!("{e:?}"))?
                    .0
                );
            }
            "move_media" => {
                s.move_media(MediaId(id(p, "media")?), number(p, "start")?);
            }
            "resize_media" => {
                s.resize_media(
                    MediaId(id(p, "media")?),
                    number(p, "start")?,
                    number(p, "duration")?,
                );
            }
            "add_marker" => {
                created = json!(
                    s.add_marker(number(p, "time")?, text(p, "label")?)
                        .map_err(|e| format!("{e:?}"))?
                        .0
                );
            }
            "move_marker" => {
                s.move_marker(MarkerId(id(p, "marker")?), number(p, "time")?);
            }
            "add_key" => {
                let key =
                    somnium_ecs::curve::CurveKey::new(number(p, "time")?, number(p, "value")?);
                created = json!(
                    s.add_keyframe(TrackId(id(p, "track")?), id(p, "channel")? as usize, key)
                        .map_err(|e| format!("{e:?}"))?
                );
            }
            "move_key" => {
                s.move_keyframe(
                    TrackId(id(p, "track")?),
                    id(p, "channel")? as usize,
                    id(p, "key")? as usize,
                    number(p, "time")?,
                );
            }
            "remove_track" => {
                s.remove_track(TrackId(id(p, "track")?));
            }
            "select_channel" => {
                if !s.select_channel(TrackId(id(p, "track")?), id(p, "channel")? as usize) {
                    return Err("unknown channel".into());
                }
            }
            "replace" => {
                let doc =
                    crate::timeline::serial::from_json(&p["document"].to_string(), &s.catalogue)
                        .map_err(|e| e.to_string())?;
                s.replace_authored(doc)?;
            }
            "scrub" => {
                s.scrub(number(p, "time")?);
            }
            "undo" => {
                s.undo();
            }
            "redo" => {
                s.redo();
            }
            _ => return Err("unknown timeline operation".into()),
        }
    }
    let data = crate::timeline::serial::to_json(s.document()).map_err(|e| e.to_string())?;
    Ok(
        json!({"ok":true,"editor":"timeline","view_token":view_token(&data),"document":parsed(data)?,"created":created,"playhead":s.playhead,"history":s.history_labels()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_native_history_and_conflict() {
        let mut s = GraphSurface::new(crate::graph::scatter::catalogue());
        let q = graph(&mut s, &json!({})).unwrap();
        let node = q["catalogue"][0]["id"].clone();
        let after=graph(&mut s,&json!({"operation":"add_node","expected_view":q["view_token"],"archetype":node,"position":[10,20]})).unwrap();
        assert!(
            graph(
                &mut s,
                &json!({"operation":"delete","expected_view":q["view_token"]})
            )
            .is_err()
        );
        let undone = graph(
            &mut s,
            &json!({"operation":"undo","expected_view":after["view_token"]}),
        )
        .unwrap();
        assert_eq!(q["document"], undone["document"]);
    }
    #[test]
    fn timeline_native_history_and_finite_validation() {
        let mut s = TimelineSurface::new(crate::timeline::catalogues::animation(), 10.0);
        let q = timeline(&mut s, &json!({})).unwrap();
        let after=timeline(&mut s,&json!({"operation":"add_marker","expected_view":q["view_token"],"time":2.0,"label":"ignite"})).unwrap();
        assert!(timeline(&mut s,&json!({"operation":"add_marker","expected_view":after["view_token"],"time":"nan","label":"bad"})).is_err());
        let undone = timeline(
            &mut s,
            &json!({"operation":"undo","expected_view":after["view_token"]}),
        )
        .unwrap();
        assert_eq!(q["document"], undone["document"]);
    }
}
