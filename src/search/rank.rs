//! 打分：hybrid 动态融合与关系邻域扩展（设计 §7、D11）。

use crate::error::KbResult;
use crate::ids::WikiId;
use rusqlite::{Connection, params};
use std::collections::HashMap;

/// kw∈[0.7,0.9]，随 kw 分线性上升；sem 补足。
pub(crate) fn fuse(
    kw: Vec<(WikiId, f64)>,
    sem: Vec<(WikiId, f64)>,
) -> Vec<(WikiId, f64)> {
    let mut merged: HashMap<WikiId, (f64, f64)> = HashMap::new();
    for (id, s) in kw {
        merged.insert(id, (s, 0.0));
    }
    for (id, s) in sem {
        merged.entry(id).or_insert((0.0, 0.0)).1 = s;
    }
    merged
        .into_iter()
        .map(|(id, (k, s))| {
            let w = 0.7 + 0.2 * k;
            (id, w * k + (1.0 - w) * s)
        })
        .collect()
}

/// 沿 entry_relations 双向 BFS（深度 ≤2），邻居按父分 × 0.8^跳数计分。
pub(crate) fn expand(
    conn: &Connection,
    hits: Vec<(WikiId, f64)>,
    depth: u8,
) -> KbResult<Vec<(WikiId, f64)>> {
    let mut best: HashMap<WikiId, f64> = HashMap::new();
    let mut frontier = Vec::new();
    for (id, score) in hits {
        let slot = best.entry(id.clone()).or_insert(0.0);
        if score > *slot {
            *slot = score;
        }
        frontier.push((id, score));
    }

    for _ in 0..depth {
        let mut next = Vec::new();
        for (id, score) in frontier {
            for n in neighbors(conn, &id)? {
                if n == id {
                    continue;
                }
                let s = score * 0.8;
                let slot = best.entry(n.clone()).or_insert(0.0);
                if s > *slot {
                    *slot = s;
                    next.push((n, s));
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    Ok(best.into_iter().collect())
}

fn neighbors(conn: &Connection, id: &WikiId) -> KbResult<Vec<WikiId>> {
    let mut stmt = conn.prepare(
        "SELECT to_id FROM entry_relations WHERE from_id = ?1
         UNION
         SELECT from_id FROM entry_relations WHERE to_id = ?1",
    )?;
    let rows = stmt.query_map(params![id.as_str()], |r| r.get::<_, String>(0))?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    out.into_iter().map(|s| WikiId::new(&s)).collect()
}

pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len());
    let (mut dot, mut na, mut nb) = (0.0_f64, 0.0_f64, 0.0_f64);
    for i in 0..n {
        dot += a[i] as f64 * b[i] as f64;
        na += (a[i] as f64).powi(2);
        nb += (b[i] as f64).powi(2);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}
